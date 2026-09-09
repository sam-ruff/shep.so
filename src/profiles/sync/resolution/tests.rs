use super::*;
use crate::{
    model::{Appearance, Preferences},
    profiles::{
        fixture::{Fixture, NAMESPACE},
        sync::runner::tests::enrolled,
    },
};
use shep_profile_core::{Action, Operation, history::LocalEdit};
use std::{collections::BTreeSet, time::Duration};

async fn setup() -> (Fixture, tempfile::TempDir, Store, Subscription) {
    let fixture = Fixture::start(1, false, Duration::ZERO).await.unwrap();
    let drive = fixture
        .connect(NAMESPACE.into(), "drive:fixture")
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let (store, sub) = enrolled(root.path(), &drive).await;
    (fixture, root, store, sub)
}
fn change(value: &str) -> Change {
    Change {
        action: Action::Setting {
            key: SettingKey::Appearance,
            value: value.into(),
        },
        extra: Default::default(),
    }
}
async fn local(store: &Store, value: Appearance) {
    let mut prefs: Preferences = store.get("preferences").await.unwrap();
    prefs.appearance = value;
    store
        .save_profile_preferences(prefs, BTreeSet::from([SettingKey::Appearance]))
        .await
        .unwrap();
}
async fn concurrent(root: &Path, sub: &Subscription, count: usize) {
    let h = history(&root.join("discovery"), sub).await.unwrap();
    for n in 0..count {
        let record = Operation {
            format: shep_profile_core::FORMAT.into(),
            major: 1,
            minor: 0,
            requires: vec![
                "causal-v1".into(),
                "settings-v1".into(),
                "accounts-v1".into(),
                "initialization-v1".into(),
            ],
            namespace: sub.binding.namespace.clone(),
            profile: sub.binding.profile,
            generation: sub.binding.generation,
            device: Uuid::from_u128(800000 + n as u128),
            operation: Uuid::from_u128(900000 + n as u128),
            parents: vec![Uuid::from_u128(10002)],
            changes: vec![change(if n % 2 == 0 { "Light" } else { "System" })],
            extra: Default::default(),
        };
        h.request(HistoryCommand::Import {
            record: String::from_utf8(record.encode().unwrap()).unwrap(),
        })
        .await
        .unwrap();
    }
    h.close().await.unwrap();
}
async fn appearance(store: &Store) -> Appearance {
    store
        .get::<Preferences>("preferences")
        .await
        .unwrap()
        .appearance
}

#[tokio::test]
async fn paged_profile_conflict_review_requires_every_version_and_survives_reopen() {
    let (_fixture, root, store, sub) = setup().await;
    let profile = sub.binding.storage_key().unwrap();
    concurrent(root.path(), &sub, 51).await;
    let first = begin(
        &store,
        &root.path().join("discovery"),
        profile.clone(),
        SettingKey::Appearance,
    )
    .await
    .unwrap();
    let review = first.review.unwrap();
    assert_eq!(
        (review.total, review.seen, first.versions.len(), first.more),
        (51, 50, 50, true)
    );
    assert!(
        save(
            &store,
            &root.path().join("discovery"),
            profile.clone(),
            review.id,
            Some(Choice::Local)
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("every version")
    );
    drop(store);
    let store = Store::open(root.path().join("mail.sqlite")).unwrap();
    assert!(store.profile_sync_review_pending().await.unwrap());
    let current = store
        .profile_resolution_current(profile.clone())
        .await
        .unwrap();
    assert_eq!(current.review.unwrap().id, review.id);
    let last = store
        .profile_resolution_page(
            profile.clone(),
            review.id,
            first.versions.last().map(|v| v.operation),
        )
        .await
        .unwrap();
    assert_eq!(last.versions.len(), 1);
    assert!(!last.more);
    assert_eq!(last.review.unwrap().seen, 51);
    let selected = last.versions[0].operation;
    let snapshot = save(
        &store,
        &root.path().join("discovery"),
        profile.clone(),
        review.id,
        Some(Choice::Version(selected)),
    )
    .await
    .unwrap();
    assert_eq!(snapshot.value.appearance, Appearance::Light);
    assert!(!store.profile_sync_review_pending().await.unwrap());
    let h = history(
        &root.path().join("discovery"),
        &store
            .profile_sync_subscription(profile.clone())
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    let Reply::State(state) = h.request(HistoryCommand::State).await.unwrap() else {
        panic!()
    };
    assert_eq!((state.conflicts, state.queued), (0, 1));
    h.close().await.unwrap();
    // Lost result/duplicate retry acknowledges the same operation only.
    save(
        &store,
        &root.path().join("discovery"),
        profile.clone(),
        review.id,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        store
            .profile_sync_subscription(profile)
            .await
            .unwrap()
            .pending,
        0
    );
}

#[tokio::test]
async fn changed_profile_reviews_cannot_replace_local_or_later_shared_intent() {
    let (_fixture, root, store, sub) = setup().await;
    let profile = sub.binding.storage_key().unwrap();
    concurrent(root.path(), &sub, 2).await;
    let review = begin(
        &store,
        &root.path().join("discovery"),
        profile.clone(),
        SettingKey::Appearance,
    )
    .await
    .unwrap()
    .review
    .unwrap();
    local(&store, Appearance::Light).await;
    local(&store, Appearance::Dark).await;
    assert!(
        save(
            &store,
            &root.path().join("discovery"),
            profile.clone(),
            review.id,
            Some(Choice::Local)
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("changed after")
    );
    let next = begin(
        &store,
        &root.path().join("discovery"),
        profile.clone(),
        SettingKey::Appearance,
    )
    .await
    .unwrap()
    .review
    .unwrap();
    // A different history owner completes a newer resolution while review is open.
    let h = history(&root.path().join("discovery"), &sub).await.unwrap();
    h.request(HistoryCommand::Edit {
        edit: LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: next.history_revision,
            changes: vec![change("System")],
            resolutions: vec![shep_profile_core::history::Resolution {
                target: "setting:appearance".into(),
                versions: vec![Uuid::from_u128(900000), Uuid::from_u128(900001)],
            }],
        },
    })
    .await
    .unwrap();
    h.close().await.unwrap();
    assert!(
        save(
            &store,
            &root.path().join("discovery"),
            profile.clone(),
            next.id,
            Some(Choice::Local)
        )
        .await
        .is_err()
    );
    let page = store
        .profile_resolution_page(profile.clone(), next.id, None)
        .await
        .unwrap();
    assert_eq!(page.review.unwrap().phase, "stale");
    assert_eq!(appearance(&store).await, Appearance::Dark);
    assert!(!store.profile_sync_review_pending().await.unwrap());
}

#[tokio::test]
async fn staged_profile_decision_retains_exact_request_and_newer_reverted_local_edit_after_lost_receipt()
 {
    let (_fixture, root, store, sub) = setup().await;
    let profile = sub.binding.storage_key().unwrap();
    concurrent(root.path(), &sub, 2).await;
    let page = begin(
        &store,
        &root.path().join("discovery"),
        profile.clone(),
        SettingKey::Appearance,
    )
    .await
    .unwrap();
    let review = page.review.unwrap();
    let staged = store
        .profile_resolution_stage(
            profile.clone(),
            review.id,
            Some(Choice::Version(page.versions[0].operation)),
        )
        .await
        .unwrap();
    let exact = staged.request.unwrap();
    let h = history(&root.path().join("discovery"), &sub).await.unwrap();
    h.request(HistoryCommand::Edit {
        edit: exact.request.clone(),
    })
    .await
    .unwrap();
    h.close().await.unwrap();
    assert!(
        store
            .profile_resolution_cancel(profile.clone(), review.id)
            .await
            .is_err()
    );
    assert!(
        begin(
            &store,
            &root.path().join("discovery"),
            profile.clone(),
            SettingKey::Appearance
        )
        .await
        .is_err()
    );
    local(&store, Appearance::System).await;
    local(&store, Appearance::Dark).await;
    drop(store);
    let store = Store::open(root.path().join("mail.sqlite")).unwrap();
    let resumed = store
        .profile_resolution_current(profile.clone())
        .await
        .unwrap()
        .review
        .unwrap();
    assert_eq!(resumed.phase, "staged");
    assert_eq!(
        serde_json::to_value(resumed.request).unwrap(),
        serde_json::to_value(Some(exact)).unwrap()
    );
    let snapshot = save(
        &store,
        &root.path().join("discovery"),
        profile.clone(),
        review.id,
        None,
    )
    .await
    .unwrap();
    assert_eq!(snapshot.value.appearance, Appearance::Dark);
    assert_eq!(
        store
            .profile_sync_subscription(profile.clone())
            .await
            .unwrap()
            .pending,
        1
    );
    // The newer edit is captured against the accepted resolution, not erased.
    let edit = store
        .profile_sync_prepare_edit(profile, SettingKey::Appearance)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edit.request.changes, vec![change("Dark")]);
    let h = history(&root.path().join("discovery"), &sub).await.unwrap();
    h.request(HistoryCommand::Edit { edit: edit.request })
        .await
        .unwrap();
    h.close().await.unwrap();
}

#[tokio::test]
async fn pending_profile_edit_is_replayed_before_its_definitive_rejection_can_be_retired() {
    let (_fixture, root, store, sub) = setup().await;
    let profile = sub.binding.storage_key().unwrap();
    local(&store, Appearance::Light).await;
    let pending = store
        .profile_sync_prepare_edit(profile.clone(), SettingKey::Appearance)
        .await
        .unwrap()
        .unwrap();
    concurrent(root.path(), &sub, 2).await;
    begin(
        &store,
        &root.path().join("discovery"),
        profile.clone(),
        SettingKey::Appearance,
    )
    .await
    .unwrap();
    assert!(
        store
            .profile_sync_pending_edit(profile.clone(), SettingKey::Appearance)
            .await
            .unwrap()
            .is_none()
    );
    let operation = pending.operation.to_string();
    let rejected: String = store
        .run(move |db| {
            Ok(db.query_row(
                "SELECT request FROM profile_sync_rejected_edits WHERE operation=?",
                [operation],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(rejected, serde_json::to_string(&pending).unwrap());
    let current = store
        .profile_resolution_current(profile.clone())
        .await
        .unwrap()
        .review
        .unwrap();
    assert_eq!(current.local, serde_json::json!("Light"));
    save(
        &store,
        &root.path().join("discovery"),
        profile,
        current.id,
        Some(Choice::Local),
    )
    .await
    .unwrap();
    assert_eq!(appearance(&store).await, Appearance::Light);
}

#[tokio::test]
async fn accepted_profile_edit_with_lost_receipt_is_not_discarded_when_review_opens() {
    let (_fixture, root, store, sub) = setup().await;
    let profile = sub.binding.storage_key().unwrap();
    local(&store, Appearance::Light).await;
    let pending = store
        .profile_sync_prepare_edit(profile.clone(), SettingKey::Appearance)
        .await
        .unwrap()
        .unwrap();
    let h = history(&root.path().join("discovery"), &sub).await.unwrap();
    h.request(HistoryCommand::Edit {
        edit: pending.request.clone(),
    })
    .await
    .unwrap();
    h.close().await.unwrap();
    concurrent(root.path(), &sub, 2).await;
    let page = begin(
        &store,
        &root.path().join("discovery"),
        profile.clone(),
        SettingKey::Appearance,
    )
    .await
    .unwrap();
    assert_eq!(page.review.unwrap().total, 3);
    assert!(
        page.versions
            .iter()
            .any(|v| v.operation == pending.operation)
    );
    assert_eq!(
        store
            .run(|db| Ok(db.query_row(
                "SELECT count(*) FROM profile_sync_rejected_edits",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        0
    );
    assert_eq!(appearance(&store).await, Appearance::Light);
}

#[tokio::test]
async fn profile_decision_receipt_failure_rolls_back_preferences_and_retries_only_the_saved_operation()
 {
    let (_fixture, root, store, sub) = setup().await;
    let profile = sub.binding.storage_key().unwrap();
    concurrent(root.path(), &sub, 2).await;
    let page = begin(
        &store,
        &root.path().join("discovery"),
        profile.clone(),
        SettingKey::Appearance,
    )
    .await
    .unwrap();
    let id = page.review.unwrap().id;
    store.run(|db|{db.execute_batch("CREATE TEMP TRIGGER fail_decision BEFORE UPDATE OF phase ON profile_sync_reviews WHEN NEW.phase='complete' BEGIN SELECT RAISE(FAIL,'synthetic receipt failure');END;")?;Ok(())}).await.unwrap();
    assert!(
        save(
            &store,
            &root.path().join("discovery"),
            profile.clone(),
            id,
            Some(Choice::Version(page.versions[0].operation))
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("synthetic receipt failure")
    );
    assert_eq!(appearance(&store).await, Appearance::Dark);
    let staged = store
        .profile_resolution_current(profile.clone())
        .await
        .unwrap()
        .review
        .unwrap();
    assert_eq!(staged.phase, "staged");
    assert!(store.profile_sync_review_pending().await.unwrap());
    store
        .run(|db| {
            db.execute_batch("DROP TRIGGER fail_decision")?;
            Ok(())
        })
        .await
        .unwrap();
    let next = save(
        &store,
        &root.path().join("discovery"),
        profile.clone(),
        id,
        None,
    )
    .await
    .unwrap();
    assert_eq!(next.value.appearance, Appearance::Light);
    let complete = store
        .profile_resolution_page(profile, id, None)
        .await
        .unwrap()
        .review
        .unwrap();
    assert_eq!(
        serde_json::to_value(staged.request).unwrap(),
        serde_json::to_value(complete.request).unwrap()
    );
    let h = history(&root.path().join("discovery"), &sub).await.unwrap();
    let Reply::State(state) = h.request(HistoryCommand::State).await.unwrap() else {
        panic!()
    };
    assert_eq!(state.queued, 1);
    h.close().await.unwrap();
}
