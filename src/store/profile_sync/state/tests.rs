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
    let mut common = appearance("Light");
    common
        .extra
        .insert("future_hint".into(), json!({"keep":"opaque"}));
    let state = worker
        .edit(history::LocalEdit {
            operation: initial,
            expected_revision: 0,
            changes: vec![common.clone()],
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
        requires: vec!["causal-v1".into(), "settings-v1".into()],
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
            requires: vec!["causal-v1".into(), "settings-v1".into()],
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
