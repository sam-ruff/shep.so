use super::*;
use crate::profile_sync::account_reviews::{self as reviews, Choice};

fn shared_change() -> Change {
    shep_profile_core::Operation::decode(include_bytes!(
        "../../../../tests/support/profile-operation.json"
    ))
    .unwrap()
    .changes
    .remove(0)
}

fn local_account(exact: bool) -> Account {
    let Action::AccountConnection { account } = shared_change().action else {
        panic!("connection fixture")
    };
    let mut local = metadata::review_account(&account, "Studio on this device").unwrap();
    local.id = "local-studio".into();
    if !exact {
        local.host = "other-incoming.example.test".into();
    }
    local
}

/// A populated device joined without linking; a matching definition then
/// arrives through the continuous loop and must wait for an explicit choice.
async fn arrived(
    path: &std::path::Path,
    exact: bool,
) -> (Store, Replica, history::Binding, Account, Uuid) {
    let local = local_account(exact);
    {
        let store = Store::open(path.join("cache.sqlite")).unwrap();
        store.save_account(local.clone()).await.unwrap();
    }
    let (store, mut replica, binding, _) = fixture(path).await;
    let mail = parse_mail(
        &local.id,
        "kept-server-id",
        "INBOX",
        b"Subject: Kept mail\r\n\r\nStays with this device".to_vec(),
        true,
        false,
    )
    .unwrap();
    store.upsert(vec![mail]).await.unwrap();
    let change = shared_change();
    let Action::AccountConnection { account } = &change.action else {
        panic!("connection fixture")
    };
    let shared = account.id;
    edit_remote(
        &mut replica,
        vec![
            change,
            Change {
                action: Action::AccountName {
                    id: shared,
                    name: "Shared studio".into(),
                },
                extra: Default::default(),
            },
        ],
    )
    .await;
    assert_eq!(apply_observed(&store, &replica).await.review, 1);
    assert_eq!(
        store.get::<Vec<Account>>("accounts").await.unwrap(),
        vec![local.clone()]
    );
    let state = store.profile_replication(binding.clone()).await.unwrap();
    assert!(state.local_only.contains(&local.id));
    assert!(state.accounts.is_empty());
    (store, replica, binding, local, shared)
}

async fn review(store: &Store, replica: &Replica) -> reviews::Review {
    let page = reviews::prepare(store, replica, None).await.unwrap();
    assert!(page.after.is_none());
    assert_eq!(page.reviews.len(), 1);
    page.reviews.into_iter().next().unwrap()
}

#[tokio::test]
async fn profile_account_link_review_offers_link_only_for_exact_connection_matches() {
    for exact in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut replica, _, local, shared) = arrived(dir.path(), exact).await;
        let review = review(&store, &replica).await;
        let link = review.link().expect("link review");
        assert_eq!(review.shared(), shared);
        assert_eq!(link.account.name, "Shared studio");
        assert_eq!(link.account.email, local.email);
        assert_eq!(link.linkable(), exact);
        assert_eq!(link.matches.len(), 1);
        assert_eq!(link.matches[0].account, local);
        assert_eq!(link.matches[0].exact, exact);
        assert!(review.versions().is_empty() && !review.removed());
        let result = reviews::accept(
            &store,
            &mut replica,
            review,
            Choice::LinkExisting(local.id.clone()),
        )
        .await;
        assert_eq!(result.is_ok(), exact);
        let state = store
            .profile_replication(replica.binding().clone())
            .await
            .unwrap();
        assert_eq!(
            state.accounts.get(&local.id).copied(),
            exact.then_some(shared)
        );
        assert_eq!(
            store.get::<Vec<Account>>("accounts").await.unwrap(),
            vec![local]
        );
        replica.close().await.unwrap();
    }
}

#[tokio::test]
async fn profile_account_link_keeps_native_identity_and_mail_without_republishing_the_definition() {
    let dir = tempfile::tempdir().unwrap();
    let (store, replica, binding, local, shared) = arrived(dir.path(), true).await;
    let mut replica = replica;
    let review = review(&store, &replica).await;
    let remote = review.link().unwrap().change.clone();
    let before = replica.state().await.unwrap();
    reviews::accept(
        &store,
        &mut replica,
        review,
        Choice::LinkExisting(local.id.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        store.get::<Vec<Account>>("accounts").await.unwrap(),
        vec![local.clone()]
    );
    store
        .require_account_reconnected(local.id.clone())
        .await
        .unwrap();
    assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 1);
    assert_eq!(replica.state().await.unwrap().queued, before.queued);
    let state = store.profile_replication(binding.clone()).await.unwrap();
    assert_eq!(state.accounts[&local.id], shared);
    assert!(state.local_only.is_empty() && state.suppressed.is_empty());
    assert_eq!(
        state.fields[&reviews::target(shared)].remote.as_ref(),
        Some(&remote)
    );
    assert!(
        reviews::prepare(&store, &replica, None)
            .await
            .unwrap()
            .reviews
            .is_empty()
    );
    // Restart before the follow-up: the saved link survives and only this
    // device's account name is shared, never another connection definition.
    replica.close().await.unwrap();
    drop(store);
    let store = Store::open(dir.path().join("cache.sqlite")).unwrap();
    let mut replica = Replica::open(
        dir.path().join("history.sqlite"),
        binding.clone(),
        Journal::open(None).unwrap(),
    )
    .await
    .unwrap();
    let pending = store.capture_profile_change().await.unwrap().unwrap();
    assert!(matches!(
        &pending.change.action,
        Action::AccountName { id, name } if *id == shared && name == "Studio on this device"
    ));
    let receipt = replica.admit_local(pending).await.unwrap();
    store.acknowledge_profile_change(receipt).await.unwrap();
    assert!(store.capture_profile_change().await.unwrap().is_none());
    assert_eq!(apply_observed(&store, &replica).await.review, 0);
    assert_eq!(
        store.get::<Vec<Account>>("accounts").await.unwrap(),
        vec![local]
    );
    assert_eq!(
        replica
            .versions(reviews::target(shared), None)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 1);
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_account_link_add_new_creates_one_reconnecting_account_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, binding, local, shared) = arrived(dir.path(), true).await;
    let review = review(&store, &replica).await;
    reviews::accept(&store, &mut replica, review, Choice::AddNew)
        .await
        .unwrap();
    let accounts = store.get::<Vec<Account>>("accounts").await.unwrap();
    assert_eq!(accounts.len(), 2);
    let added = accounts.iter().find(|a| a.id != local.id).unwrap();
    assert_eq!(added.name, "Shared studio");
    assert_eq!(added.host, local.host);
    assert!(Uuid::parse_str(&added.id).is_ok());
    assert!(
        store
            .require_account_reconnected(added.id.clone())
            .await
            .is_err()
    );
    store
        .require_account_reconnected(local.id.clone())
        .await
        .unwrap();
    let state = store.profile_replication(binding.clone()).await.unwrap();
    assert_eq!(state.accounts[&added.id], shared);
    assert!(state.local_only.contains(&local.id));
    assert!(store.capture_profile_change().await.unwrap().is_none());
    assert_eq!(apply_observed(&store, &replica).await.review, 0);
    assert!(
        reviews::prepare(&store, &replica, None)
            .await
            .unwrap()
            .reviews
            .is_empty()
    );
    replica.close().await.unwrap();
    drop(store);
    let store = Store::open(dir.path().join("cache.sqlite")).unwrap();
    let replica = Replica::open(
        dir.path().join("history.sqlite"),
        binding,
        Journal::open(None).unwrap(),
    )
    .await
    .unwrap();
    assert!(store.capture_profile_change().await.unwrap().is_none());
    assert_eq!(apply_observed(&store, &replica).await.review, 0);
    assert_eq!(
        store.get::<Vec<Account>>("accounts").await.unwrap(),
        accounts
    );
    assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 1);
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_account_link_keep_local_suppresses_the_shared_identity_durably() {
    for exact in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut replica, binding, local, shared) = arrived(dir.path(), exact).await;
        let review = review(&store, &replica).await;
        let before = replica.state().await.unwrap();
        reviews::accept(&store, &mut replica, review, Choice::KeepLocal)
            .await
            .unwrap();
        assert_eq!(
            store.get::<Vec<Account>>("accounts").await.unwrap(),
            vec![local.clone()]
        );
        let state = store.profile_replication(binding.clone()).await.unwrap();
        assert!(state.suppressed.contains(&shared));
        assert!(state.accounts.is_empty());
        assert!(state.local_only.contains(&local.id));
        assert_eq!(replica.state().await.unwrap().queued, before.queued);
        assert!(store.capture_profile_change().await.unwrap().is_none());
        assert_eq!(apply_observed(&store, &replica).await.review, 0);
        assert!(
            reviews::prepare(&store, &replica, None)
                .await
                .unwrap()
                .reviews
                .is_empty()
        );
        let mut edited = local.clone();
        edited.sent_folder = "Edited later".into();
        store.save_account(edited.clone()).await.unwrap();
        assert!(store.capture_profile_change().await.unwrap().is_none());
        replica.close().await.unwrap();
        drop(store);
        let store = Store::open(dir.path().join("cache.sqlite")).unwrap();
        let replica = Replica::open(
            dir.path().join("history.sqlite"),
            binding.clone(),
            Journal::open(None).unwrap(),
        )
        .await
        .unwrap();
        assert!(
            store
                .profile_replication(binding)
                .await
                .unwrap()
                .suppressed
                .contains(&shared)
        );
        assert_eq!(apply_observed(&store, &replica).await.review, 0);
        assert_eq!(
            store.get::<Vec<Account>>("accounts").await.unwrap(),
            vec![edited]
        );
        assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 1);
        replica.close().await.unwrap();
    }
}

#[tokio::test]
async fn profile_account_link_rejects_stale_native_google_option_and_history_choices() {
    for (reason, choice) in [
        ("native", Choice::LinkExisting("local-studio".into())),
        ("google", Choice::AddNew),
        ("options", Choice::KeepLocal),
        ("history", Choice::LinkExisting("local-studio".into())),
        ("mapping", Choice::AddNew),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut replica, binding, mut local, shared) = arrived(dir.path(), true).await;
        let review = review(&store, &replica).await;
        match reason {
            "native" => {
                local.sent_folder = "Newer local folder".into();
                store.save_account(local.clone()).await.unwrap();
            }
            "google" => {
                store
                    .update_preferences(|p| p.google_lifecycle.revision += 1)
                    .await
                    .unwrap();
            }
            "options" => {
                let snapshot = store.profile_enrollment().await.unwrap();
                let mut options = snapshot.enrollment.options;
                options.accounts = false;
                store
                    .set_profile_sync_options(snapshot.enrollment.revision, options)
                    .await
                    .unwrap();
            }
            "history" => {
                let Action::AccountConnection { mut account } = shared_change().action else {
                    panic!("connection fixture")
                };
                account.host = "moved.example.test".into();
                edit_remote(
                    &mut replica,
                    vec![Change {
                        action: Action::AccountConnection { account },
                        extra: Default::default(),
                    }],
                )
                .await;
            }
            _ => {
                // Another window already decided; the shared identity is taken.
                reviews::accept(&store, &mut replica, review.clone(), Choice::KeepLocal)
                    .await
                    .unwrap();
            }
        }
        assert!(
            reviews::accept(&store, &mut replica, review, choice)
                .await
                .is_err(),
            "{reason}"
        );
        assert_eq!(
            store.get::<Vec<Account>>("accounts").await.unwrap(),
            vec![local.clone()],
            "{reason}"
        );
        let state = store.profile_replication(binding).await.unwrap();
        assert!(state.accounts.is_empty(), "{reason}");
        assert_eq!(state.suppressed.contains(&shared), reason == "mapping");
        assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 1);
        replica.close().await.unwrap();
    }
}

#[tokio::test]
async fn profile_account_link_pages_stay_bounded_after_mapped_reviews() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, _, _, _) = arrived(dir.path(), true).await;
    // Nine further unmapped definitions share this device's address; with the
    // first, ten link candidates must span two bounded pages.
    let Action::AccountConnection { account } = shared_change().action else {
        panic!("connection fixture")
    };
    let mut changes = Vec::new();
    for n in 2..=10u32 {
        let mut copy = account.clone();
        copy.id = Uuid::parse_str(&format!("50000000-0000-4000-8000-{n:012}")).unwrap();
        copy.host = format!("host{n}.example.test");
        changes.push(Change {
            action: Action::AccountConnection { account: copy },
            extra: Default::default(),
        });
    }
    edit_remote(&mut replica, changes).await;
    assert_eq!(apply_observed(&store, &replica).await.review, 10);
    let first = reviews::prepare(&store, &replica, None).await.unwrap();
    assert_eq!(first.reviews.len(), reviews::PAGE_SIZE);
    assert!(first.reviews.iter().all(|r| r.link().is_some()));
    let after = first.after.clone().expect("second page");
    let second = reviews::prepare(&store, &replica, Some(after))
        .await
        .unwrap();
    assert_eq!(second.reviews.len(), 2);
    assert!(second.after.is_none());
    let mut seen: Vec<_> = first
        .reviews
        .iter()
        .chain(second.reviews.iter())
        .map(|r| r.shared())
        .collect();
    seen.dedup();
    assert_eq!(seen.len(), 10);
    assert_eq!(
        first
            .reviews
            .iter()
            .chain(second.reviews.iter())
            .filter(|r| r.link().is_some_and(|l| l.linkable()))
            .count(),
        1
    );
    replica.close().await.unwrap();
}
