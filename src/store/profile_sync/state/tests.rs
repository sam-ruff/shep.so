use super::*;
use crate::profile_sync::{journal::Journal, metadata, replica::Replica, state::Field};
use serde_json::json;
use shep_profile_core::{
    Action, Change, SettingKey,
    history::{Command, Worker},
};

fn appearance(value: &str) -> Change {
    Change {
        action: Action::Setting {
            key: SettingKey::Appearance,
            value: json!(value),
        },
        extra: Default::default(),
    }
}
async fn fixture(path: &std::path::Path) -> (Store, Replica, history::Binding, Uuid) {
    let store = Store::open(path.join("cache.sqlite")).unwrap();
    super::super::tests::connected(&store).await;
    store
        .update_preferences(|p| p.appearance = Appearance::Light)
        .await
        .unwrap();
    let selection = super::super::tests::selection();
    let binding = selection.binding.clone();
    let pending = store
        .begin_profile_enrollment(
            store.profile_enrollment().await.unwrap(),
            selection,
            Options {
                enabled: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let ready = store.profile_sync_succeeded(pending, 0).await.unwrap();
    let mut worker = Replica::open(
        path.join("history.sqlite"),
        binding.clone(),
        Journal::open(None).unwrap(),
    )
    .await
    .unwrap();
    let initial = Uuid::new_v4();
    worker
        .edit(history::LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: 0,
            changes: vec![Change {
                action: Action::ProfileSetup { complete: false },
                extra: Default::default(),
            }],
            resolutions: vec![],
        })
        .await
        .unwrap();
    let mut common = appearance("Light");
    common
        .extra
        .insert("future_hint".into(), json!({"keep":"opaque"}));
    let state = worker
        .edit(history::LocalEdit {
            operation: initial,
            expected_revision: worker.state().await.unwrap().revision,
            changes: vec![common.clone()],
            resolutions: vec![],
        })
        .await
        .unwrap();
    let state = worker
        .edit(history::LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: state.revision,
            changes: vec![Change {
                action: Action::ProfileSetup { complete: true },
                extra: Default::default(),
            }],
            resolutions: vec![],
        })
        .await
        .unwrap();
    store
        .initialize_profile_replication(ready, state.revision, Default::default(), vec![common])
        .await
        .unwrap();
    (store, worker, binding, initial)
}

async fn apply_observed(
    store: &Store,
    replica: &Replica,
) -> crate::profile_sync::continuous::Report {
    store
        .apply_profile_observation(replica.observe(None).await.unwrap())
        .await
        .unwrap()
}
async fn edit_remote(replica: &mut Replica, changes: Vec<Change>) {
    replica
        .edit(history::LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: replica.state().await.unwrap().revision,
            changes,
            resolutions: vec![],
        })
        .await
        .unwrap();
}

async fn remote_appearance(replica: &mut Replica, value: &str) {
    let mut change = appearance(value);
    // The peer must preserve the optional extension introduced by fixture().
    change
        .extra
        .insert("future_hint".into(), json!({"keep":"opaque"}));
    edit_remote(replica, vec![change]).await;
}

#[tokio::test]
async fn profile_reverted_native_setting_survives_pull_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, binding, _) = fixture(dir.path()).await;
    remote_appearance(&mut replica, "Dark").await;
    let observation = replica.observe(None).await.unwrap();
    store
        .update_preferences(|p| p.appearance = Appearance::Dark)
        .await
        .unwrap();
    store
        .update_preferences(|p| p.appearance = Appearance::Light)
        .await
        .unwrap();
    let report = store.apply_profile_observation(observation).await.unwrap();
    assert_eq!(report.review, 1);
    assert_eq!(
        store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Light
    );
    let pending = store.capture_profile_change().await.unwrap().unwrap();
    assert_eq!(pending.local, appearance("Light"));
    assert!(pending.native_revision > 0);
    // The prior common value is still an explicit edit against its old basis.
    assert!(replica.admit_local(pending.clone()).await.is_err());
    store
        .defer_profile_change(binding.clone(), pending.clone())
        .await
        .unwrap();
    drop(store);
    let reopened = Store::open(dir.path().join("cache.sqlite")).unwrap();
    assert_eq!(
        reopened.capture_profile_change().await.unwrap(),
        Some(pending)
    );
    assert_eq!(
        reopened
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Light
    );
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_acknowledgment_retains_newer_reversion_generation() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, _, _) = fixture(dir.path()).await;
    store
        .update_preferences(|p| p.appearance = Appearance::Dark)
        .await
        .unwrap();
    let captured = store.capture_profile_change().await.unwrap().unwrap();
    let admitted = replica.admit_local(captured.clone()).await.unwrap();
    // These native actions happen after admission but before its store receipt.
    store
        .update_preferences(|p| p.appearance = Appearance::Light)
        .await
        .unwrap();
    store
        .update_preferences(|p| p.appearance = Appearance::Dark)
        .await
        .unwrap();
    store.acknowledge_profile_change(admitted).await.unwrap();
    remote_appearance(&mut replica, "Light").await;
    let report = apply_observed(&store, &replica).await;
    assert_eq!(report.review, 1);
    assert_eq!(
        store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Dark
    );
    let latest = store.capture_profile_change().await.unwrap().unwrap();
    assert_eq!(latest.local, captured.local);
    assert!(latest.native_revision > captured.native_revision);
    assert_ne!(latest.operation, captured.operation);
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_matching_remote_reversion_converges_without_echo() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, _, _) = fixture(dir.path()).await;
    store
        .update_preferences(|p| p.appearance = Appearance::Dark)
        .await
        .unwrap();
    store
        .update_preferences(|p| p.appearance = Appearance::Light)
        .await
        .unwrap();
    remote_appearance(&mut replica, "Light").await;
    assert_eq!(apply_observed(&store, &replica).await.review, 0);
    assert!(store.capture_profile_change().await.unwrap().is_none());
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_native_account_name_reversion_survives_remote_rename() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, _, _) = fixture(dir.path()).await;
    let operation = shep_profile_core::Operation::decode(include_bytes!(
        "../../../../tests/support/profile-operation.json"
    ))
    .unwrap();
    let Action::AccountConnection { account: wire } = &operation.changes[0].action else {
        panic!("account");
    };
    edit_remote(
        &mut replica,
        vec![
            operation.changes[0].clone(),
            Change {
                action: Action::AccountName {
                    id: wire.id,
                    name: "Original".into(),
                },
                extra: Default::default(),
            },
        ],
    )
    .await;
    apply_observed(&store, &replica).await;
    let mut account = store
        .get::<Vec<Account>>("accounts")
        .await
        .unwrap()
        .remove(0);
    assert_eq!(account.name, "Original");
    account.name = "Temporary".into();
    store.save_account(account.clone()).await.unwrap();
    account.name = "Original".into();
    store.save_account(account).await.unwrap();
    edit_remote(
        &mut replica,
        vec![Change {
            action: Action::AccountName {
                id: wire.id,
                name: "Remote".into(),
            },
            extra: Default::default(),
        }],
    )
    .await;
    assert_eq!(apply_observed(&store, &replica).await.review, 1);
    assert_eq!(
        store.get::<Vec<Account>>("accounts").await.unwrap()[0].name,
        "Original"
    );
    let pending = store.capture_profile_change().await.unwrap().unwrap();
    assert!(matches!(pending.local.action, Action::AccountName { name, .. } if name == "Original"));
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_remote_application_preserves_racing_local_intent_and_applies_other_fields() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, binding, _) = fixture(dir.path()).await;
    let mut changed = appearance("Dark");
    changed
        .extra
        .insert("future_hint".into(), json!({"keep":"opaque"}));
    let original: Preferences = store.get("preferences").await.unwrap();
    edit_remote(
        &mut replica,
        vec![
            changed,
            Change {
                action: Action::Setting {
                    key: SettingKey::UnifiedInbox,
                    value: json!(!original.unified_inbox),
                },
                extra: Default::default(),
            },
        ],
    )
    .await;
    let observation = replica.observe(None).await.unwrap();
    // The edit occurs after the verified pull and before its cache transaction.
    store
        .update_preferences(|p| p.appearance = Appearance::System)
        .await
        .unwrap();
    let result = store.apply_profile_observation(observation).await.unwrap();
    assert_eq!((result.applied, result.review), (1, 1));
    let prefs: Preferences = store.get("preferences").await.unwrap();
    assert_eq!(prefs.appearance, Appearance::System);
    assert_eq!(prefs.unified_inbox, !original.unified_inbox);
    let pending = store.capture_profile_change().await.unwrap().unwrap();
    assert!(replica.admit_local(pending.clone()).await.is_err());
    store
        .defer_profile_change(binding.clone(), pending.clone())
        .await
        .unwrap();
    store
        .update_preferences(|p| p.tooltips = !p.tooltips)
        .await
        .unwrap();
    let excluded = std::collections::BTreeSet::from([pending.target()]);
    let other = store
        .capture_profile_change_except(excluded)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(other.target(), pending.target());
    store
        .acknowledge_profile_change(replica.admit_local(other).await.unwrap())
        .await
        .unwrap();
    drop(store);
    let store = Store::open(dir.path().join("cache.sqlite")).unwrap();
    assert_eq!(store.capture_profile_change().await.unwrap(), Some(pending));
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_remote_settings_recheck_category_and_opt_out_before_commit() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, _, _) = fixture(dir.path()).await;
    let mut changed = appearance("Dark");
    changed
        .extra
        .insert("future_hint".into(), json!({"keep":"opaque"}));
    edit_remote(&mut replica, vec![changed]).await;
    for changes in [
        enrollment::Changes {
            settings: Some(false),
            ..Default::default()
        },
        enrollment::Changes {
            enabled: Some(false),
            ..Default::default()
        },
    ] {
        let observation = replica.observe(None).await.unwrap();
        store.change_profile_sync_options(changes).await.unwrap();
        assert_eq!(
            store
                .apply_profile_observation(observation)
                .await
                .unwrap()
                .applied,
            0
        );
        assert_eq!(
            store
                .get::<Preferences>("preferences")
                .await
                .unwrap()
                .appearance,
            Appearance::Light
        );
    }
    store
        .change_profile_sync_options(enrollment::Changes {
            enabled: Some(true),
            settings: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(apply_observed(&store, &replica).await.applied, 1);
    assert_eq!(apply_observed(&store, &replica).await.applied, 0);
    assert!(store.capture_profile_change().await.unwrap().is_none());
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_remote_account_is_staged_once_and_changed_endpoint_never_reuses_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, binding, _) = fixture(dir.path()).await;
    let operation = shep_profile_core::Operation::decode(include_bytes!(
        "../../../../tests/support/profile-operation.json"
    ))
    .unwrap();
    let Action::AccountConnection {
        account: mut connection,
    } = operation.changes[0].action.clone()
    else {
        panic!("account")
    };
    let shared = connection.id;
    edit_remote(
        &mut replica,
        vec![
            Change {
                action: Action::AccountConnection {
                    account: connection.clone(),
                },
                extra: Default::default(),
            },
            Change {
                action: Action::AccountName {
                    id: shared,
                    name: "Shared mailbox".into(),
                },
                extra: Default::default(),
            },
        ],
    )
    .await;
    assert_eq!(apply_observed(&store, &replica).await.applied, 2);
    let accounts: Vec<Account> = store.get("accounts").await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_ne!(accounts[0].id, shared.to_string());
    assert_eq!(accounts[0].name, "Shared mailbox");
    assert!(
        store
            .require_account_reconnected(accounts[0].id.clone())
            .await
            .is_err()
    );
    assert_eq!(apply_observed(&store, &replica).await.applied, 0);
    assert!(store.capture_profile_change().await.unwrap().is_none());
    connection.host = "changed.example.com".into();
    edit_remote(
        &mut replica,
        vec![Change {
            action: Action::AccountConnection {
                account: connection,
            },
            extra: Default::default(),
        }],
    )
    .await;
    let report = apply_observed(&store, &replica).await;
    assert_eq!((report.applied, report.review), (0, 1));
    assert_eq!(
        store.get::<Vec<Account>>("accounts").await.unwrap(),
        accounts
    );
    let checkpoint = store.profile_replication(binding).await.unwrap();
    assert_eq!(checkpoint.accounts.len(), 1);
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_edits_keep_exact_request_across_restart_and_newer_local_changes_before_ack() {
    let directory = tempfile::tempdir().unwrap();
    let (store, mut worker, binding, _) = fixture(directory.path()).await;
    assert!(store.capture_profile_change().await.unwrap().is_none());
    store
        .update_preferences(|p| p.appearance = Appearance::Dark)
        .await
        .unwrap();
    let first = store.capture_profile_change().await.unwrap().unwrap();
    assert_eq!(first.change.extra["future_hint"], json!({"keep":"opaque"}));
    let applied = worker.edit(first.edit()).await.unwrap();
    store
        .update_preferences(|p| p.appearance = Appearance::System)
        .await
        .unwrap();
    drop(store);
    let store = Store::open(directory.path().join("cache.sqlite")).unwrap();
    assert_eq!(
        store.capture_profile_change().await.unwrap(),
        Some(first.clone())
    );
    // Replaying the lost history acknowledgment also verifies its current field.
    let admitted = worker.admit_local(first.clone()).await.unwrap();
    assert_eq!(worker.state().await.unwrap().operations, applied.operations);
    store
        .acknowledge_profile_change(admitted.clone())
        .await
        .unwrap();
    assert_eq!(
        store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::System
    );
    let next = store.capture_profile_change().await.unwrap().unwrap();
    assert_ne!(first.operation, next.operation);
    assert_eq!(next.local, appearance("System"));
    assert_eq!(next.expected_revision, applied.revision);
    assert!(store.acknowledge_profile_change(admitted).await.is_err());
    assert_eq!(
        store.profile_replication(binding).await.unwrap().pending,
        Some(next)
    );
    worker.close().await.unwrap();
}

#[tokio::test]
async fn profile_edit_acknowledgment_drains_after_opt_out_without_restoring_options_or_current_values()
 {
    let directory = tempfile::tempdir().unwrap();
    let (store, mut worker, binding, _) = fixture(directory.path()).await;
    store
        .update_preferences(|p| p.appearance = Appearance::Dark)
        .await
        .unwrap();
    let pending = store.capture_profile_change().await.unwrap().unwrap();
    let admitted = worker.admit_local(pending.clone()).await.unwrap();
    store
        .change_profile_sync_options(enrollment::Changes {
            enabled: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    store.acknowledge_profile_change(admitted).await.unwrap();
    store
        .update_preferences(|p| p.appearance = Appearance::System)
        .await
        .unwrap();
    assert!(store.capture_profile_change().await.unwrap().is_none());
    assert!(
        !store
            .profile_enrollment()
            .await
            .unwrap()
            .enrollment
            .options
            .enabled
    );
    store
        .change_profile_sync_options(enrollment::Changes {
            enabled: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    let pending = store.capture_profile_change().await.unwrap().unwrap();
    let admitted = worker.admit_local(pending.clone()).await.unwrap();
    store
        .update_preferences(|p| p.google_lifecycle.disconnected = true)
        .await
        .unwrap();
    store.acknowledge_profile_change(admitted).await.unwrap();
    assert!(store.capture_profile_change().await.is_err());
    assert!(
        store
            .profile_replication(binding)
            .await
            .unwrap()
            .pending
            .is_none()
    );
    worker.close().await.unwrap();
}

#[tokio::test]
async fn profile_local_edit_cannot_be_silently_reparented_after_a_concurrent_remote_field() {
    let directory = tempfile::tempdir().unwrap();
    let (store, worker, binding, initial) = fixture(directory.path()).await;
    worker.close().await.unwrap();
    let worker = Worker::open(directory.path().join("history.sqlite"), binding.clone())
        .await
        .unwrap();
    store
        .update_preferences(|p| p.appearance = Appearance::Dark)
        .await
        .unwrap();
    let pending = store.capture_profile_change().await.unwrap().unwrap();
    let remote = shep_profile_core::Operation {
        format: shep_profile_core::FORMAT.into(),
        major: 1,
        minor: 0,
        requires: vec![
            "causal-v1".into(),
            "settings-v1".into(),
            "initialization-v1".into(),
        ],
        namespace: binding.namespace.clone(),
        profile: binding.profile,
        generation: binding.generation,
        device: Uuid::new_v4(),
        operation: Uuid::new_v4(),
        parents: vec![initial],
        changes: vec![appearance("System")],
        extra: Default::default(),
    };
    worker
        .request(Command::Import {
            record: String::from_utf8(remote.encode().unwrap()).unwrap(),
        })
        .await
        .unwrap();
    assert!(matches!(
        worker
            .request(Command::Edit {
                edit: pending.edit()
            })
            .await,
        Err(history::Error::Changed)
    ));
    assert_eq!(
        store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Dark
    );
    assert_eq!(store.capture_profile_change().await.unwrap(), Some(pending));
    worker.close().await.unwrap();
}

#[tokio::test]
async fn profile_replayed_edit_cannot_acknowledge_an_unseen_remote_successor_or_conflict() {
    for concurrent in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let (store, mut replica, binding, initial) = fixture(dir.path()).await;
        store
            .update_preferences(|p| p.appearance = Appearance::Dark)
            .await
            .unwrap();
        let pending = store.capture_profile_change().await.unwrap().unwrap();
        // The local history commit succeeds; its cache acknowledgment is lost.
        replica.edit(pending.edit()).await.unwrap();
        replica.close().await.unwrap();
        let worker = Worker::open(dir.path().join("history.sqlite"), binding.clone())
            .await
            .unwrap();
        let remote = shep_profile_core::Operation {
            format: shep_profile_core::FORMAT.into(),
            major: 1,
            minor: 0,
            requires: vec![
                "causal-v1".into(),
                "settings-v1".into(),
                "initialization-v1".into(),
            ],
            namespace: binding.namespace.clone(),
            profile: binding.profile,
            generation: binding.generation,
            device: Uuid::new_v4(),
            operation: Uuid::new_v4(),
            parents: vec![if concurrent {
                initial
            } else {
                pending.operation
            }],
            changes: vec![appearance("System")],
            extra: Default::default(),
        };
        worker
            .request(Command::Import {
                record: String::from_utf8(remote.encode().unwrap()).unwrap(),
            })
            .await
            .unwrap();
        // Shared Edit retries are idempotent even when the field has moved on.
        worker
            .request(Command::Edit {
                edit: pending.edit(),
            })
            .await
            .unwrap();
        worker.close().await.unwrap();
        let mut replica = Replica::open(
            dir.path().join("history.sqlite"),
            binding.clone(),
            Journal::open(None).unwrap(),
        )
        .await
        .unwrap();
        assert!(
            replica.admit_local(pending.clone()).await.is_err(),
            "concurrent={concurrent}"
        );
        assert_eq!(
            store.profile_replication(binding).await.unwrap().pending,
            Some(pending)
        );
        assert_eq!(
            store
                .get::<Preferences>("preferences")
                .await
                .unwrap()
                .appearance,
            Appearance::Dark
        );
        replica.close().await.unwrap();
    }
}

#[tokio::test]
async fn profile_category_pause_keeps_exact_pending_edit_and_does_not_capture_device_only_changes()
{
    let dir = tempfile::tempdir().unwrap();
    let (store, mut replica, _, _) = fixture(dir.path()).await;
    store
        .update_preferences(|p| {
            p.backup_folder = "/device-only".into();
            p.reader_font_size = 19;
        })
        .await
        .unwrap();
    assert!(store.capture_profile_change().await.unwrap().is_none());
    store
        .update_preferences(|p| p.appearance = Appearance::Dark)
        .await
        .unwrap();
    let pending = store.capture_profile_change().await.unwrap().unwrap();
    store
        .change_profile_sync_options(enrollment::Changes {
            settings: Some(false),
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .update_preferences(|p| p.appearance = Appearance::System)
        .await
        .unwrap();
    assert!(store.capture_profile_change().await.unwrap().is_none());
    store
        .change_profile_sync_options(enrollment::Changes {
            settings: Some(true),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        store.capture_profile_change().await.unwrap(),
        Some(pending.clone())
    );
    let admitted = replica.admit_local(pending).await.unwrap();
    store.acknowledge_profile_change(admitted).await.unwrap();
    assert_eq!(
        store.capture_profile_change().await.unwrap().unwrap().local,
        appearance("System")
    );
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_checkpoints_preserve_local_only_accounts_and_suppress_local_removal() {
    let dir = tempfile::tempdir().unwrap();
    let (store, mut worker, binding, _) = fixture(dir.path()).await;
    let op = shep_profile_core::Operation::decode(include_bytes!(
        "../../../../tests/support/profile-operation.json"
    ))
    .unwrap();
    let Action::AccountConnection {
        account: connection,
    } = &op.changes[0].action
    else {
        panic!("account")
    };
    let mut account = metadata::review_account(connection, "First").unwrap();
    store.save_account(account.clone()).await.unwrap();
    // An account added after enrollment becomes a new shared identity, saved
    // with its first request rather than regenerated on every sync retry.
    let first = store.capture_profile_change().await.unwrap().unwrap();
    let checkpoint = store.profile_replication(binding.clone()).await.unwrap();
    let shared = *checkpoint.accounts.get(&account.id).unwrap();
    assert_ne!(shared, connection.id);
    assert_eq!(
        store.capture_profile_change().await.unwrap(),
        Some(first.clone())
    );
    let admitted = worker.admit_local(first).await.unwrap();
    store.acknowledge_profile_change(admitted).await.unwrap();
    // Simulate only the already-existing account lifecycle's committed removal;
    // this cache test never reads credentials or contacts a provider.
    store.put("accounts", Vec::<Account>::new()).await.unwrap();
    assert!(store.capture_profile_change().await.unwrap().is_none());
    assert!(
        store
            .profile_replication(binding.clone())
            .await
            .unwrap()
            .suppressed
            .contains(&shared)
    );
    account.id = Uuid::new_v4().to_string();
    let mut state = State::new(
        binding,
        0,
        &[account.clone()],
        &Preferences::default(),
        Default::default(),
        vec![],
    )
    .unwrap();
    assert!(state.local_only.contains(&account.id));
    assert!(
        state
            .capture(
                &[account],
                &Preferences::default(),
                Options {
                    enabled: true,
                    ..Default::default()
                }
            )
            .unwrap()
            .is_none()
    );
    worker.close().await.unwrap();
}

#[test]
fn profile_checkpoint_capture_retains_field_basis_and_blocks_unsupported_state() {
    let binding = super::super::tests::selection().binding;
    let prefs = Preferences::default();
    let mut state = State::new(binding, 8, &[], &prefs, Default::default(), vec![]).unwrap();
    let key = history::target(&appearance("Light").action);
    state.fields.insert(
        key.clone(),
        Field {
            local: Some(appearance("Light")),
            remote: Some(appearance("Light")),
            revision: 2,
            native_revision: 0,
        },
    );
    let pending = state
        .capture(
            &[],
            &prefs,
            Options {
                enabled: true,
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        pending.expected_revision, 2,
        "A newer unrelated checkpoint cannot upgrade this field's causal basis"
    );
    state.fields.get_mut(&key).unwrap().revision = 9;
    assert!(state.validate().is_err());
}
