use super::*;
use crate::profile_sync::account_reviews::{self as reviews, Choice};

async fn changed(path: &std::path::Path) -> (Store, Replica, history::Binding, Account) {
    let (store, mut replica, binding, _) = fixture(path).await;
    let operation = shep_profile_core::Operation::decode(include_bytes!(
        "../../../../tests/support/profile-operation.json"
    ))
    .unwrap();
    let Action::AccountConnection { account: mut wire } = operation.changes[0].action.clone()
    else {
        panic!("connection")
    };
    edit_remote(
        &mut replica,
        vec![
            operation.changes[0].clone(),
            Change {
                action: Action::AccountName {
                    id: wire.id,
                    name: "Original account".into(),
                },
                extra: Default::default(),
            },
        ],
    )
    .await;
    apply_observed(&store, &replica).await;
    let original = store
        .get::<Vec<Account>>("accounts")
        .await
        .unwrap()
        .remove(0);
    // Stand in for the prior device reconnect; these store tests never read a keychain.
    store.save_account(original.clone()).await.unwrap();
    let mail = parse_mail(
        &original.id,
        "same-server-id",
        "INBOX",
        b"Subject: Old server mail\r\n\r\nPreserved original".to_vec(),
        true,
        false,
    )
    .unwrap();
    store.upsert(vec![mail]).await.unwrap();
    wire.host = "new-incoming.example.test".into();
    wire.smtp_host = "new-outgoing.example.test".into();
    edit_remote(
        &mut replica,
        vec![Change {
            action: Action::AccountConnection { account: wire },
            extra: Default::default(),
        }],
    )
    .await;
    assert_eq!(apply_observed(&store, &replica).await.review, 1);
    (store, replica, binding, original)
}

#[tokio::test]
async fn profile_account_review_adds_separate_connection_and_preserves_old_mail_identity() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, binding, original) = changed(dir.path()).await;
    let review = reviews::prepare(&store, &replica, None)
        .await
        .unwrap()
        .reviews
        .remove(0);
    let chosen = review.versions()[0].operation;
    reviews::accept(&store, &mut replica, review, Choice::AddShared(chosen))
        .await
        .unwrap();
    let accounts = store.get::<Vec<Account>>("accounts").await.unwrap();
    assert_eq!(accounts.len(), 2);
    let previous = accounts.iter().find(|a| a.id == original.id).unwrap();
    let current = accounts.iter().find(|a| a.id != original.id).unwrap();
    assert_eq!(previous.host, original.host);
    assert_eq!(previous.smtp_host, original.smtp_host);
    assert_eq!(previous.name, "Original account (previous setup)");
    assert_eq!(current.name, "Original account");
    assert_eq!(current.host, "new-incoming.example.test");
    assert!(
        store
            .require_account_reconnected(current.id.clone())
            .await
            .is_err()
    );
    store
        .require_account_reconnected(previous.id.clone())
        .await
        .unwrap();
    let state = store.profile_replication(binding).await.unwrap();
    assert_eq!(state.accounts.len(), 1);
    assert!(state.accounts.contains_key(&current.id));
    assert!(state.local_only.contains(&original.id));
    let mail = parse_mail(
        &current.id,
        "same-server-id",
        "INBOX",
        b"Subject: New server mail\r\n\r\nNew content".to_vec(),
        false,
        false,
    )
    .unwrap();
    store.upsert(vec![mail]).await.unwrap();
    let page = store.query(MailQuery::default()).await.unwrap();
    assert_eq!(page.total, 2);
    assert!(
        page.rows
            .iter()
            .any(|m| m.account_id == original.id && m.subject == "Old server mail")
    );
    assert!(store.capture_profile_change().await.unwrap().is_none());
    assert!(
        reviews::prepare(&store, &replica, None)
            .await
            .unwrap()
            .reviews
            .is_empty()
    );
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_account_review_keep_local_publishes_only_the_reviewed_connection() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, _, original) = changed(dir.path()).await;
    let review = reviews::prepare(&store, &replica, None)
        .await
        .unwrap()
        .reviews
        .remove(0);
    store
        .update_preferences(|p| p.interface_scale = 120)
        .await
        .unwrap();
    reviews::accept(&store, &mut replica, review, Choice::Local)
        .await
        .unwrap();
    assert_eq!(
        store.get::<Vec<Account>>("accounts").await.unwrap(),
        vec![original]
    );
    assert_eq!(
        store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .interface_scale,
        120
    );
    assert!(
        reviews::prepare(&store, &replica, None)
            .await
            .unwrap()
            .reviews
            .is_empty()
    );
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_account_review_rejects_new_native_settings_history_and_disabled_consent() {
    for reason in ["native", "history", "consent"] {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut replica, _, original) = changed(dir.path()).await;
        let review = reviews::prepare(&store, &replica, None)
            .await
            .unwrap()
            .reviews
            .remove(0);
        let choice = Choice::AddShared(review.versions()[0].operation);
        match reason {
            "native" => {
                let mut account = original.clone();
                account.sent_folder = "New sent folder".into();
                store.save_account(account).await.unwrap();
            }
            "history" => remote_appearance(&mut replica, "Dark").await,
            "consent" => {
                store
                    .change_profile_sync_options(enrollment::Changes {
                        accounts: Some(false),
                        ..Default::default()
                    })
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            reviews::accept(&store, &mut replica, review, choice)
                .await
                .is_err()
        );
        let accounts = store.get::<Vec<Account>>("accounts").await.unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].host, original.host);
        if reason == "native" {
            assert_eq!(accounts[0].sent_folder, "New sent folder");
        }
        replica.close().await.unwrap();
    }
}

#[tokio::test]
async fn profile_account_review_restart_admits_the_reserved_identity_without_duplicate_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let (store, replica, binding, original) = changed(dir.path()).await;
    let review = reviews::prepare(&store, &replica, None)
        .await
        .unwrap()
        .reviews
        .remove(0);
    let change = replica
        .value(
            reviews::target(review.basis.shared),
            review.versions()[0].operation,
        )
        .await
        .unwrap();
    let pending = store
        .reserve_profile_account_review(review, change, true)
        .await
        .unwrap();
    let accounts = store.get::<Vec<Account>>("accounts").await.unwrap();
    replica.close().await.unwrap();
    drop(store);
    let store = Store::open(dir.path().join("cache.sqlite")).unwrap();
    let mut replica = Replica::open(
        dir.path().join("history.sqlite"),
        binding,
        Journal::open(None).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        store.capture_profile_change().await.unwrap(),
        Some(pending.clone())
    );
    // A lost admission reply can be retried before its native acknowledgment.
    replica.admit_local(pending.clone()).await.unwrap();
    let receipt = replica.admit_local(pending).await.unwrap();
    store.acknowledge_profile_change(receipt).await.unwrap();
    assert_eq!(
        store.get::<Vec<Account>>("accounts").await.unwrap(),
        accounts
    );
    assert_eq!(
        store
            .query(MailQuery {
                account: Some(original.id),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        1
    );
    assert!(store.capture_profile_change().await.unwrap().is_none());
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_account_review_rejects_removed_remote_account_and_new_google_identity() {
    for google in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut replica, _, _) = changed(dir.path()).await;
        let review = reviews::prepare(&store, &replica, None)
            .await
            .unwrap()
            .reviews
            .remove(0);
        let choice = Choice::AddShared(review.versions()[0].operation);
        if google {
            store
                .update_preferences(|p| p.google_lifecycle.revision += 1)
                .await
                .unwrap();
        } else {
            edit_remote(
                &mut replica,
                vec![Change {
                    action: Action::AccountRemoved {
                        id: review.basis.shared,
                    },
                    extra: Default::default(),
                }],
            )
            .await;
            assert!(
                reviews::prepare(&store, &replica, None)
                    .await
                    .unwrap()
                    .reviews
                    .is_empty()
            );
        }
        assert!(
            reviews::accept(&store, &mut replica, review, choice)
                .await
                .is_err()
        );
        assert_eq!(
            store.get::<Vec<Account>>("accounts").await.unwrap().len(),
            1
        );
        assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 1);
        replica.close().await.unwrap();
    }
}
