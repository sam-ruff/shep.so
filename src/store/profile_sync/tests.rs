use super::*;
use crate::model::Appearance;
use shep_profile_core::history::{Binding, Journal};
use std::collections::{BTreeMap, BTreeSet};

fn change(key: SettingKey, value: impl Into<serde_json::Value>) -> Change {
    Change {
        action: Action::Setting {
            key,
            value: value.into(),
        },
        extra: Default::default(),
    }
}
fn binding() -> Binding {
    Binding {
        namespace: "so.shep.fixture".into(),
        principal: "drive:fixture".into(),
        profile: Uuid::new_v4(),
        generation: Uuid::new_v4(),
    }
}
fn shared(b: &Binding) -> Journal {
    let mut j = Journal::memory(b.clone()).unwrap();
    for changes in [
        vec![Change {
            action: Action::ProfileSetup { complete: false },
            extra: Default::default(),
        }],
        vec![
            change(SettingKey::Appearance, "System"),
            change(SettingKey::Tooltips, true),
        ],
        vec![Change {
            action: Action::ProfileSetup { complete: true },
            extra: Default::default(),
        }],
    ] {
        j.edit(LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: j.state().unwrap().revision,
            changes,
            resolutions: vec![],
        })
        .unwrap();
    }
    j
}
async fn subscribe(db: &Store, b: &Binding, j: &Journal) -> Subscription {
    let seed = Seed {
        baseline: None,
        local_intent: Default::default(),
        binding: b.clone(),
        device: j.state().unwrap().device,
        name: "Work".into(),
        history_revision: j.state().unwrap().revision,
        fields: BTreeMap::from([
            (
                SettingKey::Appearance,
                Some(change(SettingKey::Appearance, "System")),
            ),
            (
                SettingKey::Tooltips,
                Some(change(SettingKey::Tooltips, true)),
            ),
        ]),
    };
    let s = db.profile_sync_seed(seed).await.unwrap();
    db.profile_sync_enable(b.storage_key().unwrap(), s.revision, true)
        .await
        .unwrap()
}
async fn appearance(db: &Store, value: Appearance) {
    let mut p: Preferences = db.get("preferences").await.unwrap();
    p.appearance = value;
    db.save_profile_preferences(p, BTreeSet::from([SettingKey::Appearance]))
        .await
        .unwrap();
}

#[tokio::test]
async fn exact_local_request_and_newer_edit_survive_lost_receipt_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("mail.sqlite");
    let b = binding();
    let key = b.storage_key().unwrap();
    let mut history = shared(&b);
    let db = Store::open(&path).unwrap();
    subscribe(&db, &b, &history).await;
    appearance(&db, Appearance::Dark).await;
    assert_eq!(
        db.profile_sync_subscription(key.clone())
            .await
            .unwrap()
            .pending,
        1
    );
    let first = db
        .profile_sync_prepare_edit(key.clone(), SettingKey::Appearance)
        .await
        .unwrap()
        .unwrap();
    let saved = history.edit(first.request.clone()).unwrap();
    appearance(&db, Appearance::Light).await;
    drop(db);
    let db = Store::open(&path).unwrap();
    let replay = db
        .profile_sync_prepare_edit(key.clone(), SettingKey::Appearance)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(&first).unwrap(),
        serde_json::to_value(&replay).unwrap()
    );
    assert_eq!(
        history.edit(replay.request.clone()).unwrap().operations,
        saved.operations
    );
    db.profile_sync_edit_saved(replay, saved.revision)
        .await
        .unwrap();
    let latest = db
        .profile_sync_prepare_edit(key.clone(), SettingKey::Appearance)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(latest.operation, first.operation);
    assert_eq!(
        latest.request.changes[0],
        change(SettingKey::Appearance, "Light")
    );
    assert!(
        db.profile_sync_edit_saved(first, saved.revision)
            .await
            .is_err()
    );
    let saved = history.edit(latest.request.clone()).unwrap();
    db.profile_sync_edit_saved(latest, saved.revision)
        .await
        .unwrap();
    assert_eq!(db.profile_sync_subscription(key).await.unwrap().pending, 0);
    assert_eq!(
        db.get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Light
    );
}

#[tokio::test]
async fn remote_application_is_atomic_and_does_not_echo_or_replace_device_settings() {
    let b = binding();
    let key = b.storage_key().unwrap();
    let history = shared(&b);
    let db = Store::memory().unwrap();
    let prefs = Preferences {
        google_connection_id: "drive:device-only".into(),
        backup_folder: "/fixture/device-only".into(),
        reader_font_size: 23,
        ..Default::default()
    };
    db.put("preferences", prefs).await.unwrap();
    subscribe(&db, &b, &history).await;
    let revision = history.state().unwrap().revision + 1;
    let ApplyResult::Applied(snapshot) = db
        .profile_sync_apply_setting(
            key.clone(),
            SettingKey::Appearance,
            change(SettingKey::Appearance, "Dark"),
            revision,
        )
        .await
        .unwrap()
    else {
        panic!("remote preference must apply")
    };
    assert_eq!(snapshot.value.appearance, Appearance::Dark);
    assert_eq!(snapshot.value.google_connection_id, "drive:device-only");
    assert_eq!(snapshot.value.backup_folder, "/fixture/device-only");
    assert_eq!(snapshot.value.reader_font_size, 23);
    assert!(
        db.profile_sync_prepare_edit(key.clone(), SettingKey::Appearance)
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        db.profile_sync_apply_setting(
            key,
            SettingKey::Appearance,
            change(SettingKey::Appearance, "System"),
            revision - 1
        )
        .await
        .unwrap(),
        ApplyResult::Unchanged
    ));
    assert_eq!(
        db.get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Dark
    );
}

#[tokio::test]
async fn remote_change_preserves_newer_reverted_local_intent_and_other_fields_continue() {
    let b = binding();
    let key = b.storage_key().unwrap();
    let history = shared(&b);
    let db = Store::memory().unwrap();
    subscribe(&db, &b, &history).await;
    appearance(&db, Appearance::Dark).await;
    appearance(&db, Appearance::System).await;
    let revision = history.state().unwrap().revision + 1;
    assert!(matches!(
        db.profile_sync_apply_setting(
            key.clone(),
            SettingKey::Appearance,
            change(SettingKey::Appearance, "Light"),
            revision
        )
        .await
        .unwrap(),
        ApplyResult::ReviewRequired
    ));
    assert_eq!(
        db.get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::System
    );
    let fields = db.profile_sync_fields(key.clone()).await.unwrap();
    let field = fields
        .iter()
        .find(|f| f.key == SettingKey::Appearance)
        .unwrap();
    assert_eq!(
        field.incoming,
        Some(change(SettingKey::Appearance, "Light"))
    );
    assert_eq!(field.local, serde_json::json!("System"));
    assert!(field.pending && field.error.is_some());
    assert!(matches!(
        db.profile_sync_apply_setting(
            key.clone(),
            SettingKey::Tooltips,
            change(SettingKey::Tooltips, false),
            revision
        )
        .await
        .unwrap(),
        ApplyResult::Applied(_)
    ));
    assert!(!db.get::<Preferences>("preferences").await.unwrap().tooltips);
    assert_eq!(
        db.profile_sync_subscription(key).await.unwrap().conflicts,
        1
    );
}

#[tokio::test]
async fn pause_keeps_pending_receipts_and_stale_toggles_cannot_resume() {
    let b = binding();
    let key = b.storage_key().unwrap();
    let mut history = shared(&b);
    let db = Store::memory().unwrap();
    let active = subscribe(&db, &b, &history).await;
    appearance(&db, Appearance::Dark).await;
    let edit = db
        .profile_sync_prepare_edit(key.clone(), SettingKey::Appearance)
        .await
        .unwrap()
        .unwrap();
    let paused = db
        .profile_sync_enable(key.clone(), active.revision, false)
        .await
        .unwrap();
    assert!(
        db.profile_sync_enable(key.clone(), active.revision, true)
            .await
            .is_err()
    );
    assert!(
        db.profile_sync_prepare_edit(key.clone(), SettingKey::Appearance)
            .await
            .is_err()
    );
    assert!(
        db.profile_sync_apply_setting(
            key.clone(),
            SettingKey::Tooltips,
            change(SettingKey::Tooltips, false),
            history.state().unwrap().revision + 1
        )
        .await
        .is_err()
    );
    // A step accepted before Pause may still acknowledge its durable receipt.
    let saved = history.edit(edit.request.clone()).unwrap();
    db.profile_sync_edit_saved(edit, saved.revision)
        .await
        .unwrap();
    assert!(
        !db.profile_sync_subscription(key.clone())
            .await
            .unwrap()
            .enabled
    );
    let enabled = db
        .profile_sync_enable(key.clone(), paused.revision, true)
        .await
        .unwrap();
    let disabled = db
        .profile_sync_field_enable(key.clone(), enabled.revision, SettingKey::Appearance, false)
        .await
        .unwrap();
    appearance(&db, Appearance::Light).await;
    assert!(
        db.profile_sync_prepare_edit(key.clone(), SettingKey::Appearance)
            .await
            .unwrap()
            .is_none()
    );
    db.profile_sync_field_enable(key.clone(), disabled.revision, SettingKey::Appearance, true)
        .await
        .unwrap();
    assert!(
        db.profile_sync_prepare_edit(key, SettingKey::Appearance)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn different_principals_cannot_share_the_active_subscription_or_pending_request() {
    let first = binding();
    let mut second = first.clone();
    second.principal = "drive:other-fixture".into();
    let history = shared(&first);
    let db = Store::memory().unwrap();
    subscribe(&db, &first, &history).await;
    let other = db
        .profile_sync_seed(Seed {
            baseline: None,
            local_intent: Default::default(),
            binding: second.clone(),
            device: Uuid::new_v4(),
            name: "Other".into(),
            history_revision: 0,
            fields: BTreeMap::from([(SettingKey::Appearance, None)]),
        })
        .await
        .unwrap();
    assert!(
        db.profile_sync_enable(second.storage_key().unwrap(), other.revision, true)
            .await
            .is_err()
    );
    appearance(&db, Appearance::Dark).await;
    let edit = db
        .profile_sync_prepare_edit(first.storage_key().unwrap(), SettingKey::Appearance)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edit.binding, first);
    assert!(
        !db.profile_sync_subscription(second.storage_key().unwrap())
            .await
            .unwrap()
            .enabled
    );
}

#[tokio::test]
async fn failed_remote_receipt_rolls_back_preference_value_and_revisions() {
    let b = binding();
    let key = b.storage_key().unwrap();
    let history = shared(&b);
    let db = Store::memory().unwrap();
    subscribe(&db, &b, &history).await;
    let before: Preferences = db.get("preferences").await.unwrap();
    let revision: u64 = db.get("preferences_revision").await.unwrap();
    db.run(|db| {db.execute_batch("CREATE TEMP TRIGGER failed_sync_receipt BEFORE UPDATE OF shared_revision ON profile_sync_fields BEGIN SELECT RAISE(FAIL,'synthetic failed sync receipt');END;")?;Ok(())}).await.unwrap();
    let apply_revision = history.state().unwrap().revision + 1;
    assert!(
        db.profile_sync_apply_setting(
            key.clone(),
            SettingKey::Appearance,
            change(SettingKey::Appearance, "Dark"),
            apply_revision
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("synthetic failed sync receipt")
    );
    assert_eq!(db.get::<Preferences>("preferences").await.unwrap(), before);
    assert_eq!(
        db.get::<u64>("preferences_revision").await.unwrap(),
        revision
    );
    assert_eq!(
        db.profile_sync_subscription(key.clone())
            .await
            .unwrap()
            .pending,
        0
    );
    db.run(|db| {
        db.execute_batch("DROP TRIGGER failed_sync_receipt")?;
        Ok(())
    })
    .await
    .unwrap();
    assert!(matches!(
        db.profile_sync_apply_setting(
            key,
            SettingKey::Appearance,
            change(SettingKey::Appearance, "Dark"),
            apply_revision
        )
        .await
        .unwrap(),
        ApplyResult::Applied(_)
    ));
}

#[tokio::test]
async fn pausing_a_profile_does_not_authorize_switching_its_workspace() {
    let first = binding();
    let history = shared(&first);
    let db = Store::memory().unwrap();
    let initial = subscribe(&db, &first, &history).await;
    db.profile_sync_enable(first.storage_key().unwrap(), initial.revision, false)
        .await
        .unwrap();
    let second = binding();
    let other = db
        .profile_sync_seed(Seed {
            baseline: None,
            local_intent: Default::default(),
            binding: second.clone(),
            device: Uuid::new_v4(),
            name: "Other setup".into(),
            history_revision: 0,
            fields: BTreeMap::from([(SettingKey::Appearance, None)]),
        })
        .await
        .unwrap();
    assert!(
        db.profile_sync_enable(second.storage_key().unwrap(), other.revision, true)
            .await
            .unwrap_err()
            .to_string()
            .contains("Review a profile switch")
    );
    assert!(db.profile_sync_active().await.unwrap().is_none());
    let paused = db
        .profile_sync_subscription(first.storage_key().unwrap())
        .await
        .unwrap();
    db.profile_sync_enable(first.storage_key().unwrap(), paused.revision, true)
        .await
        .unwrap();
}

#[tokio::test]
async fn reviewed_setup_captures_reverted_intent_atomically_and_preserves_saved_choices() {
    let db = Store::memory().unwrap();
    let b = binding();
    let history = shared(&b);
    let baseline = db
        .run(|db| Ok(profile_preferences::state(db)?.revisions))
        .await
        .unwrap();
    appearance(&db, Appearance::Dark).await;
    appearance(&db, Appearance::System).await;
    let seed = Seed {
        binding: b.clone(),
        device: history.state().unwrap().device,
        name: "Reviewed".into(),
        history_revision: history.state().unwrap().revision,
        baseline: Some(baseline),
        local_intent: Default::default(),
        fields: BTreeMap::from([(
            SettingKey::Appearance,
            Some(change(SettingKey::Appearance, "System")),
        )]),
    };
    let subscription = db.profile_sync_seed(seed.clone()).await.unwrap();
    assert!(!subscription.enabled);
    assert_eq!(subscription.pending, 1);
    let key = b.storage_key().unwrap();
    let enabled = db
        .profile_sync_enable(key.clone(), subscription.revision, true)
        .await
        .unwrap();
    let queued = db
        .profile_sync_prepare_edit(key.clone(), SettingKey::Appearance)
        .await
        .unwrap()
        .unwrap();
    let reseeded = db.profile_sync_seed(seed).await.unwrap();
    assert_eq!(reseeded.revision, enabled.revision);
    assert!(reseeded.enabled);
    assert_eq!(
        db.profile_sync_prepare_edit(key, SettingKey::Appearance)
            .await
            .unwrap()
            .unwrap()
            .operation,
        queued.operation
    );
}
