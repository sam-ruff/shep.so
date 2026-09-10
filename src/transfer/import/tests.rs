use super::*;
use crate::model::{Appearance, parse_mail};

pub(super) async fn workspace(path: &Path) -> Store {
    let store = Store::open(path).unwrap();
    store.save_account(serde_json::from_value(serde_json::json!({
        "id":"work", "name":"Import fixture", "email":"mail@example.test", "protocol":"Imap",
        "host":"imap.example.test", "port":993, "username":"mail@example.test",
        "smtp_host":"smtp.example.test", "smtp_port":465
    })).unwrap()).await.unwrap();
    store
        .save_preferences(Preferences {
            appearance: Appearance::Dark,
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .save_draft(Draft {
            id: "unsent".into(),
            account_id: "work".into(),
            body: "Keep this complete draft.".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let mail = parse_mail(
        "work",
        "7.2",
        "INBOX",
        b"From: sender@example.test\r\nSubject: Imported mail\r\n\r\nFull original body.".to_vec(),
        true,
        false,
    )
    .unwrap();
    store.upsert(vec![mail]).await.unwrap();
    store.run(|c| {
        // A real attachment makes copying span several bounded page steps.
        c.execute("INSERT INTO draft_attachments VALUES('file','unsent','large.bin','application/octet-stream',2097152,zeroblob(2097152))", [])?;
        Ok(())
    }).await.unwrap();
    store
}

#[tokio::test]
async fn complete_snapshot_is_private_consistent_and_does_not_touch_either_workspace() {
    let original = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let path = original.path().join("source.sqlite");
    let source = workspace(&path).await;
    let destination = Store::open(local.path().join("shep.sqlite")).unwrap();
    let (entered, waiting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let mut gate = Some((entered, held));
    let mut import = stage_observed(destination.clone(), path, move |progress, _| {
        if progress.phase == Phase::Copying
            && let Some((entered, held)) = gate.take()
        {
            assert!(progress.copied_pages < progress.total_pages);
            entered.send(()).unwrap();
            held.blocking_recv().unwrap();
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap();
    // Both real stores remain available while the snapshot holds its read point.
    tokio::time::timeout(Duration::from_secs(5), async {
        source.put("after-snapshot", true).await.unwrap();
        destination.put("local-only", "untouched").await.unwrap();
        assert_eq!(source.query(Default::default()).await.unwrap().total, 1);
        assert!(
            stage(destination.clone(), original.path().join("source.sqlite"))
                .await
                .is_err()
        );
    })
    .await
    .unwrap();
    assert!(futures::FutureExt::now_or_never(import.finish()).is_none());
    release.send(()).unwrap();
    let prepared = import.finish().await.unwrap().unwrap();
    assert_eq!(prepared.review.messages, 1);
    assert_eq!(prepared.review.accounts[0].email, "mail@example.test");
    assert_eq!(prepared.review.drafts, 1);
    assert!(prepared.review.bytes > 2_097_152);
    let staged = prepared.path().to_owned();
    let c = Connection::open_with_flags(&staged, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        count(&c, "SELECT length(data) FROM draft_attachments").unwrap(),
        2_097_152
    );
    assert_eq!(
        count(&c, "SELECT count(*) FROM kv WHERE key='after-snapshot'").unwrap(),
        0
    );
    let prefs: Preferences = setting(&c, "preferences").unwrap();
    assert_eq!(prefs.appearance, Appearance::Dark);
    assert_eq!(
        destination.get::<String>("local-only").await.unwrap(),
        "untouched"
    );
    assert_eq!(
        destination.query(Default::default()).await.unwrap().total,
        0
    );
    drop(c);
    drop(prepared);
    assert!(!staged.exists());
}

#[tokio::test]
async fn cancellation_and_dropped_observers_remove_private_partial_files() {
    for drop_observer in [false, true] {
        let original = tempfile::tempdir().unwrap();
        let local = tempfile::tempdir().unwrap();
        let path = original.path().join("source.sqlite");
        let source = workspace(&path).await;
        let destination = Store::open(local.path().join("shep.sqlite")).unwrap();
        let (entered, waiting) = oneshot::channel();
        let mut entered = Some(entered);
        let mut import = stage_observed(
            destination.clone(),
            path.clone(),
            move |progress, mut cancel| {
                if progress.phase == Phase::Copying
                    && let Some(entered) = entered.take()
                {
                    entered.send(()).unwrap();
                    let _ = futures::executor::block_on(cancel.changed());
                }
            },
        )
        .await
        .unwrap();
        waiting.await.unwrap();
        let mut progress = import.progress.clone();
        if drop_observer {
            drop(import);
        } else {
            import.cancel();
            assert!(import.finish().await.unwrap().is_none());
        }
        while progress.changed().await.is_ok() {}
        assert!(!local.path().read_dir().unwrap().any(|p| {
            p.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".shep-import-")
        }));
        assert_eq!(source.query(Default::default()).await.unwrap().total, 1);
        assert_eq!(
            destination.query(Default::default()).await.unwrap().total,
            0
        );
        let mut retry = stage(destination, path).await.unwrap();
        assert!(retry.finish().await.unwrap().is_some());
    }
}

#[tokio::test]
async fn unsupported_or_executable_schema_and_invalid_settings_never_become_a_store() {
    for mutation in [
        "PRAGMA user_version=999",
        "CREATE TRIGGER foreign_trigger AFTER INSERT ON messages BEGIN DELETE FROM kv; END",
        "DROP VIEW visible_mail; CREATE VIEW visible_mail AS SELECT * FROM messages WHERE 0",
        "CREATE TABLE unknown_future_extension(data TEXT)",
        "DROP TABLE backup_history; PRAGMA user_version=3; CREATE TABLE unknown_future_extension(data TEXT)",
        "DROP TABLE backup_history; DROP TABLE imported_operations; PRAGMA user_version=2; CREATE TABLE unknown_future_extension(data TEXT)",
        "UPDATE kv SET value='not JSON' WHERE key='preferences'",
        "UPDATE kv SET value=json_set(value,'$[0].id','google-oauth') WHERE key='accounts'",
        "UPDATE kv SET value=json_set(value,'$[0].id','GOOGLE-OAUTH') WHERE key='accounts'",
        "UPDATE kv SET value=json_set(value,'$[0].id','backup-passphrase:target') WHERE key='accounts'",
        "UPDATE kv SET value=json_set(value,'$[0].id','work:smtp') WHERE key='accounts'",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("source.sqlite");
        let source = workspace(&path).await;
        source
            .run(move |c| {
                c.execute_batch(mutation)?;
                Ok(())
            })
            .await
            .unwrap();
        let destination = Store::open(directory.path().join("destination.sqlite")).unwrap();
        let mut import = stage(destination.clone(), path).await.unwrap();
        assert!(import.finish().await.is_err(), "accepted {mutation}");
        assert_eq!(
            destination.query(Default::default()).await.unwrap().total,
            0
        );
        assert!(
            source.get::<Preferences>("preferences").await.is_ok() || mutation.contains("not JSON")
        );
        assert!(!directory.path().read_dir().unwrap().any(|p| {
            p.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".shep-import-")
        }));
    }
}

#[tokio::test]
async fn corrupt_non_sqlite_and_broken_foreign_keys_are_rejected_without_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.sqlite");
    std::fs::write(&path, b"This is not a database.").unwrap();
    let destination = Store::open(directory.path().join("destination.sqlite")).unwrap();
    let mut import = stage(destination.clone(), path.clone()).await.unwrap();
    assert!(import.finish().await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"This is not a database.");
    std::fs::remove_file(&path).unwrap();
    let source = workspace(&path).await;
    source
        .run(|c| {
            c.execute_batch(
                "PRAGMA foreign_keys=OFF; INSERT INTO restored_messages VALUES('missing');",
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let mut import = stage(destination, path).await.unwrap();
    assert!(
        import
            .finish()
            .await
            .unwrap_err()
            .to_string()
            .contains("broken references")
    );
}

#[tokio::test]
async fn sqlite_statistics_are_accepted_and_no_file_is_created_for_memory_only_targets() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.sqlite");
    let source = workspace(&path).await;
    source
        .run(|c| {
            c.execute_batch("ANALYZE")?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(stage(Store::memory().unwrap(), path.clone()).await.is_err());
    let destination = Store::open(directory.path().join("destination.sqlite")).unwrap();
    let mut import = stage(destination, path).await.unwrap();
    assert!(import.finish().await.unwrap().is_some());
}
