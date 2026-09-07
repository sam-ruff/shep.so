use shep::{
    folder_actions::{Action, Outcome, Status},
    folders::{Mailbox, NameEncoding},
    model::*,
    store::{ConnectionKind, ConnectionRef, FolderLease, MailSelectionId, Store},
};

fn account() -> Account {
    serde_json::from_value(serde_json::json!({"id":"work","name":"Work","email":"work@example.test","protocol":"Imap","host":"localhost","port":993,"username":"work","smtp_host":"localhost","smtp_port":465,"sent_folder":"Projects/Sent"})).unwrap()
}
fn catalog() -> Vec<Mailbox> {
    [
        "INBOX",
        "Projects",
        "Projects/Design",
        "Projects/Design/&ZeVnLIqe-",
        "Projects/Sent",
        "Storage",
    ]
    .into_iter()
    .map(|name| Mailbox {
        delimiter: Some('/'),
        encoding: NameEncoding::ImapUtf7,
        ..Mailbox::flat(name.into())
    })
    .collect()
}
async fn seed(store: &Store) -> Vec<Mail> {
    store.save_account(account()).await.unwrap();
    store
        .save_folder_catalog("work".into(), catalog())
        .await
        .unwrap();
    let mut preferences = Preferences::default();
    preferences.expanded_folders.insert(
        "work".into(),
        [
            "Projects".into(),
            "Projects/Design".into(),
            "Storage".into(),
        ]
        .into(),
    );
    store.save_preferences(preferences).await.unwrap();
    let messages: Vec<_> = catalog().iter().enumerate().map(|(index, folder)| parse_mail("work", &format!("42.{}", index + 1), &folder.name,
        format!("Message-ID: <folder-{index}@example.test>\r\nReferences: <thread@example.test>\r\nSubject: Folder {index}\r\nFrom: sender@example.test\r\n\r\nSearchable original {index}").into_bytes(), true, index % 2 == 0).unwrap()).collect();
    let metadata = messages.iter().map(|mail| mail.summary.clone()).collect();
    store.upsert(messages).await.unwrap();
    store
        .apply_sync(MailSyncItem::SentFolder(
            "work".into(),
            Some("Projects/Sent".into()),
        ))
        .await
        .unwrap();
    metadata
}
async fn start(store: &Store, id: &str, action: Action) -> FolderLease {
    let review = store
        .folder_review("work".into(), "Projects".into(), action)
        .await
        .unwrap();
    store.start_folder_change(id.into(), review).await.unwrap();
    store.folder_lease(id.into()).await.unwrap()
}
fn move_to_storage() -> Action {
    Action::Move {
        parent: Some("Storage".into()),
    }
}

#[tokio::test]
async fn acknowledged_move_rekeys_entire_subtree_atomically_and_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    let metadata = seed(&store).await;
    let before = store.raw_message(metadata[3].id.clone()).await.unwrap();
    let restored = metadata[3].id.clone();
    store
        .run(move |c| {
            c.execute("INSERT INTO restored_messages VALUES(?)", [restored])?;
            Ok(())
        })
        .await
        .unwrap();
    let lease = start(&store, "move", move_to_storage()).await;
    assert!(store.ensure_folder_idle("work".into()).await.is_err());
    let step = store.claim_folder_step(&lease).await.unwrap().unwrap();
    assert!(
        store
            .commit_folder_step(&lease, step.position)
            .await
            .is_err()
    );
    store
        .record_folder_outcome(&lease, step.position, Outcome::Applied)
        .await
        .unwrap();
    let job = store
        .commit_folder_step(&lease, step.position)
        .await
        .unwrap();
    assert!(job.closed);
    // Reapplying the cache phase never duplicates messages or erases a receipt.
    assert!(
        store
            .commit_folder_step(&lease, step.position)
            .await
            .unwrap()
            .closed
    );
    drop(lease);
    drop(store);
    let store = Store::open(&path).unwrap();
    let workspace = store.workspace().await.unwrap();
    let tree = &workspace.folder_trees["work"];
    assert!(tree.node("Projects").is_none());
    assert_eq!(
        tree.node("Storage/Projects/Design/&ZeVnLIqe-")
            .unwrap()
            .label,
        "日本語"
    );
    assert_eq!(workspace.accounts[0].sent_folder, "Storage/Projects/Sent");
    assert!(workspace.preferences.expanded_folders["work"].contains("Storage/Projects/Design"));
    assert!(!workspace.preferences.expanded_folders["work"].contains("Projects"));
    for mail in metadata {
        let folder = mail.folder.replacen("Projects", "Storage/Projects", 1);
        let id = format!("work:{folder}:{}", mail.remote_id);
        let current = store.mail_metadata(id.clone()).await.unwrap();
        assert_eq!(current.folder, folder);
        assert_eq!(current.unread, mail.unread);
        assert_eq!(current.starred, mail.starred);
        if id != mail.id {
            assert!(store.mail_metadata(mail.id).await.is_err());
        }
    }
    let renamed = "work:Storage/Projects/Design/&ZeVnLIqe-:42.4".to_owned();
    assert_eq!(store.raw_message(renamed.clone()).await.unwrap(), before);
    assert_eq!(
        store
            .conversation(renamed.clone(), None)
            .await
            .unwrap()
            .total,
        6
    );
    assert_eq!(
        store
            .query(MailQuery {
                folder: "Storage/Projects/Design/&ZeVnLIqe-".into(),
                search: "Searchable".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        1
    );
    store
        .run(move |c| {
            assert!(c.query_row(
                "SELECT EXISTS(SELECT 1 FROM restored_messages WHERE id=?)",
                [renamed],
                |r| r.get::<_, bool>(0)
            )?);
            assert_eq!(
                c.query_row(
                    "SELECT folder FROM sent_folders WHERE account='work'",
                    [],
                    |r| r.get::<_, String>(0)
                )?,
                "Storage/Projects/Sent"
            );
            Ok(())
        })
        .await
        .unwrap();
    assert!(store.ensure_folder_idle("work".into()).await.is_ok());
}

#[tokio::test]
async fn partial_delete_commits_successes_and_retries_only_rejected_step() {
    let store = Store::memory().unwrap();
    let mail = seed(&store).await;
    let lease = start(&store, "delete", Action::Delete).await;
    let first = store.claim_folder_step(&lease).await.unwrap().unwrap();
    store
        .record_folder_outcome(&lease, first.position, Outcome::Applied)
        .await
        .unwrap();
    store
        .commit_folder_step(&lease, first.position)
        .await
        .unwrap();
    let second = store.claim_folder_step(&lease).await.unwrap().unwrap();
    store
        .record_folder_outcome(
            &lease,
            second.position,
            Outcome::Rejected("Read only folder".into()),
        )
        .await
        .unwrap();
    assert!(store.claim_folder_step(&lease).await.unwrap().is_none());
    assert!(store.raw_message(mail[3].id.clone()).await.is_err());
    assert!(store.raw_message(mail[2].id.clone()).await.is_ok());
    store.retry_folder_change(&lease).await.unwrap();
    let retry = store.claim_folder_step(&lease).await.unwrap().unwrap();
    assert_eq!(retry.position, second.position);
    store
        .record_folder_outcome(&lease, retry.position, Outcome::Applied)
        .await
        .unwrap();
    store
        .commit_folder_step(&lease, retry.position)
        .await
        .unwrap();
    while let Some(step) = store.claim_folder_step(&lease).await.unwrap() {
        store
            .record_folder_outcome(&lease, step.position, Outcome::Applied)
            .await
            .unwrap();
        store
            .commit_folder_step(&lease, step.position)
            .await
            .unwrap();
    }
    assert!(store.folder_job("delete".into()).await.unwrap().closed);
    let workspace = store.workspace().await.unwrap();
    assert!(workspace.folder_trees["work"].node("Projects").is_none());
    assert!(workspace.accounts[0].sent_folder.is_empty());
    assert!(store.raw_message(mail[0].id.clone()).await.is_ok());
    assert!(store.raw_message(mail[5].id.clone()).await.is_ok());
}

#[tokio::test]
async fn interrupted_write_is_not_replayed_but_durable_acknowledgment_can_finish_cache_commit() {
    for acknowledged in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mail.sqlite");
        let store = Store::open(&path).unwrap();
        seed(&store).await;
        let lease = start(&store, "restart", move_to_storage()).await;
        let step = store.claim_folder_step(&lease).await.unwrap().unwrap();
        if acknowledged {
            store
                .record_folder_outcome(&lease, step.position, Outcome::Applied)
                .await
                .unwrap();
        }
        drop(lease);
        drop(store);
        let store = Store::open(&path).unwrap();
        let lease = store.folder_lease("restart".into()).await.unwrap();
        let job = store.recover_folder_change(&lease).await.unwrap();
        assert!(store.claim_folder_step(&lease).await.unwrap().is_none());
        if acknowledged {
            assert_eq!(job.steps[0].status, Status::Acknowledged);
            assert!(store.commit_folder_step(&lease, 0).await.unwrap().closed);
        } else {
            assert_eq!(job.steps[0].status, Status::Uncertain);
            assert!(store.retry_folder_change(&lease).await.is_err());
            assert!(store.stop_folder_change(&lease, false).await.is_err());
            assert!(store.stop_folder_change(&lease, true).await.unwrap().closed);
            assert!(
                store.workspace().await.unwrap().folder_trees["work"]
                    .node("Projects")
                    .is_some()
            );
        }
    }
}

#[tokio::test]
async fn cache_collision_preserves_acknowledgment_and_all_original_bytes() {
    let store = Store::memory().unwrap();
    let mail = seed(&store).await;
    // A cache-only destination orphan is not a valid excuse to overwrite data.
    let collision = parse_mail(
        "work",
        &mail[1].remote_id,
        "Storage/Projects",
        b"Subject: Orphan\r\n\r\nKeep separately".to_vec(),
        false,
        false,
    )
    .unwrap();
    let other_id = collision.summary.id.clone();
    store.upsert(vec![collision.clone()]).await.unwrap();
    assert!(
        store
            .folder_review("work".into(), "Projects".into(), move_to_storage())
            .await
            .is_err(),
        "Known collisions are rejected before a provider command"
    );
    store.remove(other_id.clone()).await.unwrap();
    let lease = start(&store, "collision", move_to_storage()).await;
    let step = store.claim_folder_step(&lease).await.unwrap().unwrap();
    // Model an older independent writer that does not know the new journal.
    store.run(move |c| {
        let m = collision.summary;
        c.execute("INSERT INTO messages(id,account,folder,sender,subject,body,timestamp,unread,starred,data,raw) VALUES(?,?,?,?,?,?,?,?,?,?,?)", rusqlite::params![m.id,m.account_id,m.folder,m.sender,m.subject,collision.text,m.timestamp,m.unread,m.starred,serde_json::to_string(&m)?,collision.raw])?;
        Ok(())
    }).await.unwrap();
    store
        .record_folder_outcome(&lease, step.position, Outcome::Applied)
        .await
        .unwrap();
    assert!(
        store
            .commit_folder_step(&lease, step.position)
            .await
            .is_err()
    );
    assert_eq!(
        store.folder_job("collision".into()).await.unwrap().steps[0].status,
        Status::Acknowledged
    );
    assert!(store.raw_message(mail[1].id.clone()).await.is_ok());
    assert!(store.raw_message(other_id).await.is_ok());
    // Every folder update rolls back, including those processed before collision.
    assert!(store.raw_message(mail[3].id.clone()).await.is_ok());
    assert!(
        store.workspace().await.unwrap().folder_trees["work"]
            .node("Projects")
            .is_some()
    );
}

#[tokio::test]
async fn changed_review_and_overlapping_sync_or_group_work_cannot_bypass_folder_ownership() {
    let store = Store::memory().unwrap();
    let mail = seed(&store).await;
    let review = store
        .folder_review("work".into(), "Projects".into(), Action::Delete)
        .await
        .unwrap();
    let extra = parse_mail(
        "work",
        "42.99",
        "Projects",
        b"Subject: New arrival\r\n\r\nBody".to_vec(),
        true,
        false,
    )
    .unwrap();
    store.upsert(vec![extra]).await.unwrap();
    assert!(
        store
            .start_folder_change("stale".into(), review)
            .await
            .is_err()
    );
    let selection = MailSelectionId::default();
    store
        .capture_selection(
            selection,
            0,
            MailQuery {
                folder: "Projects".into(),
                ..Default::default()
            },
            true,
            vec![],
        )
        .await
        .unwrap();
    let frozen = store.freeze_selection(selection, 0).await.unwrap().id;
    let lease = start(&store, "current", move_to_storage()).await;
    assert!(
        store
            .start_bulk(
                "overlap".into(),
                frozen,
                shep::bulk::Action::Move {
                    account: None,
                    folder: "Storage".into()
                }
            )
            .await
            .is_err()
    );
    assert!(
        store
            .save_folder_catalog("work".into(), catalog())
            .await
            .is_err()
    );
    assert!(store.flags(mail[1].clone()).await.is_err());
    assert!(
        store
            .patch_flags(
                mail[1].clone(),
                shep::mail_actions::Flags {
                    unread: Some(false),
                    starred: None
                }
            )
            .await
            .is_err()
    );
    assert!(
        store
            .move_local(mail[1].id.clone(), "Storage".into())
            .await
            .is_err()
    );
    assert!(store.remove(mail[1].id.clone()).await.is_err());
    assert!(
        store
            .mail_metadata(mail[1].id.clone())
            .await
            .unwrap()
            .unread
    );
    assert!(
        store
            .apply_sync(MailSyncItem::Reconcile {
                account: "work".into(),
                folder: "Projects".into(),
                live_ids: Default::default()
            })
            .await
            .is_err()
    );
    assert!(store.raw_message(mail[1].id.clone()).await.is_ok());
    assert!(
        store
            .stop_folder_change(&lease, false)
            .await
            .unwrap()
            .closed
    );
    assert!(
        store
            .start_bulk(
                "overlap".into(),
                frozen,
                shep::bulk::Action::Move {
                    account: None,
                    folder: "Storage".into()
                }
            )
            .await
            .is_ok()
    );
    assert!(
        store
            .folder_review("work".into(), "Projects".into(), Action::Delete)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn completed_group_undo_receipts_follow_renamed_folders_and_delete_retires_only_affected_items()
 {
    use shep::{
        bulk::Receipt,
        mail_actions::{Flags, MoveReceipt},
    };
    for delete in [false, true] {
        let store = Store::memory().unwrap();
        let mail = seed(&store).await;
        let original = mail[1].clone();
        let current = mail[2].clone();
        let unaffected = mail[5].clone();
        let receipt = Receipt::Move(Box::new(MoveReceipt {
            recovery: None,
            account: "work".into(),
            folder: current.folder.clone(),
            current: Some(current),
            fingerprint: None,
            connections: vec![],
        }));
        store.run(move |c| {
            c.execute("INSERT INTO bulk_jobs(id,action,source,created) VALUES('history',?,'fixture',0)", [serde_json::to_string(&shep::bulk::Action::Move { account: None, folder: "Projects/Design".into() })?])?;
            c.execute("INSERT INTO bulk_items(job,position,id,original,status,receipt) VALUES('history',0,?,?,'done',?)", rusqlite::params![original.id,serde_json::to_string(&original)?,serde_json::to_string(&receipt)?])?;
            c.execute("INSERT INTO bulk_items(job,position,id,original,status,receipt) VALUES('history',1,?,?,'done',?)", rusqlite::params![unaffected.id,serde_json::to_string(&unaffected)?,serde_json::to_string(&Receipt::Flags { before: Flags {unread:Some(true),starred:None},after:Flags {unread:Some(false),starred:None} })?])?;
            Ok(())
        }).await.unwrap();
        assert_eq!(
            store
                .folder_review("work".into(), "Projects".into(), Action::Delete)
                .await
                .unwrap()
                .affected_history,
            1,
            "A receipt referencing two affected folders is one history item"
        );
        let lease = start(
            &store,
            "change",
            if delete {
                Action::Delete
            } else {
                move_to_storage()
            },
        )
        .await;
        while let Some(step) = store.claim_folder_step(&lease).await.unwrap() {
            store
                .record_folder_outcome(&lease, step.position, Outcome::Applied)
                .await
                .unwrap();
            store
                .commit_folder_step(&lease, step.position)
                .await
                .unwrap();
        }
        let items = store.bulk_items("history".into(), None).await.unwrap();
        assert_eq!(items[1].status, "done");
        assert_eq!(
            items[0].original.as_ref().unwrap().folder,
            if delete {
                "Projects"
            } else {
                "Storage/Projects"
            }
        );
        if delete {
            assert_eq!(items[0].status, "cancelled");
        } else {
            let Receipt::Move(receipt) = items[0].receipt.as_ref().unwrap() else {
                panic!()
            };
            assert_eq!(receipt.folder, "Storage/Projects/Design");
            assert_eq!(
                receipt.current.as_ref().unwrap().id,
                "work:Storage/Projects/Design:42.3"
            );
        }
        let undo = store.request_bulk_undo("history".into()).await.unwrap();
        assert_eq!(undo.remaining, if delete { 1 } else { 2 });
    }
}

#[tokio::test]
async fn folder_recovery_participates_in_reviewed_account_removal_and_cannot_resurrect_it() {
    let store = Store::memory().unwrap();
    seed(&store).await;
    let target = ConnectionRef {
        kind: ConnectionKind::Account,
        id: "work".into(),
    };
    let old = store.removal_preview(target.clone()).await.unwrap();
    let lease = start(&store, "remove", Action::Delete).await;
    assert!(store.remove_connection(old, true).await.is_err());
    let current = store.removal_preview(target).await.unwrap();
    assert!(current.transfers > 0);
    assert!(
        store
            .remove_connection(current.clone(), false)
            .await
            .is_err()
    );
    store.remove_connection(current, true).await.unwrap();
    assert!(store.claim_folder_step(&lease).await.is_err());
    assert!(store.folder_jobs(0).await.unwrap().is_empty());
    assert!(store.workspace().await.unwrap().accounts.is_empty());
}

#[tokio::test]
async fn pop3_local_folder_moves_keep_download_identity() {
    let store = Store::memory().unwrap();
    let mail = seed(&store).await;
    let mut config = account();
    config.protocol = Protocol::Pop3;
    store.save_account(config).await.unwrap();
    let known = store.known("work".into()).await.unwrap();
    let lease = start(&store, "local", move_to_storage()).await;
    let step = store.claim_folder_step(&lease).await.unwrap().unwrap();
    store
        .record_folder_outcome(&lease, step.position, Outcome::Applied)
        .await
        .unwrap();
    store
        .commit_folder_step(&lease, step.position)
        .await
        .unwrap();
    assert_eq!(store.known("work".into()).await.unwrap(), known);
    assert_eq!(
        store
            .mail_metadata(mail[2].id.clone())
            .await
            .unwrap()
            .folder,
        "Storage/Projects/Design"
    );
}

#[tokio::test]
async fn folder_executor_lease_excludes_another_connection_and_rejects_wrong_store() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    seed(&store).await;
    let lease = start(&store, "lease", Action::Delete).await;
    let other = Store::open(&path).unwrap();
    assert!(other.folder_lease("lease".into()).await.is_err());
    assert!(other.claim_folder_step(&lease).await.is_err());
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "folder_lease_process_probe", "--nocapture"])
        .env("SHEP_FOLDER_LEASE_PROBE", &path)
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
    assert!(
        String::from_utf8_lossy(&child.stdout).contains("folder lease excluded another process")
    );
    drop(lease);
    assert!(other.folder_lease("lease".into()).await.is_ok());
}

#[test]
fn folder_lease_process_probe() {
    let Some(path) = std::env::var_os("SHEP_FOLDER_LEASE_PROBE") else {
        return;
    };
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let store = Store::open(path).unwrap();
        assert!(store.folder_lease("lease".into()).await.is_err());
        println!("folder lease excluded another process");
    });
}

struct Server {
    catalog: Vec<Mailbox>,
    plan: shep::folder_actions::Plan,
    calls: Vec<shep::folder_actions::Step>,
    fail_call: Option<(usize, Outcome)>,
    fail_next_list: bool,
    fail_list_after_write: bool,
    stop_after_write: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}
#[async_trait::async_trait]
impl shep::folder_actions::Connection for Server {
    async fn catalog(&mut self) -> anyhow::Result<Vec<Mailbox>> {
        if std::mem::take(&mut self.fail_next_list) {
            anyhow::bail!("Fixture connection lost during LIST");
        }
        Ok(self.catalog.clone())
    }
    async fn apply(&mut self, step: &shep::folder_actions::Step) -> Outcome {
        self.calls.push(step.clone());
        if let Some((call, outcome)) = &self.fail_call
            && self.calls.len() == *call
        {
            return outcome.clone();
        }
        self.catalog = self.plan.project(&self.catalog, std::slice::from_ref(step));
        self.fail_next_list = std::mem::take(&mut self.fail_list_after_write);
        if let Some(stopping) = &self.stop_after_write {
            stopping.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Outcome::Applied
    }
}
fn server(job: &shep::folder_actions::Job) -> Server {
    Server {
        catalog: catalog(),
        plan: job.review.plan.clone(),
        calls: vec![],
        fail_call: None,
        fail_next_list: false,
        fail_list_after_write: false,
        stop_after_write: None,
    }
}

#[tokio::test]
async fn folder_runner_handles_partial_rejection_retry_and_server_refresh_failure_without_replay() {
    use shep::folder_actions::runner::run;
    use std::sync::atomic::AtomicBool;
    let store = Store::memory().unwrap();
    seed(&store).await;
    let lease = start(&store, "runner", Action::Delete).await;
    let mut server = server(&store.folder_job("runner".into()).await.unwrap());
    server.fail_call = Some((2, Outcome::Rejected("Denied".into())));
    let stopping = AtomicBool::new(false);
    let job = run(&store, &lease, Some(&mut server), &stopping, None)
        .await
        .unwrap();
    assert_eq!(job.steps[0].status, Status::Done);
    assert_eq!(job.steps[1].status, Status::Rejected);
    assert_eq!(server.calls.len(), 2);
    store.retry_folder_change(&lease).await.unwrap();
    assert!(
        run(&store, &lease, Some(&mut server), &stopping, None)
            .await
            .unwrap()
            .closed
    );
    assert_eq!(server.calls[1], server.calls[2]);
    assert_eq!(server.calls.len(), 5);

    let store = Store::memory().unwrap();
    seed(&store).await;
    let lease = start(&store, "rename", move_to_storage()).await;
    let mut server = self::server(&store.folder_job("rename".into()).await.unwrap());
    server.fail_list_after_write = true;
    assert!(
        run(&store, &lease, Some(&mut server), &stopping, None)
            .await
            .is_err()
    );
    assert_eq!(
        store.folder_job("rename".into()).await.unwrap().steps[0].status,
        Status::Acknowledged
    );
    assert!(
        run(&store, &lease, Some(&mut server), &stopping, None)
            .await
            .unwrap()
            .closed
    );
    assert_eq!(
        server.calls.len(),
        1,
        "A successful rename must not be repeated because later LIST failed"
    );
}

#[tokio::test]
async fn folder_runner_stops_between_durable_receipts_and_rechecks_new_descendants() {
    use shep::folder_actions::runner::run;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    let store = Store::memory().unwrap();
    seed(&store).await;
    let lease = start(&store, "stop", Action::Delete).await;
    let mut server = server(&store.folder_job("stop".into()).await.unwrap());
    let stopping = Arc::new(AtomicBool::new(false));
    server.stop_after_write = Some(stopping.clone());
    let (progress, receiver) = tokio::sync::watch::channel(std::sync::Arc::new(
        store.folder_job("stop".into()).await.unwrap(),
    ));
    let job = run(
        &store,
        &lease,
        Some(&mut server),
        &stopping,
        Some(&progress),
    )
    .await
    .unwrap();
    assert!(!job.closed);
    assert_eq!(server.calls.len(), 1);
    assert_eq!(receiver.borrow().steps[0].status, Status::Done);
    stopping.store(false, Ordering::SeqCst);
    server.catalog.push(Mailbox {
        delimiter: Some('/'),
        encoding: NameEncoding::ImapUtf7,
        ..Mailbox::flat("Projects/New child".into())
    });
    assert!(
        run(&store, &lease, Some(&mut server), &stopping, None)
            .await
            .is_err()
    );
    assert_eq!(
        server.calls.len(),
        1,
        "Changed destructive scope needs another review"
    );
}

#[tokio::test]
async fn runner_refuses_imap_without_provider_and_pop3_crash_can_resume_local_work() {
    use shep::folder_actions::runner::run;
    use std::sync::atomic::AtomicBool;
    let store = Store::memory().unwrap();
    seed(&store).await;
    let lease = start(&store, "imap", move_to_storage()).await;
    assert!(
        run(&store, &lease, None, &AtomicBool::new(false), None)
            .await
            .is_err()
    );
    store.stop_folder_change(&lease, false).await.unwrap();
    let mut config = account();
    config.protocol = Protocol::Pop3;
    store.save_account(config).await.unwrap();
    let lease = start(&store, "pop", move_to_storage()).await;
    store.claim_folder_step(&lease).await.unwrap().unwrap();
    assert!(
        run(&store, &lease, None, &AtomicBool::new(false), None)
            .await
            .unwrap()
            .closed
    );
}
