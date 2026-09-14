//! A move the server refused completes on this device only. The cached row
//! keeps its server identity, shows at the destination, survives sync and
//! restart, and is retried on a bounded schedule.
use shep::{
    mail_actions::{journal::*, *},
    model::*,
    store::Store,
};
use std::collections::HashSet;

async fn refused(store: &Store) -> (StoredMail, MoveRecord) {
    let original = parse_mail(
        "work",
        "42.7",
        "INBOX",
        b"Message-ID: <refused@example.test>\r\nSubject: Refused\r\n\r\nStill on the server"
            .to_vec(),
        true,
        false,
    )
    .unwrap();
    store.upsert(vec![original.clone()]).await.unwrap();
    let mut receipt = MoveReceipt::server(
        &original.summary,
        "work",
        "Archive",
        None,
        Fingerprint::of(&original.raw),
    );
    receipt.connections = vec![("work".into(), "fixture".into())];
    let record = store
        .begin_local_mail_move(
            MoveRecord::new(original.summary.clone(), receipt),
            "NO [CANNOT] read-only mailbox".into(),
        )
        .await
        .unwrap();
    assert_eq!(record.stage, MoveStage::Local);
    (original, record)
}

fn folder(name: &str) -> MailQuery {
    MailQuery {
        folder: name.into(),
        ..Default::default()
    }
}

fn reconcile(folder: &str, live: &[&str]) -> MailSyncItem {
    MailSyncItem::Reconcile {
        account: "work".into(),
        folder: folder.into(),
        live_ids: live.iter().map(|id| id.to_string()).collect::<HashSet<_>>(),
    }
}

#[tokio::test]
async fn device_only_move_shows_at_destination_and_sync_neither_restores_nor_forgets_it() {
    let store = Store::memory().unwrap();
    let (original, _) = refused(&store).await;
    let id = original.summary.id.clone();
    let inbox = store.query(folder("INBOX")).await.unwrap();
    assert_eq!((inbox.total, inbox.rows.len()), (0, 0));
    let archive = store.query(folder("Archive")).await.unwrap();
    assert_eq!(archive.total, 1);
    assert_eq!(archive.rows[0].id, id);
    assert_eq!(archive.rows[0].folder, "Archive");
    assert!(
        archive.rows[0].remote_id.is_empty(),
        "no provider UID for a projected row"
    );
    assert!(archive.move_placeholders.contains(&id));
    assert_eq!(archive.move_recovery[&id].stage, MoveStage::Local);
    assert_eq!(archive.move_pending_total, 1);
    assert!(
        store.known("work".into()).await.unwrap().contains(&id),
        "sync must not download the retained server identity again"
    );

    // The source listing still contains the message: it must not reappear there.
    store.apply_sync(reconcile("INBOX", &[&id])).await.unwrap();
    assert_eq!(store.query(folder("INBOX")).await.unwrap().total, 0);
    assert_eq!(store.query(folder("Archive")).await.unwrap().total, 1);
    // The destination listing does not contain it: it must not be forgotten.
    store.apply_sync(reconcile("Archive", &[])).await.unwrap();
    assert_eq!(store.query(folder("Archive")).await.unwrap().total, 1);
    // Even a source listing without it keeps the protected original.
    store.apply_sync(reconcile("INBOX", &[])).await.unwrap();
    assert_eq!(store.raw_message(id.clone()).await.unwrap(), original.raw);

    // Server flag changes still reach the retained identity.
    store
        .apply_sync(MailSyncItem::Flags(vec![(id.clone(), false, true)]))
        .await
        .unwrap();
    let row = &store.query(folder("Archive")).await.unwrap().rows[0];
    assert!(!row.unread && row.starred);
    let detail = store.detail(id).await.unwrap();
    assert_eq!(detail.summary.folder, "Archive");
    assert!(detail.summary.remote_id.is_empty());
}

#[tokio::test]
async fn device_only_move_survives_restart_and_is_retried_on_a_bounded_schedule() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    let (original, record) = refused(&store).await;
    drop(store);
    let store = Store::open(&path).unwrap();
    let archive = store.query(folder("Archive")).await.unwrap();
    assert_eq!(archive.rows[0].id, original.summary.id);
    assert_eq!(
        archive.move_recovery[&original.summary.id].stage,
        MoveStage::Local
    );
    let now = 1_000_000;
    let due = store.mail_move_lookups(now).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].token, record.token);
    let attempted = store
        .begin_mail_move_lookup(due[0].clone(), now)
        .await
        .unwrap();
    assert_eq!(attempted.attempted, now);
    assert!(
        store.mail_move_lookups(now + 60).await.unwrap().is_empty(),
        "a refused move waits longer than an unconfirmed one"
    );
    assert_eq!(
        store
            .mail_move_lookups(now + shep::store::LOCAL_RETRY_SECONDS)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn only_unapplied_moves_within_one_account_can_complete_locally() {
    let store = Store::memory().unwrap();
    let original = parse_mail(
        "work",
        "42.8",
        "INBOX",
        b"Subject: Transfer\r\n\r\nBody".to_vec(),
        true,
        false,
    )
    .unwrap();
    store.upsert(vec![original.clone()]).await.unwrap();
    let mut receipt = MoveReceipt::server(
        &original.summary,
        "personal",
        "Keep",
        None,
        Fingerprint::of(&original.raw),
    );
    receipt.connections = vec![("work".into(), "a".into()), ("personal".into(), "b".into())];
    let transfer = MoveRecord::new(original.summary.clone(), receipt);
    assert!(
        store
            .begin_local_mail_move(transfer.clone(), "refused".into())
            .await
            .is_err()
    );
    assert!(
        store
            .mail_move_for_source(original.summary.id.clone())
            .await
            .unwrap()
            .is_none(),
        "a refused transfer records nothing"
    );
    let mut receipt = transfer.receipt.clone();
    receipt.account = "work".into();
    receipt.connections = vec![("work".into(), "a".into())];
    let record = MoveRecord::new(original.summary.clone(), receipt);
    store.prepare_mail_move(record.clone()).await.unwrap();
    let mut acknowledged = MoveReceipt::server(
        &original.summary,
        "work",
        "Keep",
        Some("91.1".into()),
        Fingerprint::of(&original.raw),
    );
    acknowledged.connections = vec![("work".into(), "a".into())];
    acknowledged.recovery = Some(record.token.clone());
    let committed = store
        .checkpoint_mail_move(record, MoveStage::Committed, acknowledged)
        .await
        .unwrap();
    assert!(
        store
            .local_mail_move(committed, "late NO".into())
            .await
            .is_err(),
        "an acknowledged move can never be reclassified as device-only"
    );
}
