use super::*;

fn account() -> Account {
    serde_json::from_value(serde_json::json!({"id":"removal","name":"Removal account","email":"removal@example.test","protocol":"Imap","host":"example.test","port":993,"username":"removal","smtp_host":"example.test","smtp_port":465})).expect("account")
}
async fn seed(store: &Store, count: usize) {
    store.save_account(account()).await.expect("account");
    let mail = (0..count)
        .map(|id| {
            crate::model::parse_mail(
                "removal",
                &id.to_string(),
                "INBOX",
                b"Subject: Retained until cleanup\r\n\r\nBody".to_vec(),
                true,
                false,
            )
            .expect("fixture mail")
        })
        .collect();
    store.upsert(mail).await.expect("mail");
}
fn target() -> ConnectionRef {
    ConnectionRef {
        kind: ConnectionKind::Account,
        id: "removal".into(),
    }
}

#[tokio::test]
async fn failed_admission_rolls_back_hidden_state_and_can_retry_same_review() {
    let store = Store::memory().expect("store");
    seed(&store, 1).await;
    let review = store.removal_preview(target()).await.expect("review");
    let id = uuid::Uuid::new_v4().to_string();
    store.run(|c| {c.execute_batch("CREATE TRIGGER refuse_removal_admission BEFORE INSERT ON connection_tombstones BEGIN SELECT RAISE(FAIL,'fixture admission failure'); END")?;Ok(())}).await.expect("failure fixture");
    assert!(
        store
            .admit_connection_removal(id.clone(), review.clone(), true)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .workspace()
            .await
            .expect("retained account")
            .accounts
            .len(),
        1
    );
    assert_eq!(
        store
            .query(crate::model::MailQuery::default())
            .await
            .expect("retained rows")
            .total,
        1
    );
    assert!(store.removal_job(id.clone()).await.is_err());
    store
        .run(|c| {
            c.execute_batch("DROP TRIGGER refuse_removal_admission")?;
            Ok(())
        })
        .await
        .expect("restore storage");
    store
        .admit_connection_removal(id, review, true)
        .await
        .expect("retry unchanged input");
}

#[tokio::test]
async fn removal_hides_a_retained_physical_source_projected_into_another_account() {
    let store = Store::memory().expect("store");
    seed(&store, 1).await;
    let mut other = account();
    other.id = "other".into();
    store
        .save_account(other)
        .await
        .expect("destination account");
    let mail = store
        .query(crate::model::MailQuery::default())
        .await
        .expect("source")
        .rows
        .remove(0);
    store
        .start_individual_mail_action(
            uuid::Uuid::new_v4().to_string(),
            mail.clone(),
            crate::bulk::Action::Move {
                account: Some("other".into()),
                folder: "INBOX".into(),
            },
        )
        .await
        .expect("projected transfer");
    assert_eq!(
        store
            .query(crate::model::MailQuery::default())
            .await
            .expect("projected destination")
            .rows[0]
            .account_id,
        "other"
    );
    admit(&store).await;
    let page = store
        .query(crate::model::MailQuery {
            observe: vec![mail.id],
            ..Default::default()
        })
        .await
        .expect("hidden physical ownership");
    assert_eq!(page.total, 0);
    assert!(page.inbox_unread.is_empty());
    assert_eq!(page.observed.len(), 1);
    assert!(page.observed.values().all(Option::is_none));
    assert_eq!(
        store
            .run(|c| Ok(c.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))?))
            .await
            .expect("retained until drain"),
        1
    );
}
async fn admit(store: &Store) -> RemovalJob {
    let preview = store.removal_preview(target()).await.expect("review");
    store
        .admit_connection_removal(uuid::Uuid::new_v4().to_string(), preview, true)
        .await
        .expect("admit")
}

#[tokio::test]
async fn admission_hides_rows_and_counts_before_bounded_cleanup_and_survives_restart() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("cache.sqlite");
    let store = Store::open(&path).expect("store");
    seed(&store, 120).await;
    let query = crate::model::MailQuery::default();
    let original = store.query(query.clone()).await.expect("page");
    assert_eq!(original.total, 120);
    let job = admit(&store).await;
    let page = store.query(query.clone()).await.expect("hidden");
    assert_eq!(page.total, 0);
    assert!(page.inbox_unread.is_empty());
    assert!(
        store
            .workspace()
            .await
            .expect("workspace")
            .accounts
            .is_empty()
    );
    assert!(store.detail(original.rows[0].id.clone()).await.is_err());
    assert!(
        store
            .export()
            .await
            .expect("portable export excludes removed mail")
            .is_empty()
    );
    assert!(
        store
            .raw_message(original.rows[0].id.clone())
            .await
            .is_err()
    );
    assert!(
        store
            .mail_metadata(original.rows[0].id.clone())
            .await
            .is_err()
    );
    assert_eq!(
        store
            .run(|c| Ok(c.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))?))
            .await
            .expect("physical"),
        120
    );
    let job = store
        .finish_connection_removal(job)
        .await
        .expect("first page");
    assert!(!job.local_done);
    assert_eq!(
        store
            .run(|c| Ok(c.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))?))
            .await
            .expect("remaining"),
        70
    );
    let id = job.id.clone();
    drop(store);
    let store = Store::open(path).expect("restart");
    assert_eq!(
        store
            .query(query)
            .await
            .expect("hidden after restart")
            .total,
        0
    );
    let mut job = store.removal_job(id).await.expect("same job");
    while !job.local_done {
        job = store
            .finish_connection_removal(job)
            .await
            .expect("next page");
    }
    assert_eq!(
        store
            .run(|c| Ok(c.query_row("SELECT count(*) FROM messages", [], |r| r.get::<_, i64>(0))?))
            .await
            .expect("removed"),
        0
    );
}

#[tokio::test]
async fn changed_review_rejects_without_detaching_and_lost_reply_reuses_saved_identity() {
    let store = Store::memory().expect("store");
    seed(&store, 1).await;
    let stale = store.removal_preview(target()).await.expect("review");
    let mut changed = account();
    changed.name = "Newer name".into();
    store
        .save_account(changed.clone())
        .await
        .expect("new settings");
    assert!(
        store
            .admit_connection_removal(uuid::Uuid::new_v4().to_string(), stale, true)
            .await
            .is_err()
    );
    assert_eq!(
        store.workspace().await.expect("not detached").accounts[0],
        changed
    );
    let preview = store.removal_preview(target()).await.expect("review");
    let id = uuid::Uuid::new_v4().to_string();
    let saved = store
        .admit_connection_removal(id.clone(), preview.clone(), true)
        .await
        .expect("saved");
    assert_eq!(
        store
            .admit_connection_removal(id, preview, true)
            .await
            .expect("same reply"),
        saved
    );
    assert!(
        store
            .start_individual_mail_action(
                "late".into(),
                crate::model::parse_mail(
                    "removal",
                    "0",
                    "INBOX",
                    b"Subject: Late\r\n\r\nBody".to_vec(),
                    true,
                    false
                )
                .expect("fixture mail")
                .summary,
                crate::bulk::Action::Flags(crate::mail_actions::Flags {
                    unread: Some(false),
                    starred: None
                })
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn old_admission_and_completion_cannot_remove_a_reconnected_identity() {
    let store = Store::memory().expect("store");
    seed(&store, 1).await;
    let old = store.removal_preview(target()).await.expect("review");
    let job = admit(&store).await;
    let old_job = job.clone();
    assert!(
        store
            .run(|c| restore_removed(c, ConnectionKind::Account, "removal"))
            .await
            .is_err()
    );
    let job = store
        .finish_connection_removal(job)
        .await
        .expect("local deletion");
    assert!(job.local_done);
    assert!(
        store
            .run(|c| revive(c, ConnectionKind::Account, "removal"))
            .await
            .is_err()
    );
    for key in store.cleanup_jobs().await.expect("cleanup") {
        store
            .finish_credential_cleanup(key)
            .await
            .expect("delete fixture key");
    }
    store
        .finish_removal_credentials(target())
        .await
        .expect("cleanup receipt");
    store
        .run(|c| revive(c, ConnectionKind::Account, "removal"))
        .await
        .expect("explicit reconnect");
    store
        .save_account(account())
        .await
        .expect("same configuration");
    assert!(
        store
            .admit_connection_removal(uuid::Uuid::new_v4().to_string(), old, true)
            .await
            .is_err()
    );
    assert!(store.finish_connection_removal(old_job).await.is_err());
    assert_eq!(
        store
            .workspace()
            .await
            .expect("new binding retained")
            .accounts
            .len(),
        1
    );
}

#[tokio::test]
async fn admitted_removal_fences_queued_claims_but_retains_receipts_until_drain() {
    let store = Store::memory().expect("store");
    seed(&store, 2).await;
    let page = store
        .query(crate::model::MailQuery::default())
        .await
        .expect("page");
    let action = crate::bulk::Action::Flags(crate::mail_actions::Flags {
        unread: None,
        starred: Some(true),
    });
    let first = store
        .start_individual_mail_action(
            uuid::Uuid::new_v4().to_string(),
            page.rows[0].clone(),
            action.clone(),
        )
        .await
        .expect("first admission");
    let second = store
        .start_individual_mail_action(
            uuid::Uuid::new_v4().to_string(),
            page.rows[1].clone(),
            action,
        )
        .await
        .expect("second admission");
    let active = store
        .claim_bulk_item(first.id.clone())
        .await
        .expect("claim")
        .expect("active item");
    let before = store
        .bulk_item_state(first.id.clone(), active.position)
        .await
        .expect("active receipt");
    let removal = admit(&store).await;
    assert!(
        store
            .claim_bulk_item(second.id.clone())
            .await
            .expect("fenced queued claim")
            .is_none()
    );
    assert!(
        store
            .next_action_work(0, String::new(), vec![], vec![], false)
            .await
            .expect("scheduler fence")
            .is_none()
    );
    assert_eq!(
        store
            .bulk_item_state(first.id.clone(), active.position)
            .await
            .expect("active receipt retained"),
        before
    );
    store
        .acknowledge_bulk_flags(
            active,
            crate::bulk::Receipt::Flags {
                before: crate::mail_actions::Flags {
                    unread: None,
                    starred: Some(false),
                },
                after: crate::mail_actions::Flags {
                    unread: None,
                    starred: Some(true),
                },
            },
        )
        .await
        .expect("already dispatched acknowledgement remains durable");
    assert_eq!(
        store
            .bulk_items(first.id, None)
            .await
            .expect("retained receipt")[0]
            .status,
        "repair"
    );
    assert_eq!(
        store
            .bulk_items(second.id, None)
            .await
            .expect("queued retained")[0]
            .status,
        "queued"
    );
    assert!(
        store
            .next_action_work(5, String::new(), vec![], vec![], false)
            .await
            .expect("removal runnable")
            .is_some()
    );
    assert!(!removal.local_done);
}
