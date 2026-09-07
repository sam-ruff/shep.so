use shep::{
    model::*,
    store::{ConnectionKind, ConnectionRef, Store},
};

fn target(kind: ConnectionKind, id: &str) -> ConnectionRef {
    ConnectionRef {
        kind,
        id: id.into(),
    }
}
fn account(id: &str) -> Account {
    serde_json::from_value(serde_json::json!({"id":id,"name":id,"email":format!("{id}@example.test"),"protocol":"Imap","host":"imap.example.test","port":993,"username":id,"smtp_host":"smtp.example.test","smtp_port":465})).unwrap()
}
fn mail(account: &str, uid: &str) -> StoredMail {
    parse_mail(account, uid, "INBOX", format!("From: friend@example.test\r\nMessage-ID: <{uid}@example.test>\r\nSubject: Searchable keepsake\r\n\r\nOriginal content").into_bytes(), false, false).unwrap()
}
fn source(id: &str, kind: CalendarKind) -> CalendarSource {
    CalendarSource {
        id: id.into(),
        name: id.into(),
        url: format!("https://calendar.example.test/{id}/"),
        username: "alex".into(),
        kind,
        access: Default::default(),
    }
}
fn event(source: &str) -> CalendarEvent {
    let start = chrono::DateTime::parse_from_rfc3339("2026-09-06T09:00:00Z")
        .unwrap()
        .to_utc();
    CalendarEvent {
        id: "shared-uid".into(),
        source_id: source.into(),
        title: source.into(),
        start,
        end: start + chrono::Duration::hours(1),
        all_day: false,
        etag: None,
        remote_url: None,
        location: String::new(),
        description: String::new(),
    }
}
fn draft() -> Draft {
    Draft {
        id: "draft".into(),
        account_id: "work".into(),
        subject: "Unsent thoughts".into(),
        body: "Keep until reviewed".into(),
        revision: 1,
        ..Default::default()
    }
}

#[tokio::test]
async fn removal_cleans_only_reviewed_account_and_blocks_late_writes_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.sqlite");
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "Private draft attachment").unwrap();
    let store = Store::open(&path).unwrap();
    for id in ["work", "personal"] {
        store.save_account(account(id)).await.unwrap();
        store
            .save_folders(id.into(), vec!["INBOX".into(), id.into()])
            .await
            .unwrap();
        store.upsert(vec![mail(id, "1")]).await.unwrap();
    }
    let files = store
        .add_draft_files(draft(), vec![file.clone()])
        .await
        .unwrap();
    let reviewed = store
        .removal_preview(target(ConnectionKind::Account, "work"))
        .await
        .unwrap();
    assert_eq!(
        (reviewed.messages, reviewed.drafts, reviewed.transfers),
        (1, 1, 0)
    );
    store
        .remove_connection(reviewed.clone(), false)
        .await
        .unwrap();
    store.remove_connection(reviewed, false).await.unwrap(); // duplicate completion is harmless
    drop(store);
    let store = Store::open(path).unwrap();
    let ws = store.workspace().await.unwrap();
    assert_eq!(ws.accounts.len(), 1);
    assert_eq!(ws.accounts[0].id, "personal");
    assert!(!ws.account_folders.contains_key("work"));
    assert!(ws.account_folders.contains_key("personal"));
    assert!(ws.drafts.is_empty());
    assert_eq!(ws.credential_cleanup, 2);
    assert_eq!(
        store.export().await.unwrap()[0].summary.account_id,
        "personal"
    );
    assert_eq!(
        store
            .query(MailQuery {
                search: "keepsake".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        1
    );
    assert!(
        store
            .raw_message(mail("work", "1").summary.id)
            .await
            .is_err()
    );
    assert!(store.draft_files(files.drafts[0].clone()).await.is_err());
    assert!(store.save_draft(draft()).await.is_err());
    assert!(store.add_draft_files(draft(), vec![file]).await.is_err());
    assert!(store.save_account(account("work")).await.is_err());
    assert!(
        store
            .save_folders("work".into(), vec!["INBOX".into()])
            .await
            .is_err()
    );
    assert!(store.upsert(vec![mail("work", "2")]).await.is_err());
    store
        .run(|c| {
            let count: i64 = c.query_row(
                "SELECT count(*) FROM conversation_tokens WHERE account='work'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(count, 0);
            let count: i64 =
                c.query_row("SELECT count(*) FROM draft_attachments", [], |r| r.get(0))?;
            assert_eq!(count, 0);
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn review_rejects_same_count_replacements_text_edits_and_new_attachments() {
    let store = Store::memory().unwrap();
    store.save_account(account("work")).await.unwrap();
    store.upsert(vec![mail("work", "1")]).await.unwrap();
    store.save_draft(draft()).await.unwrap();
    let t = target(ConnectionKind::Account, "work");
    let before = store.removal_preview(t.clone()).await.unwrap();
    store.remove(mail("work", "1").summary.id).await.unwrap();
    store.upsert(vec![mail("work", "2")]).await.unwrap();
    assert!(
        store
            .remove_connection(before, false)
            .await
            .unwrap_err()
            .to_string()
            .contains("Local data changed")
    );
    let before = store.removal_preview(t.clone()).await.unwrap();
    let mut edit = draft();
    edit.body = "New unsent text".into();
    edit.revision += 1;
    store.save_draft(edit.clone()).await.unwrap();
    assert!(store.remove_connection(before, false).await.is_err());
    let before = store.removal_preview(t.clone()).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("new.txt");
    std::fs::write(&file, "A new attachment").unwrap();
    store.add_draft_files(edit, vec![file]).await.unwrap();
    assert!(store.remove_connection(before, false).await.is_err());
    let fresh = store.removal_preview(t).await.unwrap();
    store.remove_connection(fresh, false).await.unwrap();
}

#[tokio::test]
async fn calendar_removal_is_scoped_and_google_refresh_cannot_reconnect_it() {
    let store = Store::memory().unwrap();
    let home = source("home", CalendarKind::CalDav);
    let google = source("google", CalendarKind::Google);
    store
        .save_sources(vec![home.clone(), google.clone()])
        .await
        .unwrap();
    store.save_event(event("home")).await.unwrap();
    store.save_event(event("google")).await.unwrap();
    let (revision, _) = store.calendar_snapshot().await.unwrap();
    let old_connections = store.workspace().await.unwrap().connections_revision;
    let preview = store
        .removal_preview(target(ConnectionKind::Calendar, "google"))
        .await
        .unwrap();
    assert_eq!(preview.events, 1);
    store.remove_connection(preview, false).await.unwrap();
    let (removed_revision, events) = store.calendar_snapshot().await.unwrap();
    assert!(removed_revision > revision);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source_id, "home");
    assert!(store.save_event(event("google")).await.is_err());
    assert!(
        store
            .replace_events("google".into(), vec![event("google")])
            .await
            .is_err()
    );
    store
        .refresh_google_sources(vec![google.clone()])
        .await
        .unwrap();
    assert_eq!(store.workspace().await.unwrap().calendars, vec![home]);
    assert_eq!(
        store.removed_google_calendars().await.unwrap(),
        vec![google.clone()]
    );
    assert!(
        store.cleanup_jobs().await.unwrap().is_empty(),
        "Removing one calendar must retain the shared Google credential"
    );
    assert!(
        store
            .check_calendar_reconnect("google".into(), old_connections)
            .await
            .is_err()
    );
    let current = store.workspace().await.unwrap().connections_revision;
    store
        .check_calendar_reconnect("google".into(), current)
        .await
        .unwrap();
    store.save_source(google).await.unwrap();
    assert!(
        store
            .check_calendar_reconnect("google".into(), old_connections)
            .await
            .is_err(),
        "A stale setup stays stale after an explicit reconnect"
    );
    assert!(store.removed_google_calendars().await.unwrap().is_empty());
    store.save_event(event("google")).await.unwrap();
    assert_eq!(store.events().await.unwrap().len(), 2);
}

#[tokio::test]
async fn pending_moves_require_explicit_cancellation_and_other_journals_survive() {
    let store = Store::memory().unwrap();
    for id in ["work", "personal", "third"] {
        store.save_account(account(id)).await.unwrap();
        store.upsert(vec![mail(id, "1")]).await.unwrap();
    }
    for (from, to) in [
        ("work", "personal"),
        ("personal", "work"),
        ("third", "personal"),
    ] {
        store
            .put(
                &format!("transfer:{}", mail(from, "1").summary.id),
                Some((to.to_string(), "INBOX".to_string(), "uploaded".to_string())),
            )
            .await
            .unwrap();
    }
    let preview = store
        .removal_preview(target(ConnectionKind::Account, "work"))
        .await
        .unwrap();
    assert_eq!(preview.transfers, 2);
    assert!(
        store
            .remove_connection(preview.clone(), false)
            .await
            .is_err()
    );
    assert_eq!(store.workspace().await.unwrap().accounts.len(), 3);
    store.remove_connection(preview, true).await.unwrap();
    for id in ["work", "personal", "third"] {
        let journal: Option<(String, String, String)> = store
            .get(&format!("transfer:{}", mail(id, "1").summary.id))
            .await
            .unwrap();
        assert_eq!(journal.is_some(), id == "third");
    }
    assert_eq!(store.export().await.unwrap().len(), 2);
}

#[tokio::test]
async fn failed_removal_rolls_back_data_indexes_credentials_and_tombstone() {
    let store = Store::memory().unwrap();
    store.save_account(account("work")).await.unwrap();
    store.upsert(vec![mail("work", "1")]).await.unwrap();
    store.save_draft(draft()).await.unwrap();
    let preview = store
        .removal_preview(target(ConnectionKind::Account, "work"))
        .await
        .unwrap();
    let revision = store.workspace().await.unwrap().connections_revision;
    store.run(|c| { c.execute_batch("CREATE TRIGGER fail_removal BEFORE INSERT ON credential_cleanup BEGIN SELECT RAISE(ABORT,'fixture failure'); END;")?; Ok(()) }).await.unwrap();
    assert!(
        store
            .remove_connection(preview.clone(), false)
            .await
            .is_err()
    );
    assert_eq!(
        store.workspace().await.unwrap().connections_revision,
        revision
    );
    assert_eq!(
        store
            .removal_preview(preview.target.clone())
            .await
            .unwrap()
            .fingerprint,
        preview.fingerprint
    );
    assert_eq!(
        store
            .query(MailQuery {
                search: "keepsake".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        1
    );
    assert!(store.cleanup_jobs().await.unwrap().is_empty());
    store.check_connection(preview.target).await.unwrap();
}

#[tokio::test]
async fn removing_an_account_reviews_pending_groups_and_cleans_only_its_history() {
    use shep::{bulk::Action, store::MailSelectionId};
    let store = Store::memory().unwrap();
    for id in ["work", "personal"] {
        store.save_account(account(id)).await.unwrap();
        store.upsert(vec![mail(id, "1")]).await.unwrap();
    }
    let source = MailSelectionId::default();
    store
        .capture_selection(source, 0, MailQuery::default(), true, vec![])
        .await
        .unwrap();
    let frozen = store.freeze_selection(source, 0).await.unwrap();
    store
        .start_bulk(
            "shared".into(),
            frozen.id,
            Action::Move {
                account: None,
                folder: "Archive".into(),
            },
        )
        .await
        .unwrap();
    let selected = target(ConnectionKind::Account, "work");
    let review = store.removal_preview(selected.clone()).await.unwrap();
    assert_eq!((review.mail_history, review.transfers), (1, 1));
    assert!(
        store
            .remove_connection(review.clone(), false)
            .await
            .is_err()
    );
    let mut step = store
        .claim_bulk_item("shared".into())
        .await
        .unwrap()
        .unwrap();
    if step.original.as_ref().unwrap().account_id != "work" {
        store
            .finish_bulk_item(step, Err(("Other account fixture result".into(), false)))
            .await
            .unwrap();
        assert_eq!(
            store
                .removal_preview(selected.clone())
                .await
                .unwrap()
                .fingerprint,
            review.fingerprint,
            "Another account's result must not change this account's reviewed scope"
        );
        step = store
            .claim_bulk_item("shared".into())
            .await
            .unwrap()
            .unwrap();
    }
    assert_eq!(step.original.as_ref().unwrap().account_id, "work");
    assert!(
        store.remove_connection(review, true).await.is_err(),
        "A changed group needs a fresh review"
    );
    store
        .finish_bulk_item(step, Err(("Cancelled fixture step".into(), false)))
        .await
        .unwrap();
    let review = store.removal_preview(selected).await.unwrap();
    store.remove_connection(review, true).await.unwrap();
    let remaining = store.bulk_items("shared".into(), None).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(
        remaining[0].original.as_ref().unwrap().account_id,
        "personal"
    );
    let archive = store
        .query(MailQuery {
            folder: "Archive".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(archive.rows.iter().all(|m| m.account_id == "personal"));
    let personal = store
        .removal_preview(target(ConnectionKind::Account, "personal"))
        .await
        .unwrap();
    store.remove_connection(personal, true).await.unwrap();
    assert!(store.bulk_jobs(0).await.unwrap().is_empty());
}
