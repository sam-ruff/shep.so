use shep::{
    mail_actions::{
        Fingerprint, MoveReceipt, connection_key,
        journal::{MoveRecord, MoveStage},
    },
    model::*,
    store::{ConnectionKind, ConnectionRef, Store},
};

fn account(id: &str) -> Account {
    serde_json::from_value(serde_json::json!({"id":id,"name":id,"email":format!("{id}@example.test"),"protocol":"Imap","host":"imap.example.test","port":993,"username":id,"smtp_host":"smtp.example.test","smtp_port":465})).unwrap()
}
fn message(uid: &str) -> StoredMail {
    parse_mail("work",uid,"INBOX",format!("Message-ID: <{uid}@example.test>\r\nFrom: friend@example.test\r\nSubject: Original keepsake\r\n\r\nUnique body {uid}").into_bytes(),true,false).unwrap()
}
async fn prepare(store: &Store, uid: &str, target: &str) -> (StoredMail, MoveRecord) {
    let original = message(uid);
    store.upsert(vec![original.clone()]).await.unwrap();
    let mut receipt = MoveReceipt::server(
        &original.summary,
        target,
        "Keep",
        None,
        Fingerprint::of(&original.raw),
    );
    receipt.connections = vec![("work".into(), connection_key(&account("work")))];
    if target != "work" {
        receipt
            .connections
            .push((target.into(), connection_key(&account(target))));
    }
    let record = MoveRecord::new(original.summary.clone(), receipt);
    store.prepare_mail_move(record.clone()).await.unwrap();
    (original, record)
}
fn resolved(original: &StoredMail, receipt: &MoveReceipt, uid: &str) -> StoredMail {
    parse_mail(
        &receipt.account,
        uid,
        &receipt.folder,
        original.raw.clone(),
        false,
        true,
    )
    .unwrap()
}

#[tokio::test]
async fn acknowledged_move_without_uid_preserves_original_through_sync_restart_and_exact_lookup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    let (original, record) = prepare(&store, "42.7", "work").await;
    let committed = store
        .checkpoint_mail_move(record.clone(), MoveStage::Committed, record.receipt.clone())
        .await
        .unwrap();
    store
        .apply_sync(MailSyncItem::Reconcile {
            account: "work".into(),
            folder: "INBOX".into(),
            live_ids: Default::default(),
        })
        .await
        .unwrap();
    assert!(
        store.remove(original.summary.id.clone()).await.is_err(),
        "Deletion cannot destroy a journal-owned original"
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    let pending = store.pending_mail_moves(None, None).await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].stage, MoveStage::Committed);
    assert!(pending[0].receipt.current.is_none());
    assert_eq!(
        store
            .raw_message(original.summary.id.clone())
            .await
            .unwrap(),
        original.raw
    );
    let mut candidate = resolved(&original, &committed.receipt, "91.12");
    let mut bad = candidate.clone();
    bad.raw.extend_from_slice(b"different");
    assert!(
        store
            .resolve_mail_move(committed.clone(), bad)
            .await
            .is_err()
    );
    candidate.summary.timestamp = original.summary.timestamp;
    let current = candidate.summary.clone();
    let located = store
        .resolve_mail_move(committed.clone(), candidate)
        .await
        .unwrap();
    assert_eq!(located.stage, MoveStage::Located);
    assert_eq!(located.receipt.current.unwrap().id, current.id);
    assert!(store.raw_message(original.summary.id).await.is_err());
    assert_eq!(
        store.raw_message(current.id.clone()).await.unwrap(),
        original.raw
    );
    let page = store
        .query(MailQuery {
            account: Some("work".into()),
            folder: "Keep".into(),
            search: "Unique".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.total, 1);
    assert!(!page.rows[0].unread && page.rows[0].starred);
    assert!(
        store
            .pending_mail_moves(None, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store.commit_mail_move_cache(committed).await.is_err(),
        "Stale completions cannot overwrite recovery"
    );
}

#[tokio::test]
async fn copied_append_uid_and_connections_survive_restart_and_cleanup_retry() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    let (original, record) = prepare(&store, "42.8", "personal").await;
    let mut receipt = record.receipt.clone();
    receipt.current = Some(resolved(&original, &receipt, "76.5").summary);
    let copied = store
        .checkpoint_mail_move(record.clone(), MoveStage::Copied, receipt)
        .await
        .unwrap();
    let failed = store
        .fail_mail_move(copied.clone(), "Source cleanup disconnected".into())
        .await
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let resumed = store
        .mail_move_for_source(original.summary.id.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resumed.receipt.current.as_ref().unwrap().remote_id, "76.5");
    assert_eq!(resumed.receipt.connections, record.receipt.connections);
    assert_eq!(resumed.stage, MoveStage::Copied);
    assert_eq!(resumed.error, failed.error);
    assert!(store.reject_mail_move(resumed.clone()).await.is_err());
    assert!(
        store
            .checkpoint_mail_move(copied, MoveStage::Committed, resumed.receipt.clone())
            .await
            .is_err(),
        "Stale failure/ack ordering must be rejected"
    );
    let mut lost = resumed.receipt.clone();
    lost.current = None;
    assert!(
        store
            .checkpoint_mail_move(resumed.clone(), MoveStage::Committed, lost)
            .await
            .is_err()
    );
    let committed = store
        .checkpoint_mail_move(resumed.clone(), MoveStage::Committed, resumed.receipt)
        .await
        .unwrap();
    let current = committed.receipt.current.clone().unwrap();
    store.commit_mail_move_cache(committed).await.unwrap();
    assert_eq!(
        store.detail(current.id).await.unwrap().summary.account_id,
        "personal"
    );
    assert!(
        store
            .mail_metadata(original.summary.id.clone())
            .await
            .is_err()
    );
    assert_eq!(
        store
            .detail(original.summary.id)
            .await
            .unwrap()
            .summary
            .remote_id,
        "76.5"
    );
}

#[tokio::test]
async fn conflicting_destination_rolls_back_identity_release_and_keeps_original() {
    let store = Store::memory().unwrap();
    let (original, record) = prepare(&store, "42.9", "personal").await;
    let candidate = resolved(&original, &record.receipt, "91.4");
    let mut receipt = record.receipt.clone();
    receipt.current = Some(candidate.summary.clone());
    let committed = store
        .checkpoint_mail_move(record, MoveStage::Committed, receipt)
        .await
        .unwrap();
    let conflict = parse_mail(
        "personal",
        "91.4",
        "Keep",
        b"Subject: Different\r\n\r\nDo not replace".to_vec(),
        false,
        false,
    )
    .unwrap();
    store.upsert(vec![conflict]).await.unwrap();
    assert!(
        store
            .commit_mail_move_cache(committed.clone())
            .await
            .is_err()
    );
    assert_eq!(
        store.mail_move(committed.token).await.unwrap().stage,
        MoveStage::Committed
    );
    assert_eq!(
        store
            .raw_message(original.summary.id.clone())
            .await
            .unwrap(),
        original.raw
    );
    assert!(store.remove(original.summary.id).await.is_err());
    assert!(
        store
            .detail(candidate.summary.id)
            .await
            .unwrap()
            .body
            .contains("Do not replace")
    );
}

#[tokio::test]
async fn only_definite_first_command_rejection_releases_original_and_allows_a_new_intent() {
    let store = Store::memory().unwrap();
    let (original, record) = prepare(&store, "42.10", "work").await;
    assert!(
        store
            .prepare_mail_move(MoveRecord::new(
                original.summary.clone(),
                record.receipt.clone()
            ))
            .await
            .is_err()
    );
    let failed = store
        .fail_mail_move(
            record.clone(),
            "Connection lost: outcome unconfirmed".into(),
        )
        .await
        .unwrap();
    assert!(
        store.reject_mail_move(record.clone()).await.is_err(),
        "An older rejection cannot erase a newer outcome"
    );
    // The caller must establish a definite tagged rejection; an error string
    // itself never triggers this transition or authorizes another command.
    store.reject_mail_move(failed).await.unwrap();
    let next = MoveRecord::new(original.summary, record.receipt);
    store.prepare_mail_move(next.clone()).await.unwrap();
    let mut changed = next.receipt.clone();
    changed.folder = "Elsewhere".into();
    assert!(
        store
            .checkpoint_mail_move(next.clone(), MoveStage::Committed, changed)
            .await
            .is_err()
    );
    assert!(store.commit_mail_move_cache(next).await.is_err());
}

#[tokio::test]
async fn account_removal_reviews_pending_receipts_and_preserves_another_accounts_original() {
    let store = Store::memory().unwrap();
    store
        .put("accounts", vec![account("work"), account("personal")])
        .await
        .unwrap();
    let (original, record) = prepare(&store, "42.11", "personal").await;
    let target = ConnectionRef {
        kind: ConnectionKind::Account,
        id: "personal".into(),
    };
    let preview = store.removal_preview(target.clone()).await.unwrap();
    assert_eq!(preview.transfers, 1);
    assert!(
        store
            .remove_connection(preview.clone(), false)
            .await
            .is_err()
    );
    store
        .checkpoint_mail_move(record.clone(), MoveStage::Committed, record.receipt)
        .await
        .unwrap();
    assert!(
        store.remove_connection(preview, true).await.is_err(),
        "A new receipt requires a fresh review"
    );
    let preview = store.removal_preview(target).await.unwrap();
    store.remove_connection(preview, true).await.unwrap();
    assert!(
        store
            .pending_mail_moves(None, None)
            .await
            .unwrap()
            .is_empty()
    );
    store
        .apply_sync(MailSyncItem::Reconcile {
            account: "work".into(),
            folder: "INBOX".into(),
            live_ids: Default::default(),
        })
        .await
        .unwrap();
    let page = store.query(MailQuery::default()).await.unwrap();
    assert_eq!(page.total, 1);
    assert!(page.rows[0].is_local_copy());
    assert_ne!(page.rows[0].remote_id, original.summary.remote_id);
    assert_eq!(
        store.raw_message(page.rows[0].id.clone()).await.unwrap(),
        original.raw
    );
    assert!(store.mail_metadata(original.summary.id).await.is_err());
}

#[tokio::test]
async fn committed_recovery_is_readable_searchable_and_unselectable_in_destination_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    let (original, record) = prepare(&store, "42.20", "personal").await;
    store
        .checkpoint_mail_move(record.clone(), MoveStage::Committed, record.receipt.clone())
        .await
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let query = MailQuery {
        account: Some("personal".into()),
        folder: "Keep".into(),
        search: "Unique".into(),
        sort: MailSort::Relevance,
        observe: vec![original.summary.id.clone()],
        ..Default::default()
    };
    let page = store.query(query.clone()).await.unwrap();
    assert_eq!((page.total, page.rows.len(), page.unread), (1, 1, 1));
    let row = &page.rows[0];
    assert_eq!(
        (row.account_id.as_str(), row.folder.as_str()),
        ("personal", "Keep")
    );
    assert!(row.remote_id.is_empty());
    assert!(page.is_placeholder(&row.id));
    assert_eq!(page.move_recovery[&row.id].token, record.token);
    assert_eq!(page.observed[&row.id].as_ref().unwrap().folder, "Keep");
    assert_eq!(page.inbox_unread.values().sum::<usize>(), 0);
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
    let detail = store.detail(row.id.clone()).await.unwrap();
    assert_eq!(detail.summary.account_id, "personal");
    assert_eq!(detail.summary.folder, "Keep");
    assert!(detail.summary.remote_id.is_empty());
    assert!(detail.body.contains("Unique body 42.20"));
    let selection = store
        .capture_selection(Default::default(), 1, query.clone(), true, vec![])
        .await
        .unwrap();
    assert_eq!(
        (selection.total, selection.available, selection.selected),
        (0, 0, 0)
    );
    let workspace = store.workspace().await.unwrap();
    assert!(
        workspace
            .account_folders
            .get("personal")
            .is_some_and(|folders| folders.contains(&"Keep".to_owned())),
        "A pending destination needs to remain navigable even before LIST refresh"
    );
    // A queued query's older source projection cannot make an extra copy or
    // conceal the newly committed destination.
    let page = store
        .query(MailQuery {
            project_moves: vec![MailMoveProjection {
                id: row.id.clone(),
                source_account: "work".into(),
                source_folder: "INBOX".into(),
                account: "personal".into(),
                folder: "Keep".into(),
                unread: true,
                starred: false,
            }],
            ..query
        })
        .await
        .unwrap();
    assert_eq!((page.total, page.rows.len()), (1, 1));
}

#[tokio::test]
async fn recovery_queries_keep_metadata_pages_bounded_and_never_overwrite_protected_content_or_identity()
 {
    let store = Store::memory().unwrap();
    for i in 1..=55 {
        let (original, record) = prepare(&store, &format!("42.{i}"), "work").await;
        store
            .checkpoint_mail_move(record.clone(), MoveStage::Committed, record.receipt)
            .await
            .unwrap();
        let mut changed = original.clone();
        changed.raw.extend_from_slice(b"changed");
        store.upsert(vec![changed]).await.unwrap();
        assert_eq!(
            store
                .raw_message(original.summary.id.clone())
                .await
                .unwrap(),
            original.raw
        );
        assert!(
            store
                .move_local(original.summary.id.clone(), "Different".into())
                .await
                .is_err()
        );
        let mut changed = original.clone();
        changed.summary.remote_id = "99.99".into();
        store.upsert(vec![changed]).await.unwrap();
        assert_eq!(
            store
                .mail_metadata(original.summary.id.clone())
                .await
                .unwrap()
                .remote_id,
            original.summary.remote_id
        );
        let id = original.summary.id.clone();
        assert!(
            store
                .run(move |c| {
                    c.execute(
                        "UPDATE messages SET raw=? WHERE id=?",
                        rusqlite::params![b"replaced".as_slice(), id],
                    )?;
                    Ok(())
                })
                .await
                .is_err()
        );
    }
    let query = MailQuery {
        folder: "Keep".into(),
        ..Default::default()
    };
    let first = store.query(query.clone()).await.unwrap();
    assert_eq!(
        (first.total, first.rows.len(), first.move_recovery.len()),
        (55, 50, 50)
    );
    let second = store
        .query(MailQuery {
            offset: 50,
            ..query
        })
        .await
        .unwrap();
    assert_eq!(
        (second.total, second.rows.len(), second.move_recovery.len()),
        (55, 5, 5)
    );
    assert!(
        !first
            .rows
            .iter()
            .any(|a| second.rows.iter().any(|b| a.id == b.id))
    );
    let pending = store.pending_mail_moves(None, None).await.unwrap();
    assert_eq!(pending.len(), 50);
    assert_eq!(
        store
            .pending_mail_moves(None, Some(pending.last().unwrap().token.clone()))
            .await
            .unwrap()
            .len(),
        5
    );
}

#[tokio::test]
async fn selection_captured_before_move_cannot_issue_another_provider_change_on_its_cache_identity()
{
    use shep::{bulk::Action, mail_actions::Flags, store::MailSelectionId};
    let store = Store::memory().unwrap();
    let original = message("42.55");
    store.upsert(vec![original.clone()]).await.unwrap();
    let id = MailSelectionId::default();
    let selection = store
        .capture_selection(
            id,
            1,
            MailQuery {
                folder: "INBOX".into(),
                ..Default::default()
            },
            true,
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(selection.available, 1);
    let (_, record) = prepare(&store, "42.55", "work").await;
    let selection = store
        .selection_snapshot(id, vec![original.summary.id])
        .await
        .unwrap();
    assert_eq!(
        (selection.selected, selection.available, selection.unread),
        (1, 0, 0)
    );
    assert!(
        selection.accounts.is_empty()
            && selection.groups.is_empty()
            && selection.visible.is_empty()
    );
    let review = store
        .freeze_selection(id, selection.revision)
        .await
        .unwrap();
    let job = store
        .start_bulk(
            "too-late".into(),
            review.id,
            Action::Flags(Flags {
                unread: Some(false),
                starred: None,
            }),
        )
        .await
        .unwrap();
    assert_eq!((job.total, job.failed, job.remaining), (1, 1, 0));
    assert!(store.claim_bulk_item(job.id).await.unwrap().is_none());
    assert_eq!(
        store.mail_move(record.token).await.unwrap().stage,
        MoveStage::Started
    );
}

#[tokio::test]
async fn legacy_copy_migration_is_atomic_and_never_forgets_unconfirmed_uploads() {
    for stage in ["uploading", "copied"] {
        let store = Store::memory().unwrap();
        let original = message("42.56");
        store.upsert(vec![original.clone()]).await.unwrap();
        let key = format!("transfer:{}", original.summary.id);
        let tuple = Some(("personal".to_owned(), "Keep".to_owned(), stage.to_owned()));
        store.put(&key, tuple.clone()).await.unwrap();
        let receipt = MoveReceipt::server(
            &original.summary,
            "personal",
            "Keep",
            None,
            Fingerprint::of(&original.raw),
        );
        let record = MoveRecord::new(original.summary.clone(), receipt);
        let mut wrong = record.clone();
        wrong.receipt.folder = "Wrong".into();
        assert!(store.adopt_legacy_mail_move(wrong).await.is_err());
        assert_eq!(
            store
                .get::<Option<(String, String, String)>>(&key)
                .await
                .unwrap(),
            tuple
        );
        let adopted = store.adopt_legacy_mail_move(record.clone()).await.unwrap();
        assert_eq!(
            adopted.stage,
            if stage == "copied" {
                MoveStage::Copied
            } else {
                MoveStage::Started
            }
        );
        assert!(adopted.receipt.current.is_none());
        assert!(
            store
                .get::<Option<(String, String, String)>>(&key)
                .await
                .unwrap()
                .is_none()
        );
        assert!(store.remove(original.summary.id).await.is_err());
        assert!(store.adopt_legacy_mail_move(record).await.is_err());
    }
}

#[tokio::test]
async fn automatic_lookup_is_bounded_fair_and_excludes_unconfirmed_operations() {
    let store = Store::memory().unwrap();
    for i in 60..=66 {
        let (_, record) = prepare(&store, &format!("42.{i}"), "work").await;
        if i != 66 {
            store
                .checkpoint_mail_move(record.clone(), MoveStage::Committed, record.receipt)
                .await
                .unwrap();
        }
    }
    let first = store.mail_move_lookups(1000).await.unwrap();
    assert_eq!(first.len(), 3);
    for record in &first {
        let attempted = store
            .begin_mail_move_lookup(record.clone(), 1000)
            .await
            .unwrap();
        store
            .fail_mail_move(attempted, "Temporary lookup error".into())
            .await
            .unwrap();
        assert!(
            store
                .begin_mail_move_lookup(record.clone(), 1001)
                .await
                .is_err()
        );
    }
    let next = store.mail_move_lookups(1001).await.unwrap();
    assert_eq!(next.len(), 3);
    assert!(
        next.iter()
            .all(|record| !first.iter().any(|old| old.token == record.token))
    );
    for record in next {
        store.begin_mail_move_lookup(record, 1001).await.unwrap();
    }
    assert!(store.mail_move_lookups(1002).await.unwrap().is_empty());
    assert_eq!(store.mail_move_lookups(1060).await.unwrap().len(), 3);
}

#[tokio::test]
async fn reviewed_local_recovery_preserves_content_flags_and_uses_no_obsolete_server_id() {
    for stage in [MoveStage::Started, MoveStage::Copied, MoveStage::Committed] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.sqlite");
        let store = Store::open(&path).unwrap();
        let (original, mut record) = prepare(&store, "42.90", "personal").await;
        if stage != MoveStage::Started {
            record = store
                .checkpoint_mail_move(record.clone(), stage, record.receipt.clone())
                .await
                .unwrap();
        }
        assert!(store.keep_mail_move(record.clone(), false).await.is_err());
        let stale = record.clone();
        record = store
            .fail_mail_move(record, "Test interrupted connection".into())
            .await
            .unwrap();
        assert!(store.keep_mail_move(stale, true).await.is_err());
        let kept = store.keep_mail_move(record, true).await.unwrap();
        assert_eq!(kept.stage, MoveStage::Kept);
        assert!(
            kept.receipt.current.is_none(),
            "A local decision is not a server receipt"
        );
        let mail = kept.retained.clone().unwrap();
        assert!(mail.is_local_copy());
        assert_ne!(mail.id, original.summary.id);
        assert_eq!(
            (&mail.account_id, &mail.folder),
            (&original.summary.account_id, &original.summary.folder)
        );
        assert_eq!(store.workspace().await.unwrap().move_pending_total, 0);
        assert!(store.keep_mail_move(kept.clone(), true).await.is_err());
        drop(store);
        let store = Store::open(&path).unwrap();
        store
            .apply_sync(MailSyncItem::Reconcile {
                account: "work".into(),
                folder: "INBOX".into(),
                live_ids: Default::default(),
            })
            .await
            .unwrap();
        assert_eq!(
            store.raw_message(mail.id.clone()).await.unwrap(),
            original.raw
        );
        let detail = store.detail(original.summary.id.clone()).await.unwrap();
        assert_eq!(
            detail.summary.id, mail.id,
            "A late body load follows the retained identity"
        );
        assert!(detail.summary.unread);
        assert!(!detail.summary.starred);
        let page = store
            .query(MailQuery {
                observe: vec![original.summary.id.clone()],
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.rows[0].id, mail.id);
        assert_eq!(page.relocated[&original.summary.id].id, mail.id);
        assert!(page.move_placeholders.is_empty());
        assert!(page.move_recovery.is_empty());
        assert_eq!(page.inbox_unread.get("work"), Some(&1));
        let mut moved = mail.clone();
        moved.folder = "Local folder".into();
        store
            .relocate_mail(mail.clone(), moved.clone())
            .await
            .unwrap();
        let detail = store.detail(original.summary.id).await.unwrap();
        assert_eq!(detail.summary.folder, "Local folder");
        assert!(detail.summary.is_local_copy());
        store.relocate_mail(moved, mail).await.unwrap();
        assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 1);
    }
}
