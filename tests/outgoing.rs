use shep::{compose, model::*, outgoing::*, store::Store};
fn account() -> Account {
    serde_json::from_value(serde_json::json!({"id":"work","name":"Work","email":"sender@example.test","protocol":"Imap","host":"imap.example.test","port":993,"username":"sender","smtp_host":"smtp.example.test","smtp_port":465})).unwrap()
}
fn draft(id: &str) -> Draft {
    Draft {
        id: id.into(),
        account_id: "work".into(),
        to: "friend@example.test".into(),
        bcc: "private@example.test".into(),
        subject: "Delivery recovery".into(),
        body: "Original text".into(),
        revision: 1,
        ..Default::default()
    }
}
fn submission(draft: &Draft) -> Submission {
    Submission::new(
        account(),
        draft,
        compose::build(&account(), draft, vec![]).unwrap(),
    )
    .unwrap()
}
async fn begin(store: &Store, draft: &Draft) -> OutgoingInfo {
    store.save_draft(draft.clone()).await.unwrap();
    store
        .begin_outgoing(submission(draft), draft.clone())
        .await
        .unwrap()
}

#[tokio::test]
async fn interrupted_submission_survives_restart_with_exact_wire_and_private_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    store.save_account(account()).await.unwrap();
    let draft = draft("one");
    store.save_draft(draft.clone()).await.unwrap();
    let wire = submission(&draft);
    let raw = wire.raw.clone();
    let id = wire.info.message_id.clone();
    assert!(!String::from_utf8_lossy(&raw).contains("private@example.test"));
    let started = store.begin_outgoing(wire, draft.clone()).await.unwrap();
    drop(store);
    let store = Store::open(path).unwrap();
    let pending = store
        .outgoing_submission(started.attempt.clone())
        .await
        .unwrap();
    assert_eq!(pending.raw, raw);
    assert_eq!(pending.info.message_id, id);
    assert!(pending.info.needs_delivery_review());
    assert_eq!(
        pending.envelope.to,
        ["friend@example.test", "private@example.test"]
    );
    assert!(
        store
            .begin_outgoing(submission(&draft), draft.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("delivery record")
    );
    assert_eq!(store.outgoing_page(0).await.unwrap().total, 1);
    assert_eq!(store.draft_state().await.unwrap().drafts.len(), 1);
}

#[tokio::test]
async fn definite_rejection_allows_new_attempt_but_stale_acknowledgments_cannot_touch_it() {
    let store = Store::memory().unwrap();
    let draft = draft("retry");
    let first = begin(&store, &draft).await;
    store
        .record_delivery(
            first.attempt.clone(),
            DeliveryState::Rejected,
            Some("SMTP 550".into()),
        )
        .await
        .unwrap();
    let second = store
        .begin_outgoing(submission(&draft), draft)
        .await
        .unwrap();
    assert_ne!(first.attempt, second.attempt);
    assert_ne!(first.message_id, second.message_id);
    assert!(
        store
            .record_delivery(first.attempt, DeliveryState::Accepted, None)
            .await
            .is_err()
    );
    assert_eq!(
        store.outgoing_info(second.attempt).await.unwrap().delivery,
        DeliveryState::Submitting
    );
}

#[tokio::test]
async fn acknowledged_send_recovers_atomic_local_copy_and_never_turns_copy_failure_into_resend() {
    let store = Store::memory().unwrap();
    let draft = draft("sent");
    let started = begin(&store, &draft).await;
    store
        .record_delivery(started.attempt.clone(), DeliveryState::Accepted, None)
        .await
        .unwrap();
    let wire = store
        .outgoing_submission(started.attempt.clone())
        .await
        .unwrap();
    let mail = parse_mail(
        "work",
        &started.local_remote_id(),
        "Sent",
        wire.raw.clone(),
        false,
        false,
    )
    .unwrap();
    store.run(|c| {c.execute_batch("CREATE TRIGGER fail_sent BEFORE INSERT ON draft_sent BEGIN SELECT RAISE(ABORT,'fixture disk failure'); END;")?;Ok(())}).await.unwrap();
    assert!(
        store
            .outgoing_local_sent(started.attempt.clone(), mail.clone())
            .await
            .is_err()
    );
    assert!(store.export().await.unwrap().is_empty());
    assert_eq!(store.draft_state().await.unwrap().drafts.len(), 1);
    assert_eq!(
        store
            .outgoing_info(started.attempt.clone())
            .await
            .unwrap()
            .delivery,
        DeliveryState::Accepted
    );
    assert!(
        store
            .record_sent_copy(started.attempt.clone(), SentState::LocalOnly, None, None)
            .await
            .is_err(),
        "A missing local copy must retain the only durable wire data"
    );
    store
        .run(|c| {
            c.execute_batch("DROP TRIGGER fail_sent")?;
            Ok(())
        })
        .await
        .unwrap();
    store
        .outgoing_local_sent(started.attempt.clone(), mail.clone())
        .await
        .unwrap();
    store
        .outgoing_local_sent(started.attempt.clone(), mail)
        .await
        .unwrap();
    assert_eq!(store.export().await.unwrap().len(), 1);
    assert!(store.draft_state().await.unwrap().drafts.is_empty());
    store.save_draft(draft.clone()).await.unwrap();
    assert!(store.draft_state().await.unwrap().drafts.is_empty());
    assert!(
        store
            .begin_outgoing(submission(&draft), draft)
            .await
            .is_err()
    );
    store
        .record_sent_copy(
            started.attempt.clone(),
            SentState::Appending,
            Some("Sent Mail".into()),
            None,
        )
        .await
        .unwrap();
    assert!(
        store
            .record_sent_copy(
                started.attempt.clone(),
                SentState::Appending,
                Some("Sent Mail".into()),
                None
            )
            .await
            .is_err()
    );
    assert!(
        store
            .release_outgoing(started.attempt.clone())
            .await
            .is_err()
    );
    store
        .record_sent_copy(
            started.attempt.clone(),
            SentState::Uncertain,
            Some("Sent Mail".into()),
            Some("Copy acknowledgment lost".into()),
        )
        .await
        .unwrap();
    assert!(
        store
            .record_sent_copy(
                started.attempt.clone(),
                SentState::Appending,
                Some("Sent Mail".into()),
                None
            )
            .await
            .is_err()
    );
    store
        .record_sent_copy(
            started.attempt.clone(),
            SentState::Saved,
            Some("Sent Mail".into()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(store.outgoing_page(0).await.unwrap().total, 0);
    assert!(
        store.outgoing_submission(started.attempt).await.is_err(),
        "Completed journal releases its duplicate wire data"
    );
    assert_eq!(store.export().await.unwrap()[0].raw, wire.raw);
}

#[tokio::test]
async fn recovery_preserves_flags_and_folder_changes_to_the_local_sent_copy() {
    let store = Store::memory().unwrap();
    let started = begin(&store, &draft("local-edits")).await;
    store
        .record_delivery(started.attempt.clone(), DeliveryState::Accepted, None)
        .await
        .unwrap();
    let wire = store
        .outgoing_submission(started.attempt.clone())
        .await
        .unwrap();
    let mail = parse_mail(
        "work",
        &started.local_remote_id(),
        "Sent",
        wire.raw.clone(),
        false,
        false,
    )
    .unwrap();
    store
        .outgoing_local_sent(started.attempt.clone(), mail.clone())
        .await
        .unwrap();
    let mut flagged = mail.summary.clone();
    flagged.starred = true;
    store.flags(flagged).await.unwrap();
    store
        .move_local(started.local_id(), "Archive".into())
        .await
        .unwrap();
    store
        .outgoing_local_sent(started.attempt.clone(), mail)
        .await
        .unwrap();
    let local = store.detail(started.local_id()).await.unwrap();
    assert!(local.summary.starred);
    assert_eq!(local.summary.folder, "Archive");
    store
        .record_sent_copy(
            started.attempt.clone(),
            SentState::Saved,
            Some("Sent Mail".into()),
            None,
        )
        .await
        .unwrap();
    store
        .upsert(vec![
            parse_mail("work", "42.7", "Sent Mail", wire.raw, false, false).unwrap(),
        ])
        .await
        .unwrap();
    assert!(
        store
            .detail(started.local_id())
            .await
            .unwrap()
            .summary
            .starred
    );
}

#[tokio::test]
async fn returning_uncertain_mail_to_drafts_preserves_edits_and_requires_a_new_send_action() {
    let store = Store::memory().unwrap();
    let mut draft = draft("reviewed");
    let started = begin(&store, &draft).await;
    store
        .record_delivery(started.attempt.clone(), DeliveryState::Uncertain, None)
        .await
        .unwrap();
    draft.body = "Edited after the interrupted send".into();
    draft.revision += 1;
    store.save_draft(draft.clone()).await.unwrap();
    assert!(
        store
            .begin_outgoing(submission(&draft), draft.clone())
            .await
            .is_err()
    );
    store.release_outgoing(started.attempt).await.unwrap();
    assert_eq!(
        store.draft_state().await.unwrap().drafts[0].body,
        draft.body
    );
    assert_eq!(store.outgoing_page(0).await.unwrap().total, 0);
    let next = store
        .begin_outgoing(submission(&draft), draft)
        .await
        .unwrap();
    assert_eq!(next.delivery, DeliveryState::Submitting);
}

#[tokio::test]
async fn stale_draft_cannot_be_committed_as_an_outgoing_attempt_and_pages_stay_bounded() {
    let store = Store::memory().unwrap();
    let old = draft("stale");
    let mut new = old.clone();
    new.body = "Newer".into();
    new.revision += 1;
    store.save_draft(new).await.unwrap();
    assert!(store.begin_outgoing(submission(&old), old).await.is_err());
    assert_eq!(store.outgoing_page(0).await.unwrap().total, 0);
    for n in 0..25 {
        begin(&store, &draft(&format!("draft-{n}"))).await;
    }
    let first = store.outgoing_page(0).await.unwrap();
    let last = store.outgoing_page(20).await.unwrap();
    assert_eq!(
        (first.total, first.rows.len(), last.rows.len()),
        (25, 20, 5)
    );
    assert!(
        first
            .rows
            .iter()
            .all(|a| last.rows.iter().all(|b| a.attempt != b.attempt))
    );
}

#[tokio::test]
async fn sync_reconciles_local_sent_only_with_matching_account_folder_and_identity() {
    for remote_first in [false, true] {
        let store = Store::memory().unwrap();
        let draft = draft("copied");
        let started = begin(&store, &draft).await;
        store
            .record_delivery(started.attempt.clone(), DeliveryState::Accepted, None)
            .await
            .unwrap();
        let wire = store
            .outgoing_submission(started.attempt.clone())
            .await
            .unwrap();
        store
            .outgoing_local_sent(
                started.attempt.clone(),
                parse_mail(
                    "work",
                    &started.local_remote_id(),
                    "Sent",
                    wire.raw.clone(),
                    false,
                    false,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let remote =
            parse_mail("work", "42.7", "Sent Mail", wire.raw.clone(), false, true).unwrap();
        for (account, folder) in [("personal", "Sent Mail"), ("work", "Archive")] {
            store
                .upsert(vec![
                    parse_mail(account, "42.7", folder, wire.raw.clone(), false, false).unwrap(),
                ])
                .await
                .unwrap();
        }
        if remote_first {
            store.upsert(vec![remote.clone()]).await.unwrap();
        }
        store
            .record_sent_copy(
                started.attempt.clone(),
                SentState::Saved,
                Some("Sent Mail".into()),
                None,
            )
            .await
            .unwrap();
        if !remote_first {
            assert!(store.raw_message(started.local_id()).await.is_ok());
            store.upsert(vec![remote.clone()]).await.unwrap();
        }
        assert!(store.raw_message(started.local_id()).await.is_err());
        assert_eq!(store.export().await.unwrap().len(), 3);
        assert!(
            store
                .detail(remote.summary.id)
                .await
                .unwrap()
                .summary
                .starred
        );
    }
}

#[tokio::test]
async fn removal_review_includes_delivery_recovery_and_rechecks_changed_outcomes() {
    use shep::store::{ConnectionKind, ConnectionRef};
    let store = Store::memory().unwrap();
    store.save_account(account()).await.unwrap();
    let info = begin(&store, &draft("remove")).await;
    let target = ConnectionRef {
        kind: ConnectionKind::Account,
        id: "work".into(),
    };
    let preview = store.removal_preview(target.clone()).await.unwrap();
    assert_eq!(preview.outgoing, 1);
    store
        .record_delivery(info.attempt.clone(), DeliveryState::Uncertain, None)
        .await
        .unwrap();
    assert!(store.remove_connection(preview, false).await.is_err());
    store
        .remove_connection(store.removal_preview(target).await.unwrap(), false)
        .await
        .unwrap();
    assert_eq!(store.outgoing_page(0).await.unwrap().total, 0);
    assert!(store.outgoing_submission(info.attempt).await.is_err());
}

#[tokio::test]
async fn unified_sent_uses_each_accounts_real_folder_and_keeps_local_only_copies() {
    let store = Store::memory().unwrap();
    store.save_account(account()).await.unwrap();
    store
        .save_folders(
            "work".into(),
            vec!["Sent".into(), "Sent Mail".into(), "INBOX".into()],
        )
        .await
        .unwrap();
    store
        .apply_sync(MailSyncItem::SentFolder(
            "work".into(),
            Some("Sent Mail".into()),
        ))
        .await
        .unwrap();
    for (account, uid, folder) in [
        ("work", "1.1", "Sent Mail"),
        ("work", "1.2", "Sent"),
        ("work", "local-sent-old", "Sent"),
        ("personal", "1.1", "Sent"),
        ("personal", "1.2", "Sent Mail"),
    ] {
        store
            .upsert(vec![
                parse_mail(
                    account,
                    uid,
                    folder,
                    b"From: sender@example.test\r\nSubject: Folder test\r\n\r\nBody".to_vec(),
                    false,
                    false,
                )
                .unwrap(),
            ])
            .await
            .unwrap();
    }
    assert_eq!(
        store
            .query(MailQuery {
                sent_only: true,
                folder: "Sent".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        3
    );
    assert_eq!(
        store
            .query(MailQuery {
                sent_only: true,
                account: Some("work".into()),
                folder: "Sent".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        2
    );
    assert_eq!(
        store
            .query(MailQuery {
                account: Some("work".into()),
                folder: "Sent".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        2
    );
    store
        .apply_sync(MailSyncItem::SentFolder("work".into(), None))
        .await
        .unwrap();
    assert_eq!(
        store
            .query(MailQuery {
                sent_only: true,
                folder: "Sent".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        3,
        "An optional missing role must not erase the known mapping"
    );
    store
        .save_folders("work".into(), vec!["Sent".into(), "INBOX".into()])
        .await
        .unwrap();
    assert_eq!(
        store
            .query(MailQuery {
                sent_only: true,
                account: Some("work".into()),
                ..Default::default()
            })
            .await
            .unwrap()
            .rows
            .iter()
            .filter(|m| m.folder == "Sent Mail")
            .count(),
        0
    );
}
