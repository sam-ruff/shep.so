use super::*;
use tokio::sync::oneshot;

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
        let _guard = writer_engine.account_access("fixture").await;
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
            .expect("interactive write did not interrupt stalled read-only provider")
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
async fn stalled_sync_yields_to_account_write_after_cache_commit_is_observed() {
    interrupted_cache_write(Finish::Writer).await;
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
