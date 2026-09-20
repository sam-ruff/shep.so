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
async fn flag_receipt_cannot_patch_a_replacement_with_the_same_local_id() {
    let store = Store::memory().expect("store");
    seed(&store, 1).await;
    let original = store.query(MailQuery::default()).await.expect("page").rows[0].clone();
    store
        .start_individual_mail_action("identity".into(), original.clone(), read())
        .await
        .expect("admission");
    let item = store
        .claim_bulk_item("identity".into())
        .await
        .expect("claim")
        .expect("item");
    store
        .acknowledge_bulk_flags(
            item.clone(),
            Receipt::Flags {
                before: Flags {
                    unread: Some(true),
                    starred: None,
                },
                after: Flags {
                    unread: Some(false),
                    starred: None,
                },
            },
        )
        .await
        .expect("acknowledgement");
    store
        .run(|c| {
            c.execute(
                "UPDATE messages SET data=json_set(data,'$.remote_id','replacement'),starred=1",
                [],
            )?;
            Ok(())
        })
        .await
        .expect("replace physical source");
    assert!(
        store
            .finish_bulk_item(item, Ok(Receipt::Unchanged))
            .await
            .is_err()
    );
    let current = store.mail_metadata(original.id).await.expect("replacement");
    assert!(current.unread && current.starred);
    assert_eq!(current.remote_id, "replacement");
    let lease = store.bulk_lease("identity".into()).await.expect("lease");
    assert!(
        store
            .pending_bulk_flag_repair(&lease)
            .await
            .expect("retained receipt")
            .is_some()
    );
    assert_eq!(
        store.resume_bulk(&lease).await.expect("resume").uncertain,
        0
    );
}

#[tokio::test]
async fn acknowledged_flags_keep_the_receipt_and_projection_when_cache_commit_fails() {
    let store = Store::memory().expect("store");
    seed(&store, 2).await;
    let original = store.query(MailQuery::default()).await.expect("page").rows[0].clone();
    let selection = freeze(&store, MailQuery::default()).await;
    store
        .start_bulk("repair".into(), selection, read())
        .await
        .expect("admission");
    let item = store
        .claim_bulk_item("repair".into())
        .await
        .expect("claim")
        .expect("item");
    store
        .acknowledge_bulk_flags(
            item.clone(),
            Receipt::Flags {
                before: Flags {
                    unread: Some(true),
                    starred: None,
                },
                after: Flags {
                    unread: Some(false),
                    starred: None,
                },
            },
        )
        .await
        .expect("acknowledgement");
    store.run(|c| {
        c.execute_batch("CREATE TRIGGER reject_flag_cache BEFORE UPDATE OF unread ON messages BEGIN SELECT RAISE(ABORT,'fixture cache failure'); END;")?;
        Ok(())
    }).await.expect("cache failure fixture");
    assert!(
        store
            .finish_bulk_item(item.clone(), Ok(Receipt::Unchanged))
            .await
            .is_err()
    );
    assert_eq!(
        store
            .bulk_items("repair".into(), None)
            .await
            .expect("items")[0]
            .status,
        "repair"
    );
    assert_eq!(
        store
            .query(MailQuery::default())
            .await
            .expect("optimistic page")
            .unread,
        0
    );
    assert!(
        store
            .mail_metadata(original.id)
            .await
            .expect("confirmed cache")
            .unread
    );
    assert!(
        store
            .claim_bulk_item("repair".into())
            .await
            .expect("block later dispatch")
            .is_none()
    );
    let lease = store.bulk_lease("repair".into()).await.expect("lease");
    assert!(
        store
            .pending_bulk_flag_repair(&lease)
            .await
            .expect("durable receipt")
            .is_some()
    );
    store
        .run(|c| {
            c.execute_batch("DROP TRIGGER reject_flag_cache;")?;
            Ok(())
        })
        .await
        .expect("restore cache");
    let job = store
        .finish_bulk_item(item, Ok(Receipt::Unchanged))
        .await
        .expect("cache-only retry");
    assert_eq!((job.completed, job.remaining, job.uncertain), (1, 1, 0));
    assert!(
        store
            .claim_bulk_item("repair".into())
            .await
            .expect("next message")
            .is_some()
    );
}

#[tokio::test]
async fn acknowledged_flags_repair_after_restart_without_dispatch_and_preserve_undo() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("mail.sqlite");
    let original;
    {
        let store = Store::open(&path).expect("store");
        seed(&store, 1).await;
        original = store.query(MailQuery::default()).await.expect("page").rows[0].clone();
        store
            .start_individual_mail_action("flags".into(), original.clone(), read())
            .await
            .expect("admission");
        let item = store
            .claim_bulk_item("flags".into())
            .await
            .expect("claim")
            .expect("item");
        store
            .acknowledge_bulk_flags(
                item,
                Receipt::Flags {
                    before: Flags {
                        unread: Some(true),
                        starred: None,
                    },
                    after: Flags {
                        unread: Some(false),
                        starred: None,
                    },
                },
            )
            .await
            .expect("acknowledged");
        assert!(
            store
                .mail_metadata(original.id.clone())
                .await
                .expect("baseline")
                .unread
        );
        store
            .request_bulk_undo("flags".into())
            .await
            .expect("undo during cache gap");
    }
    let store = Store::open(&path).expect("reopen");
    let lease = store.bulk_lease("flags".into()).await.expect("lease");
    let resumed = store.resume_bulk(&lease).await.expect("resume");
    assert_eq!(resumed.uncertain, 0);
    assert!(
        store
            .claim_bulk_item("flags".into())
            .await
            .expect("no repeat")
            .is_none()
    );
    let item = store
        .pending_bulk_flag_repair(&lease)
        .await
        .expect("repair")
        .expect("acknowledged step");
    let finished = store
        .finish_bulk_item(
            item,
            Err((
                "a stale failure must not erase acknowledgement".into(),
                true,
            )),
        )
        .await
        .expect("repair cache");
    assert_eq!(finished.uncertain, 0);
    assert!(
        !store
            .mail_metadata(original.id)
            .await
            .expect("updated baseline")
            .unread
    );
    let inverse = store
        .claim_bulk_item("flags".into())
        .await
        .expect("inverse")
        .expect("undo");
    assert!(inverse.undo);
    assert!(matches!(
        inverse.receipt,
        Some(Receipt::Flags {
            before: Flags {
                unread: Some(true),
                ..
            },
            ..
        })
    ));
    assert!(
        store
            .pending_bulk_flag_repair(&lease)
            .await
            .expect("retired receipt")
            .is_none()
    );
}

#[tokio::test]
async fn individual_admission_shares_group_ownership_and_keeps_the_confirmed_baseline() {
    let store = Store::memory().expect("store");
    seed(&store, 3).await;
    let original = store.query(MailQuery::default()).await.expect("page").rows[0].clone();
    let raw = store.raw_message(original.id.clone()).await.expect("raw");
    let selection = freeze(&store, MailQuery::default()).await;
    let mut projected = original.clone();
    projected.unread = false;
    let job = store
        .start_individual_mail_action("individual".into(), projected.clone(), read())
        .await
        .expect("admitted");
    assert_eq!((job.total, job.remaining), (1, 1));
    assert_eq!(
        store
            .query(MailQuery::default())
            .await
            .expect("projected")
            .unread,
        2
    );
    assert!(
        store
            .mail_metadata(original.id.clone())
            .await
            .expect("baseline")
            .unread
    );
    assert!(
        store.bulk_items(job.id.clone(), None).await.expect("items")[0]
            .original
            .as_ref()
            .expect("original")
            .unread
    );
    assert_eq!(
        store.raw_message(original.id.clone()).await.expect("raw"),
        raw
    );
    assert!(
        store
            .start_bulk("overlap".into(), selection, moved("Trash"))
            .await
            .is_err()
    );
    assert!(store.bulk_job("overlap".into()).await.is_err());
    let repeated = store
        .start_individual_mail_action("individual".into(), projected, read())
        .await
        .expect("lost reply");
    assert_eq!(repeated.revision, job.revision);
    assert_eq!(store.bulk_jobs(0).await.expect("jobs").len(), 1);
    assert!(
        store
            .start_individual_mail_action("individual".into(), original, moved("Trash"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn individual_admission_rejects_changed_physical_identity_without_creating_work() {
    let store = Store::memory().expect("store");
    seed(&store, 1).await;
    let mut original = store.query(MailQuery::default()).await.expect("page").rows[0].clone();
    original.remote_id = "another-uid".into();
    assert!(
        store
            .start_individual_mail_action("stale".into(), original, read())
            .await
            .is_err()
    );
    assert!(store.bulk_jobs(0).await.expect("jobs").is_empty());
    assert_eq!(
        store
            .query(MailQuery::default())
            .await
            .expect("page")
            .unread,
        1
    );
}

#[tokio::test]
async fn individual_admission_survives_restart_and_never_requeues_a_dispatched_action() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).expect("store");
    seed(&store, 2).await;
    let rows = store.query(MailQuery::default()).await.expect("page").rows;
    store
        .start_individual_mail_action("queued".into(), rows[0].clone(), read())
        .await
        .expect("queued");
    store
        .start_individual_mail_action("dispatched".into(), rows[1].clone(), read())
        .await
        .expect("queued");
    store
        .claim_bulk_item("dispatched".into())
        .await
        .expect("claimed")
        .expect("item");
    drop(store);
    let reopened = Store::open(&path).expect("reopen");
    assert_eq!(
        reopened
            .query(MailQuery::default())
            .await
            .expect("projection")
            .unread,
        0
    );
    for id in ["queued", "dispatched"] {
        let lease = reopened.bulk_lease(id.into()).await.expect("owned");
        let state = reopened.resume_bulk(&lease).await.expect("recovered");
        if id == "queued" {
            assert_eq!((state.remaining, state.uncertain), (1, 0));
            assert!(
                reopened
                    .claim_bulk_item(id.into())
                    .await
                    .expect("claim")
                    .is_some()
            );
        } else {
            assert_eq!((state.remaining, state.uncertain), (0, 1));
            assert!(
                reopened
                    .claim_bulk_item(id.into())
                    .await
                    .expect("claim")
                    .is_none()
            );
        }
    }
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
        sort: MailSort::Relevance,
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
    let items = reopened.bulk_items("move".into(), None).await.unwrap();
    assert_eq!(items[1].status, "cancelled");
    assert_eq!(
        items[1].error.as_deref(),
        Some(shep::bulk::ACCEPTED_STATE_NOTE)
    );
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
        recovery: None,
        account: "work".into(),
        folder: "Archive".into(),
        current: Some(current.clone()),
        fingerprint: None,
        connections: vec![],
        local_only: false,
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

#[tokio::test]
async fn resolved_undo_claims_cannot_steal_another_item_or_survive_a_finished_phase() {
    let store = Store::memory().unwrap();
    seed(&store, 1).await;
    let original = store.query(MailQuery::default()).await.unwrap().rows[0].clone();
    store
        .start_bulk(
            "moving".into(),
            freeze(&store, MailQuery::default()).await,
            moved("Archive"),
        )
        .await
        .unwrap();
    let forward = store
        .claim_bulk_item("moving".into())
        .await
        .unwrap()
        .unwrap();
    store
        .claim_bulk_identity(forward.clone(), original.id.clone())
        .await
        .unwrap();
    assert!(
        store
            .claim_bulk_identity(forward.clone(), "unrelated-forward-id".into())
            .await
            .is_err()
    );
    let receipt = MoveReceipt::server(
        &original,
        "work",
        "Archive",
        None,
        store
            .message_fingerprint(original.id.clone())
            .await
            .unwrap(),
    );
    store
        .finish_bulk_item(forward.clone(), Ok(Receipt::Move(Box::new(receipt))))
        .await
        .unwrap();
    assert!(
        store
            .claim_bulk_identity(forward, original.id.clone())
            .await
            .is_err()
    );
    store.request_bulk_undo("moving".into()).await.unwrap();
    let undo = store
        .claim_bulk_item("moving".into())
        .await
        .unwrap()
        .unwrap();
    // The acknowledged MOVE had no destination UID. Another group now owns
    // this cache identity; recovering the first group must not take that claim.
    store
        .start_bulk(
            "other".into(),
            freeze(&store, MailQuery::default()).await,
            read(),
        )
        .await
        .unwrap();
    assert!(
        store
            .claim_bulk_identity(undo.clone(), original.id.clone())
            .await
            .is_err()
    );
    assert_eq!(
        store.bulk_owner(original.id).await.unwrap(),
        Some("other".into())
    );
    store
        .claim_bulk_identity(undo.clone(), "work:Archive:resolved".into())
        .await
        .unwrap();
    assert_eq!(
        store
            .bulk_owner("work:Archive:resolved".into())
            .await
            .unwrap(),
        Some("moving".into())
    );
    store
        .finish_bulk_item(undo.clone(), Err(("Definite rejection".into(), false)))
        .await
        .unwrap();
    assert!(
        store
            .bulk_owner("work:Archive:resolved".into())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .claim_bulk_identity(undo, "work:Archive:resolved".into())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn memory_job_leases_exclude_competing_clones_without_disk_files() {
    let store = Store::memory().unwrap();
    let clone = store.clone();
    let owned = store.bulk_lease("owned".into()).await.unwrap();
    assert!(clone.bulk_lease("owned".into()).await.is_err());
    let different = clone.bulk_lease("different".into()).await.unwrap();
    drop(owned);
    assert!(clone.bulk_lease("owned".into()).await.is_ok());
    drop(different);
    let folder = store.folder_lease("owned".into()).await.unwrap();
    assert!(clone.folder_lease("owned".into()).await.is_err());
    drop(folder);
    assert!(clone.folder_lease("owned".into()).await.is_ok());
}
