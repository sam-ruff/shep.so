use shep::{
    bulk::{Action, Receipt},
    mail_actions::{Flags, MoveReceipt},
    model::*,
    store::{MailSelectionId, SelectionChange, Store},
};

async fn seed(store: &Store, count: usize) {
    let mail = (0..count)
        .map(|i| {
            parse_mail(
                "work",
                &format!("42.{i}"),
                "INBOX",
                format!(
                    "From: sender@example.test\r\nSubject: Mail {i:03}\r\n\r\nExact searchable body"
                )
                .into_bytes(),
                true,
                false,
            )
            .unwrap()
        })
        .collect();
    store.upsert(mail).await.unwrap();
}
async fn freeze(store: &Store, query: MailQuery) -> MailSelectionId {
    let source = MailSelectionId::default();
    store
        .capture_selection(source, 0, query, true, vec![])
        .await
        .unwrap();
    let snapshot = store.freeze_selection(source, 0).await.unwrap();
    store.release_selection(source).await.unwrap();
    snapshot.id
}
fn moved(folder: &str) -> Action {
    Action::Move {
        account: None,
        folder: folder.into(),
    }
}
fn read() -> Action {
    Action::Flags(Flags {
        unread: Some(false),
        starred: None,
    })
}

#[tokio::test]
async fn frozen_groups_publish_full_query_effects_without_changing_originals_or_mime() {
    let store = Store::memory().unwrap();
    seed(&store, 125).await;
    let snapshot = freeze(&store, MailQuery::default()).await;
    let first = store.query(MailQuery::default()).await.unwrap().rows[0].clone();
    let raw = store.raw_message(first.id.clone()).await.unwrap();
    let job = store
        .start_bulk("archive".into(), snapshot, moved("Archive"))
        .await
        .unwrap();
    assert_eq!((job.total, job.remaining), (125, 125));
    assert!(store.selection_snapshot(snapshot, vec![]).await.is_err());
    assert_eq!(
        store
            .query(MailQuery {
                folder: "INBOX".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        0
    );
    let destination = MailQuery {
        folder: "Archive".into(),
        search: "searchable".into(),
        ..Default::default()
    };
    let page = store.query(destination.clone()).await.unwrap();
    assert_eq!(page.total, 125);
    assert_eq!(page.rows.len(), 50);
    assert_eq!(page.bulk_pending.len(), 50);
    assert!(page.inbox_unread.is_empty());
    assert_eq!(page.bulk_revision, job.revision);
    assert!(page.rows.iter().all(|m| m.folder == "Archive"));
    assert_eq!(
        store.mail_metadata(first.id.clone()).await.unwrap().folder,
        "INBOX"
    );
    assert_eq!(store.raw_message(first.id).await.unwrap(), raw);
    assert_eq!(
        store
            .query(MailQuery {
                offset: 100,
                ..destination
            })
            .await
            .unwrap()
            .rows
            .len(),
        25
    );
    assert_eq!(
        store.bulk_items(job.id.clone(), None).await.unwrap().len(),
        50
    );
    assert_eq!(store.bulk_items(job.id, Some(99)).await.unwrap().len(), 25);
}

#[tokio::test]
async fn pending_flags_and_cross_account_scopes_preserve_other_fields() {
    let store = Store::memory().unwrap();
    seed(&store, 3).await;
    let snapshot = freeze(&store, MailQuery::default()).await;
    store
        .start_bulk("read".into(), snapshot, read())
        .await
        .unwrap();
    assert_eq!(
        store
            .query(MailQuery {
                unread_only: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        store
            .query(MailQuery {
                read_only: true,
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        3
    );
    let page = store.query(MailQuery::default()).await.unwrap();
    assert!(page.rows.iter().all(|m| !m.unread));
    assert!(page.inbox_unread.is_empty());
    for mail in page.rows {
        assert!(store.mail_metadata(mail.id).await.unwrap().unread);
    }
    store.request_bulk_undo("read".into()).await.unwrap();
    let snapshot = freeze(&store, MailQuery::default()).await;
    store
        .start_bulk(
            "transfer".into(),
            snapshot,
            Action::Move {
                account: Some("personal".into()),
                folder: "Plans".into(),
            },
        )
        .await
        .unwrap();
    let page = store
        .query(MailQuery {
            account: Some("personal".into()),
            folder: "Plans".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.total, 3);
    assert!(
        page.rows
            .iter()
            .all(|m| m.account_id == "personal" && m.unread)
    );
    assert_eq!(
        store
            .query(MailQuery {
                account: Some("work".into()),
                folder: String::new(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        0
    );
}

#[tokio::test]
async fn overlap_stale_results_missing_members_and_request_replays_are_explicit() {
    let store = Store::memory().unwrap();
    seed(&store, 3).await;
    let selection = MailSelectionId::default();
    store
        .capture_selection(selection, 0, MailQuery::default(), true, vec![])
        .await
        .unwrap();
    assert!(
        store
            .start_bulk("mutable".into(), selection, read())
            .await
            .is_err()
    );
    let frozen = store.freeze_selection(selection, 0).await.unwrap();
    store
        .change_selection(selection, 0, SelectionChange::Clear, vec![])
        .await
        .unwrap();
    let first = store.query(MailQuery::default()).await.unwrap().rows[0]
        .id
        .clone();
    store.remove(first).await.unwrap();
    let job = store
        .start_bulk("first".into(), frozen.id, read())
        .await
        .unwrap();
    assert_eq!((job.total, job.failed, job.remaining), (3, 1, 2));
    assert_eq!(
        store
            .start_bulk("first".into(), frozen.id, read())
            .await
            .unwrap()
            .total,
        3
    );
    let other = freeze(&store, MailQuery::default()).await;
    assert!(
        store
            .start_bulk("overlap".into(), other, moved("Trash"))
            .await
            .is_err()
    );
    assert_eq!(store.bulk_jobs(0).await.unwrap().len(), 1);
    assert!(
        store
            .start_bulk("first".into(), other, read())
            .await
            .is_err()
    );
    assert!(
        store
            .start_bulk("first".into(), frozen.id, moved("Trash"))
            .await
            .is_err()
    );
    let item = store
        .claim_bulk_item("first".into())
        .await
        .unwrap()
        .unwrap();
    store
        .finish_bulk_item(item.clone(), Err(("Rejected".into(), false)))
        .await
        .unwrap();
    assert!(
        store
            .finish_bulk_item(item, Ok(Receipt::Unchanged))
            .await
            .is_err()
    );
    let next = store
        .claim_bulk_item("first".into())
        .await
        .unwrap()
        .unwrap();
    store
        .finish_bulk_item(next.clone(), Err(("Acknowledgment lost".into(), true)))
        .await
        .unwrap();
    assert_eq!(
        store.bulk_owner(next.id).await.unwrap(),
        Some("first".into())
    );
    assert!(
        store
            .claim_bulk_item("first".into())
            .await
            .unwrap()
            .is_none()
    );
    let job = store.bulk_job("first".into()).await.unwrap();
    assert_eq!((job.failed, job.uncertain), (2, 1));
}

#[tokio::test]
async fn undo_while_running_cancels_unsent_items_and_waits_for_a_durable_receipt() {
    let store = Store::memory().unwrap();
    seed(&store, 3).await;
    let snapshot = freeze(&store, MailQuery::default()).await;
    store
        .start_bulk("read".into(), snapshot, read())
        .await
        .unwrap();
    let item = store.claim_bulk_item("read".into()).await.unwrap().unwrap();
    let job = store.request_bulk_undo("read".into()).await.unwrap();
    assert_eq!(job.cancelled, 2);
    let mail = item.original.clone().unwrap();
    let flags = Flags {
        unread: Some(false),
        starred: None,
    };
    store.patch_flags(mail.clone(), flags).await.unwrap();
    let receipt = Receipt::Flags {
        before: Flags {
            unread: Some(true),
            starred: None,
        },
        after: flags,
    };
    store.finish_bulk_item(item, Ok(receipt)).await.unwrap();
    let page = store.query(MailQuery::default()).await.unwrap();
    assert_eq!(page.unread, 3);
    let inverse = store.claim_bulk_item("read".into()).await.unwrap().unwrap();
    assert!(inverse.undo);
    let Receipt::Flags { before, .. } = inverse.receipt.clone().unwrap() else {
        panic!("Missing flags receipt")
    };
    store.patch_flags(mail, before).await.unwrap();
    let done = store
        .finish_bulk_item(inverse, Ok(Receipt::Unchanged))
        .await
        .unwrap();
    assert_eq!((done.restored, done.cancelled, done.remaining), (1, 2, 0));
    assert!(
        store
            .query(MailQuery::default())
            .await
            .unwrap()
            .bulk_pending
            .is_empty()
    );
    assert_eq!(
        store
            .request_bulk_undo("read".into())
            .await
            .unwrap()
            .restored,
        1
    );
}

#[tokio::test]
async fn restart_preserves_receipts_and_never_replays_an_unacknowledged_step() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    seed(&store, 3).await;
    let snapshot = freeze(&store, MailQuery::default()).await;
    store
        .start_bulk("move".into(), snapshot, moved("Archive"))
        .await
        .unwrap();
    let first = store.claim_bulk_item("move".into()).await.unwrap().unwrap();
    let original = first.original.clone().unwrap();
    let receipt = MoveReceipt::local(&original, "Archive");
    store
        .relocate_mail(original, receipt.current.clone().unwrap())
        .await
        .unwrap();
    store
        .finish_bulk_item(first, Ok(Receipt::Move(Box::new(receipt))))
        .await
        .unwrap();
    let interrupted = store.claim_bulk_item("move".into()).await.unwrap().unwrap();
    drop(store);
    let reopened = Store::open(&path).unwrap();
    let lease = reopened.bulk_lease("move".into()).await.unwrap();
    let status = reopened.resume_bulk(&lease).await.unwrap();
    assert_eq!(
        (status.completed, status.uncertain, status.remaining),
        (1, 1, 1)
    );
    let next = reopened
        .claim_bulk_item("move".into())
        .await
        .unwrap()
        .unwrap();
    assert_ne!(next.id, interrupted.id);
    assert_eq!(
        reopened.mail_metadata(interrupted.id).await.unwrap().folder,
        "INBOX"
    );
    reopened
        .finish_bulk_item(next, Err(("Missing folder".into(), false)))
        .await
        .unwrap();
    let accepted = reopened.accept_bulk_uncertainty(&lease).await.unwrap();
    assert_eq!(accepted.uncertain, 0);
    assert_eq!(
        reopened.bulk_items("move".into(), None).await.unwrap()[0]
            .receipt
            .as_ref()
            .map(|r| matches!(r, Receipt::Move(_))),
        Some(true)
    );
}

#[tokio::test]
async fn a_failed_inverse_stage_cannot_erase_the_acknowledged_forward_receipt() {
    let store = Store::memory().unwrap();
    seed(&store, 1).await;
    let snapshot = freeze(&store, MailQuery::default()).await;
    store
        .start_bulk("first".into(), snapshot, moved("Archive"))
        .await
        .unwrap();
    let item = store
        .claim_bulk_item("first".into())
        .await
        .unwrap()
        .unwrap();
    let original = item.original.clone().unwrap();
    let mut current = original.clone();
    current.id = "work:Archive:42.77".into();
    current.remote_id = "42.77".into();
    current.folder = "Archive".into();
    store
        .relocate_mail(original.clone(), current.clone())
        .await
        .unwrap();
    let other = freeze(
        &store,
        MailQuery {
            folder: "Archive".into(),
            ..Default::default()
        },
    )
    .await;
    store
        .start_bulk("other".into(), other, read())
        .await
        .unwrap();
    store.request_bulk_undo("first".into()).await.unwrap();
    let receipt = MoveReceipt {
        account: "work".into(),
        folder: "Archive".into(),
        current: Some(current.clone()),
        fingerprint: None,
        connections: vec![],
    };
    let result = store
        .finish_bulk_item(item, Ok(Receipt::Move(Box::new(receipt))))
        .await
        .unwrap();
    assert_eq!(result.failed, 1);
    let saved = store
        .bulk_items("first".into(), None)
        .await
        .unwrap()
        .remove(0);
    assert!(saved.undo);
    assert!(matches!(saved.receipt, Some(Receipt::Move(_))));
    assert_eq!(
        store.bulk_owner(current.id).await.unwrap(),
        Some("other".into())
    );
}

#[tokio::test]
async fn bulk_lease_child_process() {
    let Some(path) = std::env::var_os("SHEP_BULK_LOCK_FIXTURE") else {
        return;
    };
    let store = Store::open(path).unwrap();
    assert!(store.bulk_lease("owned".into()).await.is_err());
}
#[tokio::test]
async fn a_second_process_cannot_recover_a_live_job_and_released_lease_is_reusable() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    let lease = store.bulk_lease("owned".into()).await.unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "bulk_lease_child_process", "--nocapture"])
        .env("SHEP_BULK_LOCK_FIXTURE", &path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    drop(lease);
    let _again = store.bulk_lease("owned".into()).await.unwrap();
}

#[tokio::test]
async fn query_observations_distinguish_forward_and_undo_in_the_same_count_snapshot() {
    let store = Store::memory().unwrap();
    seed(&store, 3).await;
    let source = MailSelectionId::default();
    let selected = store
        .capture_selection(source, 0, MailQuery::default(), true, vec![])
        .await
        .unwrap();
    assert_eq!(
        (
            selected.groups.len(),
            selected.groups[0].total,
            selected.groups[0].unread
        ),
        (1, 3, 3)
    );
    let frozen = store.freeze_selection(source, 0).await.unwrap();
    let query = MailQuery {
        folder: "INBOX".into(),
        observe_bulk: vec!["archive".into()],
        ..Default::default()
    };
    assert!(
        store
            .query(query.clone())
            .await
            .unwrap()
            .bulk_observed
            .is_empty()
    );
    store
        .start_bulk("archive".into(), frozen.id, moved("Archive"))
        .await
        .unwrap();
    let page = store.query(query.clone()).await.unwrap();
    assert_eq!(page.total, 0);
    assert_eq!(page.bulk_observed.get("archive"), Some(&false));
    let running = store
        .claim_bulk_item("archive".into())
        .await
        .unwrap()
        .unwrap();
    store.request_bulk_undo("archive".into()).await.unwrap();
    let page = store.query(query).await.unwrap();
    assert_eq!(
        (page.total, page.unread),
        (3, 3),
        "Undo must reveal even the currently running source"
    );
    assert_eq!(page.bulk_observed.get("archive"), Some(&true));
    assert!(
        page.bulk_pending.contains(&running.id),
        "The restored source remains owned until the receipt"
    );
    assert!(
        store
            .query(MailQuery {
                observe_bulk: vec!["archive".into(); 33],
                ..Default::default()
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_definitely_failed_inverse_can_retry_without_repeating_the_forward_action() {
    let store = Store::memory().unwrap();
    seed(&store, 1).await;
    let snapshot = freeze(&store, MailQuery::default()).await;
    store
        .start_bulk("retry".into(), snapshot, moved("Archive"))
        .await
        .unwrap();
    let forward = store
        .claim_bulk_item("retry".into())
        .await
        .unwrap()
        .unwrap();
    let original = forward.original.clone().unwrap();
    let receipt = MoveReceipt::local(&original, "Archive");
    store
        .relocate_mail(original, receipt.current.clone().unwrap())
        .await
        .unwrap();
    store
        .finish_bulk_item(forward, Ok(Receipt::Move(Box::new(receipt))))
        .await
        .unwrap();
    store.request_bulk_undo("retry".into()).await.unwrap();
    let inverse = store
        .claim_bulk_item("retry".into())
        .await
        .unwrap()
        .unwrap();
    store
        .finish_bulk_item(inverse, Err(("Rejected before writing".into(), false)))
        .await
        .unwrap();
    let retry = store.request_bulk_undo("retry".into()).await.unwrap();
    assert_eq!((retry.remaining, retry.failed), (1, 0));
    let item = store
        .claim_bulk_item("retry".into())
        .await
        .unwrap()
        .unwrap();
    assert!(item.undo);
    assert!(matches!(item.receipt, Some(Receipt::Move(_))));
}
