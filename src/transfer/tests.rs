use super::*;
use crate::model::{Appearance, Draft, Preferences, parse_mail};

async fn workspace(path: &Path) -> Store {
    let store = Store::open(path).unwrap();
    let account = serde_json::from_value(serde_json::json!({
        "id":"work", "name":"Test account", "email":"mail@example.test",
        "protocol":"Imap", "host":"imap.example.test", "port":993,
        "username":"mail@example.test", "smtp_host":"smtp.example.test", "smtp_port":465
    }))
    .unwrap();
    store.save_account(account).await.unwrap();
    store
        .save_preferences(Preferences {
            appearance: Appearance::Dark,
            ..Default::default()
        })
        .await
        .unwrap();
    let mail = parse_mail("work", "7.42", "INBOX", b"From: Sender <sender@example.test>\r\nSubject: Complete copy\r\nMessage-ID: <copy@example.test>\r\n\r\nExact original mail.\r\n".to_vec(), true, false).unwrap();
    store.upsert(vec![mail]).await.unwrap();
    store
        .save_draft(Draft {
            id: "draft".into(),
            account_id: "work".into(),
            subject: "Not sent".into(),
            body: "Keep this draft".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    store.run(|c| {
        c.execute_batch("CREATE TABLE future_extension(id TEXT PRIMARY KEY, data BLOB NOT NULL);
        INSERT INTO future_extension VALUES('large',zeroblob(2097152));
        INSERT INTO draft_attachments VALUES('file','draft','notes.txt','text/plain',5,x'68656c6c6f');")?;
        Ok(())
    }).await.unwrap();
    store
        .put(
            "transfer:pending-original",
            Some((
                "destination".to_owned(),
                "Archive".to_owned(),
                "8.90".to_owned(),
            )),
        )
        .await
        .unwrap();
    store
}

#[tokio::test]
async fn snapshot_keeps_complete_database_while_writes_and_reads_continue() {
    let directory = tempfile::tempdir().unwrap();
    let store = workspace(&directory.path().join("cache.sqlite")).await;
    let destination = directory.path().join("export.sqlite");
    let (started, waiting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let mut gate = Some((started, held));
    let mut export = start_export(
        store.clone(),
        destination.clone(),
        false,
        move |progress, _| {
            if progress.phase == Phase::Copying
                && let Some((started, held)) = gate.take()
            {
                assert!(progress.copied_pages < progress.total_pages);
                let _ = started.send(());
                let _ = held.blocking_recv();
            }
        },
    )
    .await
    .unwrap();
    waiting.await.unwrap();
    // The actor selects progress/cancel messages against finish(). Losing that
    // select branch must not consume the eventual completion acknowledgment.
    assert!(futures::FutureExt::now_or_never(export.finish()).is_none());
    // The copy is still held. These are actual mail-cache reads/writes through
    // its production worker, not timing a quickly completed background job.
    tokio::time::timeout(Duration::from_secs(5), async {
        assert_eq!(store.query(Default::default()).await.unwrap().total, 1);
        store
            .save_preferences(Preferences {
                appearance: Appearance::Light,
                ..Default::default()
            })
            .await
            .unwrap();
        store.put("new-after-snapshot", "newer").await.unwrap();
        assert!(
            export_database(store.clone(), directory.path().join("second.sqlite"), false)
                .await
                .is_err()
        );
    })
    .await
    .unwrap();
    release.send(()).unwrap();
    let result = export.finish().await.unwrap();
    assert!(matches!(result, Outcome::Saved {bytes, warning:None,..} if bytes>2*1024*1024));
    assert!(!directory.path().join("export.sqlite-wal").exists());
    assert!(!directory.path().join("export.sqlite-shm").exists());
    let exported = Connection::open(&destination).unwrap();
    let account: String = exported
        .query_row("SELECT value FROM kv WHERE key='accounts'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let accounts: Vec<crate::model::Account> = serde_json::from_str(&account).unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].email, "mail@example.test");
    assert_eq!(
        exported
            .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    let prefs: String = exported
        .query_row("SELECT value FROM kv WHERE key='preferences'", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Preferences>(&prefs)
            .unwrap()
            .appearance,
        Appearance::Dark
    );
    assert_eq!(
        exported
            .query_row(
                "SELECT count(*) FROM kv WHERE key='new-after-snapshot'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        exported
            .query_row("SELECT raw FROM messages", [], |r| r.get::<_, Vec<u8>>(0))
            .unwrap(),
        store.raw_message("work:INBOX:7.42".into()).await.unwrap()
    );
    assert_eq!(
        exported
            .query_row("SELECT data FROM draft_attachments", [], |r| r
                .get::<_, Vec<u8>>(0))
            .unwrap(),
        b"hello"
    );
    assert_eq!(
        exported
            .query_row("SELECT length(data) FROM future_extension", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2097152
    );
    assert_eq!(
        exported
            .query_row(
                "SELECT value FROM kv WHERE key='transfer:pending-original'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "[\"destination\",\"Archive\",\"8.90\"]"
    );
    assert!(
        exported
            .query_row("SELECT value FROM kv WHERE key='drafts'", [], |r| r
                .get::<_, String>(0))
            .unwrap()
            .contains("Keep this draft")
    );
}

#[tokio::test]
async fn cancellation_preserves_existing_destination_and_cleans_partial_files() {
    let directory = tempfile::tempdir().unwrap();
    let store = workspace(&directory.path().join("cache.sqlite")).await;
    let destination = directory.path().join("prior.sqlite");
    std::fs::write(&destination, b"previous complete copy").unwrap();
    let (started, waiting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let mut gate = Some((started, held));
    let mut export = start_export(store.clone(), destination.clone(), true, move |p, _| {
        if p.phase == Phase::Copying
            && let Some((started, held)) = gate.take()
        {
            let _ = started.send(());
            let _ = held.blocking_recv();
        }
    })
    .await
    .unwrap();
    waiting.await.unwrap();
    export.cancel();
    release.send(()).unwrap();
    assert_eq!(export.finish().await.unwrap(), Outcome::Cancelled);
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"previous complete copy"
    );
    assert!(!directory.path().read_dir().unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".shep-export-")
    }));
    // Cancellation releases both local and process ownership before its ack.
    let mut retry = export_database(store, destination.clone(), true)
        .await
        .unwrap();
    assert!(matches!(
        retry.finish().await.unwrap(),
        Outcome::Saved { .. }
    ));
    assert_eq!(
        &std::fs::read(destination).unwrap()[..16],
        b"SQLite format 3\0"
    );
}

#[tokio::test]
async fn dropping_export_cancels_work_and_no_replace_never_changes_a_file() {
    let directory = tempfile::tempdir().unwrap();
    let store = workspace(&directory.path().join("cache.sqlite")).await;
    let destination = directory.path().join("old.sqlite");
    std::fs::write(&destination, b"keep").unwrap();
    let mut denied = export_database(store.clone(), destination.clone(), false)
        .await
        .unwrap();
    assert!(denied.finish().await.is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"keep");
    let (started, waiting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let mut gate = Some((started, held));
    let export = start_export(store, destination.clone(), true, move |p, _| {
        if p.phase == Phase::Copying
            && let Some((started, held)) = gate.take()
        {
            let _ = started.send(());
            let _ = held.blocking_recv();
        }
    })
    .await
    .unwrap();
    let mut updates = export.progress.clone();
    waiting.await.unwrap();
    drop(export);
    release.send(()).unwrap();
    while updates.changed().await.is_ok() {}
    assert_eq!(std::fs::read(&destination).unwrap(), b"keep");
    assert!(!directory.path().read_dir().unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".shep-export-")
    }));
}

#[tokio::test]
async fn active_database_journals_and_aliases_are_protected() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("cache.sqlite");
    let store = workspace(&source).await;
    let alias = directory.path().join("alias.sqlite");
    std::fs::hard_link(&source, &alias).unwrap();
    let mut forbidden = vec![
        source.clone(),
        alias,
        directory.path().join("cache.sqlite-wal"),
        directory.path().join("backup-uploads.sqlite"),
    ];
    forbidden.push(directory.path().join("bulk-locks/locked.sqlite"));
    #[cfg(unix)]
    {
        let alias = directory.path().join("symlink.sqlite");
        std::os::unix::fs::symlink(&source, &alias).unwrap();
        forbidden.push(alias);
    }
    for destination in forbidden {
        let mut export = export_database(store.clone(), destination, true)
            .await
            .unwrap();
        assert!(export.finish().await.is_err());
    }
    assert_eq!(store.query(Default::default()).await.unwrap().total, 1);
    assert_eq!(
        store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Dark
    );
}

#[tokio::test]
async fn invalid_destination_and_in_memory_store_leave_no_export() {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("no.sqlite");
    assert!(
        export_database(Store::memory().unwrap(), destination.clone(), false)
            .await
            .is_err()
    );
    assert!(!destination.exists());
    let store = workspace(&directory.path().join("cache.sqlite")).await;
    let mut export = export_database(store, directory.path().join("missing/file.sqlite"), false)
        .await
        .unwrap();
    assert!(export.finish().await.is_err());
    assert!(!directory.path().join("missing").exists());
}
