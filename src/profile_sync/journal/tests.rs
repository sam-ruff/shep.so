use super::*;
use crate::profile_sync::drive::tests::{binding, reserved};

#[tokio::test]
async fn profile_journal_keeps_first_reservation_and_scopes_identity_across_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("uploads.sqlite");
    let first = Journal::open(Some(&path)).unwrap();
    let upload = reserved();
    first.prepare(upload.clone()).await.unwrap();
    let mut duplicate = upload.clone();
    duplicate.remote.id = "replacement".into();
    assert!(first.prepare(duplicate).await.is_err());
    let mut changed = serde_json::from_slice::<serde_json::Value>(upload.record.bytes()).unwrap();
    changed["changes"][4]["name"] = "Changed after reservation".into();
    let record =
        Record::decode(binding().namespace(), serde_json::to_vec(&changed).unwrap()).unwrap();
    let mut duplicate = upload.clone();
    duplicate.remote.sha256 = record.sha256.clone();
    duplicate.remote.size = record.bytes.len() as u64;
    duplicate.record = record;
    assert!(first.prepare(duplicate).await.is_err());
    drop(first);
    let reopened = Journal::open(Some(&path)).unwrap();
    let saved = reopened
        .load(&binding(), upload.remote.key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.upload().record.bytes(), upload.record.bytes());
    assert_eq!(saved.upload().remote, upload.remote);
    assert!(
        reopened
            .load(
                &Binding::new("drive:other".into(), binding().namespace().into()).unwrap(),
                upload.remote.key
            )
            .await
            .unwrap()
            .is_none()
    );
    let mut wrong_receipt = upload.remote.clone();
    wrong_receipt.id = "wrong".into();
    assert!(reopened.acknowledge(&saved, &wrong_receipt).await.is_err());
    reopened.acknowledge(&saved, &upload.remote).await.unwrap();
    reopened.acknowledge(&saved, &upload.remote).await.unwrap();
    assert!(
        reopened
            .load(&binding(), upload.remote.key)
            .await
            .unwrap()
            .unwrap()
            .acknowledged()
    );
}

#[tokio::test]
async fn profile_journal_rejects_foreign_newer_or_corrupt_files_without_replacing_records() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("uploads.sqlite");
    let c = Connection::open(&path).unwrap();
    c.execute_batch("CREATE TABLE original(value TEXT); INSERT INTO original VALUES('keep')")
        .unwrap();
    assert!(Journal::open(Some(&path)).is_err());
    assert_eq!(
        c.query_row("SELECT value FROM original", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "keep"
    );
    drop(c);
    let path = directory.path().join("valid.sqlite");
    let journal = Journal::open(Some(&path)).unwrap();
    let upload = reserved();
    journal.prepare(upload.clone()).await.unwrap();
    let c = Connection::open(&path).unwrap();
    c.execute_batch("PRAGMA user_version=99").unwrap();
    assert!(Journal::open(Some(&path)).is_err());
    assert_eq!(
        c.query_row("SELECT drive_id FROM uploads", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        upload.remote.id
    );
    c.execute(
        "UPDATE uploads SET bytes=zeroblob(?)",
        [MAX_RECORD_BYTES as i64 + 1],
    )
    .unwrap();
    assert!(journal.load(&binding(), upload.remote.key).await.is_err());
}

#[tokio::test]
async fn profile_journal_cancelled_observer_cannot_replace_an_accepted_reservation() {
    let journal = Journal::open(None).unwrap();
    let (entered, entry) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::sync_channel(1);
    let worker = journal.worker.clone();
    let blocker = tokio::spawn(async move {
        worker
            .run(move |_| {
                let _ = entered.send(());
                held.recv().unwrap();
                Ok(())
            })
            .await
    });
    entry.await.unwrap();
    let commands = journal.worker.clone();
    // Poll prepare once: the queue has room, so it admits the immutable write
    // and yields while the real owning thread is deliberately held.
    let mut preparing = Box::pin(journal.prepare(reserved()));
    assert!(futures::poll!(preparing.as_mut()).is_pending());
    drop(preparing);
    release.send(()).unwrap();
    blocker.await.unwrap().unwrap();
    commands.run(|_| Ok(())).await.unwrap();
    let saved = journal
        .load(&binding(), reserved().remote.key)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.upload.remote.id, "reserved-profile");
    assert!(!saved.acknowledged());
}
