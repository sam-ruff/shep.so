use super::*;
use tokio::sync::oneshot;

#[tokio::test(start_paused = true)]
async fn continuing_large_download_progress_can_exceed_the_account_idle_budget()
-> anyhow::Result<()> {
    let engine = super::super::calendar_tests::engine();
    let started = tokio::time::Instant::now();
    engine
        .account_work
        .sync("fixture", |stop| async move {
            let result = download(
                stop,
                |tx| async move {
                    for _ in 0..3 {
                        tokio::time::sleep(Duration::from_secs(400)).await;
                        tx.send(MailSyncItem::DownloadProgress).await?;
                    }
                    Ok(vec!["INBOX".into()])
                },
                |mut rx| async move {
                    assert!(rx.recv().await.is_none());
                    Ok(())
                },
            )
            .await?;
            assert_eq!(result, Some(vec!["INBOX".into()]));
            Ok(())
        })
        .await?;
    assert!(started.elapsed() >= Duration::from_secs(1200));
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn closed_cache_receiver_does_not_spin_on_the_closed_progress_watch() -> anyhow::Result<()> {
    let engine = super::super::calendar_tests::engine();
    let result = engine
        .account_work
        .sync("fixture", |stop| async move {
            download(
                stop,
                |tx| async move {
                    tx.send(MailSyncItem::Flags(vec![])).await?;
                    std::future::pending::<anyhow::Result<Vec<String>>>().await
                },
                |_rx| async move { anyhow::bail!("fixture cache rejected incoming mail") },
            )
            .await?;
            Ok(())
        })
        .await;
    assert!(
        result
            .expect_err("cache failure must be observed")
            .to_string()
            .contains("cache rejected")
    );
    Ok(())
}

struct ProviderActive(Option<oneshot::Sender<()>>);
impl Drop for ProviderActive {
    fn drop(&mut self) {
        if let Some(done) = self.0.take() {
            let _ = done.send(());
        }
    }
}

#[derive(Clone, Copy)]
enum Finish {
    Writer,
    Owner,
    Error,
    Timeout,
}

async fn interrupted_cache_write(finish: Finish) {
    let engine = super::super::calendar_tests::engine();
    let accounts = engine.account_work.clone();
    let store = engine.store.clone();
    let (cache_entered, entered) = oneshot::channel();
    let (cache_release, released) = oneshot::channel();
    let (provider_release, failure) = oneshot::channel();
    let (provider_dropped, dropped) = oneshot::channel();
    let task = tokio::spawn(async move {
        accounts
            .sync("fixture", |stop| async move {
                let folders = download(
                    stop,
                    |tx| async move {
                        let _active = ProviderActive(Some(provider_dropped));
                        tx.send(MailSyncItem::Flags(vec![])).await?;
                        failure.await?;
                        anyhow::bail!("fixture provider rejected FETCH")
                    },
                    |mut rx| async move {
                        assert!(matches!(rx.recv().await, Some(MailSyncItem::Flags(_))));
                        // An actual SQLite spawn_blocking operation is held
                        // after it starts. Cancelling its future would detach it.
                        store
                            .run(move |db| {
                                let _ = cache_entered.send(());
                                released.blocking_recv()?;
                                db.execute(
                                    "INSERT OR REPLACE INTO kv VALUES('sync-order', '\"cached\"')",
                                    [],
                                )?;
                                Ok(())
                            })
                            .await?;
                        assert!(rx.recv().await.is_none());
                        Ok(())
                    },
                )
                .await?;
                assert!(
                    folders.is_none(),
                    "interruption is not a complete folder listing"
                );
                Ok(())
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), entered)
        .await
        .expect("cache write did not start")
        .unwrap();
    match finish {
        Finish::Owner => {
            task.abort();
        }
        Finish::Error => {
            provider_release.send(()).unwrap();
        }
        Finish::Timeout => {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(481)).await;
            tokio::time::resume();
        }
        Finish::Writer => {}
    }
    let mut dropped = Some(dropped);
    if !matches!(finish, Finish::Writer) {
        tokio::time::timeout(Duration::from_secs(10), dropped.take().unwrap())
            .await
            .expect("provider did not settle after its termination")
            .unwrap();
    }
    let writer_engine = engine.clone();
    let writer = tokio::spawn(async move {
        let _guard = writer_engine.account_exclusive("fixture").await;
        writer_engine
            .store
            .run(|db| {
                db.execute(
                    "INSERT OR REPLACE INTO kv VALUES('sync-order', '\"new read intent\"')",
                    [],
                )?;
                Ok(())
            })
            .await
    });
    if let Some(dropped) = dropped {
        tokio::time::timeout(Duration::from_secs(10), dropped)
            .await
            .expect("exclusive work did not interrupt stalled read-only provider")
            .unwrap();
    }
    assert!(
        !writer.is_finished(),
        "cache commit must still own the account"
    );
    // Another account is completely independent of this blocked cache write.
    let other = tokio::time::timeout(Duration::from_secs(10), engine.account_access("other"))
        .await
        .unwrap();
    drop(other);
    cache_release.send(()).unwrap();
    writer.await.unwrap().unwrap();
    assert_eq!(
        engine.store.get::<String>("sync-order").await.unwrap(),
        "new read intent"
    );
    let result = task.await;
    match finish {
        Finish::Writer => result.unwrap().unwrap(),
        Finish::Owner => assert!(result.unwrap_err().is_cancelled()),
        Finish::Error => assert!(
            result
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("rejected FETCH")
        ),
        Finish::Timeout => assert!(
            result
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("timed out")
        ),
    }
}

#[tokio::test]
async fn stalled_sync_yields_to_exclusive_account_work_after_cache_commit_is_observed() {
    interrupted_cache_write(Finish::Writer).await;
}

#[tokio::test]
async fn mail_action_completes_beside_a_held_readonly_sync() {
    let engine = super::super::calendar_tests::engine();
    let accounts = engine.account_work.clone();
    let (provider_dropped, mut dropped) = oneshot::channel();
    let (started, start) = oneshot::channel();
    let task = tokio::spawn(async move {
        accounts
            .sync("fixture", |stop| async move {
                download(
                    stop,
                    |tx| async move {
                        let _active = ProviderActive(Some(provider_dropped));
                        tx.send(MailSyncItem::Flags(vec![])).await?;
                        started.send(()).unwrap();
                        std::future::pending::<anyhow::Result<Vec<String>>>().await
                    },
                    |mut rx| async move {
                        while rx.recv().await.is_some() {}
                        Ok(())
                    },
                )
                .await?;
                Ok(())
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), start)
        .await
        .unwrap()
        .unwrap();
    let write = tokio::time::timeout(Duration::from_secs(10), engine.account_access("fixture"))
        .await
        .expect("a mail action must not wait for the download");
    drop(write);
    assert!(
        matches!(dropped.try_recv(), Err(oneshot::error::TryRecvError::Empty)),
        "the download must continue past the action"
    );
    task.abort();
    tokio::time::timeout(Duration::from_secs(10), dropped)
        .await
        .expect("shutdown still stops the provider")
        .unwrap();
}

#[tokio::test]
async fn a_listing_taken_before_a_local_flag_write_does_not_undo_it() -> anyhow::Result<()> {
    let engine = super::super::calendar_tests::engine();
    let store = engine.store.clone();
    let mail = parse_mail(
        "fixture",
        "1.7",
        "INBOX",
        b"Subject: Flags\r\n\r\nBody".to_vec(),
        true,
        false,
    )?;
    let summary = mail.summary.clone();
    store.upsert(vec![mail]).await?;
    let (listed, listing) = oneshot::channel();
    let (written, write) = oneshot::channel::<()>();
    let epoch = store.sync_epoch().await?;
    let accounts = engine.account_work.clone();
    let cache = store.clone();
    let id = summary.id.clone();
    let sync = tokio::spawn(async move {
        accounts
            .sync("fixture", move |stop| async move {
                download(
                    stop,
                    |tx| async move {
                        // The scripted FETCH FLAGS is taken before the local
                        // write and delivered after it.
                        let stale = vec![(id, true, false)];
                        listed.send(()).unwrap();
                        write.await?;
                        tx.send(MailSyncItem::Flags(stale)).await?;
                        Ok(vec!["INBOX".into()])
                    },
                    |mut rx| async move {
                        while let Some(item) = rx.recv().await {
                            cache.apply_sync_since(item, Some(epoch)).await?;
                        }
                        Ok(())
                    },
                )
                .await?;
                Ok(())
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), listing).await??;
    let access = tokio::time::timeout(Duration::from_secs(10), engine.account_access("fixture"))
        .await
        .expect("the flag write must not wait for the download");
    store
        .patch_flags(
            summary.clone(),
            crate::mail_actions::Flags {
                unread: None,
                starred: Some(true),
            },
        )
        .await?;
    drop(access);
    written.send(()).unwrap();
    sync.await??;
    store.sync_finished("fixture".into(), epoch).await?;
    assert!(
        store.mail_metadata(summary.id.clone()).await?.starred,
        "the stale listing must not undo the flag"
    );
    let later = store.sync_epoch().await?;
    store
        .apply_sync_since(
            MailSyncItem::Flags(vec![(summary.id.clone(), true, false)]),
            Some(later),
        )
        .await?;
    assert!(
        !store.mail_metadata(summary.id).await?.starred,
        "a check started after the acknowledgement observes the server"
    );
    Ok(())
}

#[tokio::test]
async fn dropped_refresh_owner_cancels_download_but_keeps_inflight_cache_write_owned() {
    interrupted_cache_write(Finish::Owner).await;
}

#[tokio::test]
async fn failed_fetch_keeps_inflight_cache_write_owned() {
    interrupted_cache_write(Finish::Error).await;
}

#[tokio::test]
async fn timed_out_fetch_keeps_inflight_cache_write_owned() {
    interrupted_cache_write(Finish::Timeout).await;
}

#[tokio::test]
async fn failed_cache_receiver_closes_bounded_provider_channel_without_deadlock() {
    let accounts = account_work::Accounts::default();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        accounts.sync("fixture", |stop| async move {
            download(
                stop,
                |tx| async move {
                    loop {
                        tx.send(MailSyncItem::Flags(vec![])).await?;
                    }
                },
                |mut rx| async move {
                    assert!(rx.recv().await.is_some());
                    anyhow::bail!("fixture disk failure")
                },
            )
            .await?;
            Ok(())
        }),
    )
    .await
    .unwrap();
    assert!(result.unwrap_err().to_string().contains("disk failure"));
    let _write = accounts.write("fixture").await;
}

#[tokio::test]
async fn complete_sync_drains_every_received_item_and_returns_the_folder_list() {
    let accounts = account_work::Accounts::default();
    accounts
        .sync("fixture", |stop| async move {
            let folders = download(
                stop,
                |tx| async move {
                    for _ in 0..32 {
                        tx.send(MailSyncItem::Flags(vec![])).await?;
                    }
                    Ok(vec!["INBOX".into()])
                },
                |mut rx| async move {
                    let mut count = 0;
                    while rx.recv().await.is_some() {
                        count += 1;
                    }
                    assert_eq!(count, 32);
                    Ok(())
                },
            )
            .await?;
            assert_eq!(folders, Some(vec!["INBOX".into()]));
            Ok(())
        })
        .await
        .unwrap();
}
