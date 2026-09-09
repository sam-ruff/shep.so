use super::*;
use crate::profile_sync::enrollment::Origin;
use serde_json::json;
use shep_profile_core::{Action, Change, SettingKey, history};
use uuid::Uuid;

#[tokio::test]
async fn profile_login_opt_out_is_durable_before_enrollment_and_survives_reconnection() {
    let legacy: Options =
        serde_json::from_value(json!({"enabled":false,"accounts":true,"settings":true})).unwrap();
    assert!(legacy.discover_on_login);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.sqlite");
    let store = Store::open(&path).unwrap();
    connected(&store).await;
    let first = store.profile_enrollment().await.unwrap();
    assert!(first.empty_workspace && crate::profile_sync::onboarding::eligible(&first));
    let declined = store
        .change_profile_sync_options(enrollment::Changes {
            discover_on_login: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(declined.enrollment.revision > first.enrollment.revision);
    assert!(!crate::profile_sync::onboarding::eligible(&declined));
    drop(store);
    let reopened = Store::open(&path).unwrap();
    connected(&reopened).await;
    let saved = reopened.profile_enrollment().await.unwrap();
    assert!(!saved.enrollment.options.discover_on_login && saved.enrollment.selection.is_none());
    let accepted = reopened
        .change_profile_sync_options(enrollment::Changes {
            discover_on_login: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(crate::profile_sync::onboarding::eligible(&accepted));
    assert!(accepted.enrollment.selection.is_none());
}

#[tokio::test]
async fn profile_login_checks_local_settings_and_draft_intent_before_automatic_import() {
    let store = Store::memory().unwrap();
    connected(&store).await;
    assert!(store.profile_enrollment().await.unwrap().empty_workspace);
    store
        .update_preferences(|p| p.appearance = Appearance::Dark)
        .await
        .unwrap();
    assert!(!store.profile_enrollment().await.unwrap().empty_workspace);
    let accounts_only = store
        .change_profile_sync_options(enrollment::Changes {
            settings: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(accounts_only.empty_workspace);
    store.put("drafts_revision", 1_u64).await.unwrap();
    assert!(!store.profile_enrollment().await.unwrap().empty_workspace);
}

pub(super) async fn connected(store: &Store) {
    store
        .update_preferences(|p| {
            p.google_client_id = "fixture-client".into();
            p.google_connection_id = "drive:fixture-user".into();
            p.google_grant = GoogleGrant {
                id: "fixture-grant".into(),
                client_id: "fixture-client".into(),
                access: GoogleAccess {
                    known: true,
                    drive: true,
                    calendar_read: false,
                    calendar_write: false,
                },
            };
        })
        .await
        .unwrap();
}
pub(super) fn selection() -> Selection {
    Selection {
        binding: history::Binding {
            namespace: "so.shep.fixture".into(),
            principal: "drive:fixture-user".into(),
            profile: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        },
        name: "Personal".into(),
        origin: Origin::Create,
        ready: false,
    }
}
async fn begin(store: &Store) -> Snapshot {
    let reviewed = store.profile_enrollment().await.unwrap();
    store
        .begin_profile_enrollment(
            reviewed,
            selection(),
            Options {
                enabled: true,
                ..Default::default()
            },
        )
        .await
        .unwrap()
}
fn setting(key: SettingKey, value: serde_json::Value) -> Change {
    Change {
        action: Action::Setting { key, value },
        extra: Default::default(),
    }
}

#[tokio::test]
async fn profile_enrollment_keeps_original_pending_choice_across_restart_and_rejects_stale_reviews()
{
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.sqlite");
    let store = Store::open(&path).unwrap();
    connected(&store).await;
    let initial = store.profile_enrollment().await.unwrap();
    assert!(!initial.enrollment.options.enabled);
    let pending = begin(&store).await;
    assert!(!pending.enrollment.selection.as_ref().unwrap().ready);
    assert!(pending.enrollment.last_success.is_none());
    assert!(
        store
            .begin_profile_enrollment(
                initial,
                selection(),
                Options {
                    enabled: true,
                    ..Default::default()
                }
            )
            .await
            .is_err()
    );
    let same = store
        .begin_profile_enrollment(
            pending.clone(),
            pending.enrollment.selection.clone().unwrap(),
            pending.enrollment.options,
        )
        .await
        .unwrap();
    assert_eq!(same.enrollment, pending.enrollment);
    assert!(
        store
            .begin_profile_enrollment(pending.clone(), selection(), pending.enrollment.options)
            .await
            .is_err()
    );
    drop(store);
    let store = Store::open(path).unwrap();
    assert_eq!(
        store.profile_enrollment().await.unwrap().enrollment,
        pending.enrollment
    );
    let confirmed = store.profile_sync_succeeded(pending, 123).await.unwrap();
    assert!(confirmed.enrollment.selection.unwrap().ready);
    assert_eq!(confirmed.enrollment.last_success, Some(123));
}

#[tokio::test]
async fn profile_controls_and_google_disconnect_fence_late_results_without_network_or_credentials()
{
    let store = Store::memory().unwrap();
    connected(&store).await;
    let started = begin(&store).await;
    let disabled = store
        .set_profile_sync_options(
            started.enrollment.revision,
            Options {
                enabled: false,
                accounts: false,
                settings: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        store
            .profile_sync_succeeded(started.clone(), 123)
            .await
            .is_err()
    );
    assert!(
        store
            .set_profile_sync_options(started.enrollment.revision, started.enrollment.options)
            .await
            .is_err()
    );
    assert_eq!(
        store.profile_enrollment().await.unwrap().enrollment,
        disabled.enrollment
    );
    let enabled = store
        .set_profile_sync_options(disabled.enrollment.revision, started.enrollment.options)
        .await
        .unwrap();
    store
        .disconnect_google(enabled.google_revision)
        .await
        .unwrap();
    let paused = store.profile_enrollment().await.unwrap();
    assert!(!paused.available);
    assert!(!paused.enrollment.options.enabled);
    assert!(store.profile_sync_succeeded(enabled, 123).await.is_err());
    assert!(
        store
            .set_profile_sync_options(paused.enrollment.revision, started.enrollment.options)
            .await
            .is_err()
    );
    assert!(
        store
            .profile_enrollment()
            .await
            .unwrap()
            .enrollment
            .selection
            .is_some()
    );
    assert_eq!(store.get::<usize>("unrelated-test-key").await.unwrap(), 0);
}

#[tokio::test]
async fn invalid_profile_enrollment_cannot_prevent_google_disconnect_or_become_a_fresh_setup() {
    let store = Store::memory().unwrap();
    connected(&store).await;
    store
        .put(STORAGE_KEY, json!({"unsupported_future_shape":true}))
        .await
        .unwrap();
    assert!(store.profile_enrollment().await.is_err());
    store.disconnect_google(0).await.unwrap();
    assert!(
        store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .google_lifecycle
            .disconnected
    );
    assert_eq!(
        store.get::<serde_json::Value>(STORAGE_KEY).await.unwrap(),
        json!({"unsupported_future_shape":true})
    );
    assert!(store.profile_enrollment().await.is_err());
}

#[tokio::test]
async fn profile_settings_apply_atomically_preserve_device_fields_and_refuse_newer_local_intent() {
    let store = Store::memory().unwrap();
    connected(&store).await;
    store
        .update_preferences(|p| {
            p.reader_split = 0.6;
            p.backup_folder = "/fixture/device-only".into();
            p.google_client_secret = "fixture-secret-not-portable".into();
            p.contacts = vec!["local@example.test".into()];
        })
        .await
        .unwrap();
    let expected = begin(&store).await;
    let before: Preferences = store.get("preferences").await.unwrap();
    for changes in [
        vec![
            setting(SettingKey::Appearance, json!("Dark")),
            setting(SettingKey::UnifiedInbox, json!("not a bool")),
        ],
        vec![
            setting(SettingKey::Appearance, json!("Dark")),
            setting(SettingKey::Appearance, json!("Light")),
        ],
    ] {
        assert!(
            store
                .apply_profile_settings(expected.clone(), changes)
                .await
                .is_err()
        );
        assert_eq!(
            store.get::<Preferences>("preferences").await.unwrap(),
            before
        );
    }
    let (applied, preferences) = store
        .apply_profile_settings(
            expected,
            vec![
                setting(SettingKey::Appearance, json!("Dark")),
                setting(SettingKey::UnifiedInbox, json!(false)),
            ],
        )
        .await
        .unwrap();
    let mut wanted = before.clone();
    wanted.appearance = Appearance::Dark;
    wanted.unified_inbox = false;
    assert_eq!(preferences.value, wanted);
    let (same, _) = store
        .apply_profile_settings(
            applied.clone(),
            vec![setting(SettingKey::Appearance, json!("Dark"))],
        )
        .await
        .unwrap();
    assert_eq!(same.preferences_revision, applied.preferences_revision);
    store
        .update_preferences(|p| p.appearance = Appearance::Light)
        .await
        .unwrap();
    assert!(
        store
            .apply_profile_settings(
                applied,
                vec![setting(SettingKey::Appearance, json!("Dark"))]
            )
            .await
            .is_err()
    );
    assert_eq!(
        store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Light
    );
    let review = store.profile_enrollment().await.unwrap();
    let paused = store
        .set_profile_sync_options(
            review.enrollment.revision,
            Options {
                enabled: true,
                accounts: true,
                settings: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        store
            .apply_profile_settings(paused, vec![setting(SettingKey::Appearance, json!("Dark"))])
            .await
            .is_err()
    );
}

#[tokio::test]
async fn profile_metadata_maps_explicit_auth_and_keeps_unimplemented_settings_and_extensions() {
    use crate::profile_sync::metadata;
    let operation = shep_profile_core::Operation::decode(include_bytes!(
        "../../../tests/support/profile-operation.json"
    ))
    .unwrap();
    let Action::AccountConnection { account } = &operation.changes[0].action else {
        panic!()
    };
    let native = metadata::review_account(account, "Équipe").unwrap();
    let exported = metadata::export_account(&native, account.id).unwrap();
    assert_eq!(exported[0], operation.changes[0]);
    assert_eq!(exported[1], operation.changes[1]);
    assert!(metadata::export_account(&native, Uuid::new_v4()).is_err());
    let mut extended = account.clone();
    extended
        .extra
        .insert("future_connection".into(), json!("preserve"));
    assert!(metadata::review_account(&extended, "Équipe").is_err());
    let mut preferences = Preferences::default();
    assert!(metadata::apply_setting(&mut preferences, &operation.changes[2]).unwrap());
    assert_eq!(preferences.appearance, Appearance::Dark);
    assert!(!metadata::apply_setting(&mut preferences, &operation.changes[3]).unwrap());
    assert!(metadata::setting_value(SettingKey::PreviewLines, &preferences).is_none());
    let tooltip = Change {
        action: Action::Setting {
            key: SettingKey::Tooltips,
            value: json!(false),
        },
        extra: Default::default(),
    };
    assert!(metadata::apply_setting(&mut preferences, &tooltip).unwrap());
    assert!(!preferences.tooltips);
    assert!(
        preferences.shortcut_tooltips,
        "The independent shortcut hint choice stays local."
    );
    for (key, value) in [
        (SettingKey::LeftSwipe, json!("archive")),
        (SettingKey::RightSwipe, json!("read")),
        (SettingKey::SenderPictures, json!(false)),
    ] {
        assert!(
            !metadata::apply_setting(
                &mut preferences,
                &Change {
                    action: Action::Setting { key, value },
                    extra: Default::default()
                }
            )
            .unwrap()
        );
        assert!(metadata::setting_value(key, &preferences).is_none());
    }
}

#[tokio::test]
async fn profile_seed_freezes_legacy_account_mapping_and_chunk_retries_across_restart() {
    use crate::profile_sync::{
        enrollment::{SEED_KEY, Seed},
        metadata,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.sqlite");
    let store = Store::open(&path).unwrap();
    connected(&store).await;
    let operation = shep_profile_core::Operation::decode(include_bytes!(
        "../../../tests/support/profile-operation.json"
    ))
    .unwrap();
    let connection = operation
        .changes
        .iter()
        .find_map(|c| match &c.action {
            Action::AccountConnection { account } => Some(account),
            _ => None,
        })
        .unwrap();
    let mut accounts = vec![];
    for index in 0..70 {
        let mut account =
            metadata::review_account(connection, &format!("Account {index}")).unwrap();
        account.id = format!("legacy-{index}");
        accounts.push(account);
    }
    store.put("accounts", accounts).await.unwrap();
    let original = begin(&store).await;
    let seed = store.profile_seed(original.clone()).await.unwrap();
    assert!(seed.chunks.len() > 1);
    assert_eq!(seed.account_ids.len(), 70);
    assert!(
        store
            .checkpoint_profile_seed(original.clone(), seed.chunks[0].operation, u64::MAX)
            .await
            .is_err()
    );
    assert!(
        store.profile_seed(original.clone()).await.unwrap().chunks[0]
            .expected_revision
            .is_none()
    );
    let first = store
        .checkpoint_profile_seed(original.clone(), seed.chunks[0].operation, 0)
        .await
        .unwrap();
    let repeated = store
        .checkpoint_profile_seed(original.clone(), first.operation, 999)
        .await
        .unwrap();
    assert_eq!(repeated.expected_revision, Some(0));
    assert_eq!(
        serde_json::to_vec(&repeated.changes).unwrap(),
        serde_json::to_vec(&first.changes).unwrap()
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    let reopened = store.profile_seed(original.clone()).await.unwrap();
    assert_eq!(reopened.account_ids, seed.account_ids);
    assert_eq!(reopened.chunks[0].operation, first.operation);
    assert_eq!(reopened.chunks[0].expected_revision, Some(0));
    assert_eq!(
        store.get::<Vec<Account>>("accounts").await.unwrap()[0].id,
        "legacy-0"
    );

    let saved = store.get::<Option<Seed>>(SEED_KEY).await.unwrap().unwrap();
    for kind in 0..3 {
        let mut corrupt = saved.clone();
        match kind {
            0 => {
                corrupt.chunks[1].operation = corrupt.chunks[0].operation;
            }
            1 => {
                corrupt.account_ids.remove("legacy-0");
            }
            _ => {
                corrupt.chunks[0].changes[0]
                    .extra
                    .insert("password".into(), json!("forbidden"));
            }
        }
        store.put(SEED_KEY, corrupt).await.unwrap();
        assert!(store.profile_seed(original.clone()).await.is_err());
        assert!(
            store
                .checkpoint_profile_seed(original.clone(), first.operation, 1)
                .await
                .is_err()
        );
    }
    store.put(SEED_KEY, saved).await.unwrap();
    assert_eq!(
        store.profile_seed(original).await.unwrap().account_ids,
        seed.account_ids
    );
}

#[tokio::test]
async fn profile_field_choices_preserve_new_enrollment_and_disconnected_master_switch() {
    use crate::profile_sync::enrollment::Changes;
    let store = Store::memory().unwrap();
    connected(&store).await;
    let pending = begin(&store).await;
    // An earlier UI snapshot need not know setup has just enabled the profile.
    let edited = store
        .change_profile_sync_options(Changes {
            accounts: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(edited.enrollment.options.enabled);
    assert!(!edited.enrollment.options.accounts);
    assert_eq!(edited.enrollment.selection, pending.enrollment.selection);
    store
        .disconnect_google(edited.google_revision)
        .await
        .unwrap();
    let offline = store
        .change_profile_sync_options(Changes {
            accounts: Some(true),
            settings: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(!offline.enrollment.options.enabled);
    assert!(offline.enrollment.options.accounts);
    assert!(!offline.enrollment.options.settings);
    assert!(
        store
            .change_profile_sync_options(Changes {
                enabled: Some(true),
                ..Default::default()
            })
            .await
            .is_err()
    );
    assert_eq!(
        store.profile_enrollment().await.unwrap().enrollment,
        offline.enrollment
    );
}
