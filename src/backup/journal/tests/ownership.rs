use super::*;
use futures::poll;
use tokio::sync::oneshot;

#[tokio::test]
async fn backup_journal_bounded_admission_retains_ordered_sessions_after_observer_cancellation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("uploads.sqlite");
    let journal = Journal::open(Some(&path)).unwrap();
    let target = BackupTarget::Local("fixture".into());
    let original = upload("reserved", b"immutable ciphertext");
    journal
        .prepare(&target, original.clone(), b"immutable ciphertext".to_vec())
        .await
        .unwrap();
    let (entered, started) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let mut first = Box::pin(journal.run(move |_| {
        entered.send(()).unwrap();
        held.blocking_recv().unwrap();
        Ok(())
    }));
    assert!(poll!(first.as_mut()).is_pending());
    started.await.unwrap();
    let mut accepted = Vec::new();
    for n in 0..32 {
        let cloned = journal.clone();
        let target = target.clone();
        let mut upload = original.clone();
        upload.session = Some(format!("session-{n}"));
        let mut job = Box::pin(async move { cloned.checkpoint(&target, &upload).await });
        assert!(poll!(job.as_mut()).is_pending());
        accepted.push(job);
    }
    let mut overflow = original.clone();
    overflow.session = Some("cancelled-before-admission".into());
    let mut rejected = Box::pin(journal.checkpoint(&target, &overflow));
    assert!(poll!(rejected.as_mut()).is_pending());
    drop(rejected);
    drop(accepted);
    release.send(()).unwrap();
    first.await.unwrap();
    let pending = journal.pending(&target).await.unwrap().unwrap();
    assert_eq!(pending.upload.id, original.id);
    assert_eq!(pending.upload.session.as_deref(), Some("session-31"));
    assert_eq!(pending.data, b"immutable ciphertext");
    assert!(!pending.committed);
    drop(journal);
    let reopened = Journal::open(Some(&path)).unwrap();
    assert_eq!(
        reopened
            .pending(&target)
            .await
            .unwrap()
            .unwrap()
            .upload
            .session
            .as_deref(),
        Some("session-31")
    );
}

#[tokio::test]
async fn backup_journal_last_handle_close_drains_reserved_copy_and_commit_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("uploads.sqlite");
    let journal = Journal::open(Some(&path)).unwrap();
    let target = BackupTarget::Local("fixture".into());
    let (entered, started) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let mut first = Box::pin(journal.run(move |_| {
        entered.send(()).unwrap();
        held.blocking_recv().unwrap();
        Ok(())
    }));
    assert!(poll!(first.as_mut()).is_pending());
    started.await.unwrap();
    drop(first);
    let mut prepared = Box::pin(journal.prepare(
        &target,
        upload("reserved", b"ciphertext"),
        b"ciphertext".to_vec(),
    ));
    assert!(poll!(prepared.as_mut()).is_pending());
    drop(prepared);
    let mut committed = Box::pin(journal.committed(&target, "reserved"));
    assert!(poll!(committed.as_mut()).is_pending());
    drop(committed);
    let (drained, observed) = oneshot::channel();
    let mut barrier = Box::pin(journal.run(move |c| {
        let durable: i64 = c.query_row("PRAGMA synchronous", [], |r| r.get(0))?;
        assert_eq!(durable, 2); // FULL must remain in force on this owner.
        drained.send(()).unwrap();
        Ok(())
    }));
    assert!(poll!(barrier.as_mut()).is_pending());
    drop(barrier);
    drop(journal);
    release.send(()).unwrap();
    observed.await.unwrap();
    let reopened = Journal::open(Some(&path)).unwrap();
    let pending = reopened.pending(&target).await.unwrap().unwrap();
    assert_eq!(pending.upload.id, "reserved");
    assert_eq!(pending.data, b"ciphertext");
    assert!(pending.committed);
}

#[tokio::test]
async fn backup_journal_failed_transaction_does_not_poison_owner_or_replace_reserved_archive() {
    let journal = Journal::open(None).unwrap();
    let target = BackupTarget::Local("fixture".into());
    journal
        .prepare(
            &target,
            upload("reserved", b"ciphertext"),
            b"ciphertext".to_vec(),
        )
        .await
        .unwrap();
    assert!(
        journal
            .run::<()>(|c| {
                let tx = c.transaction()?;
                tx.execute("UPDATE uploads SET session='uncommitted',archive=x'00'", [])?;
                anyhow::bail!("fixture transaction failure")
            })
            .await
            .is_err()
    );
    let pending = journal.pending(&target).await.unwrap().unwrap();
    assert_eq!(pending.upload.session, None);
    assert_eq!(pending.data, b"ciphertext");
    let mut correct = pending.upload;
    correct.session = Some("durable-session".into());
    journal.checkpoint(&target, &correct).await.unwrap();
    journal.committed(&target, "reserved").await.unwrap();
    assert!(journal.pending(&target).await.unwrap().unwrap().committed);
}
