use shep::{
    backup::BackupTarget,
    model::{Appearance, Preferences},
    store::Store,
};

#[tokio::test]
async fn stale_ui_save_cannot_erase_new_backup_history() {
    let store = Store::memory().unwrap();
    let mut old_ui = Preferences::default();
    let initial = store.save_preferences(old_ui.clone()).await.unwrap();
    let completed = store
        .update_preferences(|p| p.last_backup = Some(1234))
        .await
        .unwrap();
    old_ui.appearance = Appearance::Dark;
    let saved = store.save_preferences(old_ui).await.unwrap();
    assert!(initial.revision < completed.revision && completed.revision < saved.revision);
    assert_eq!(saved.value.appearance, Appearance::Dark);
    assert_eq!(saved.value.last_backup, Some(1234));
    let workspace = store.workspace().await.unwrap();
    assert_eq!(workspace.preferences_revision, saved.revision);
    assert_eq!(workspace.preferences, saved.value);
}

#[tokio::test]
async fn backup_completion_preserves_settings_changed_while_uploading() {
    let store = Store::memory().unwrap();
    let backup_started_with = store
        .save_preferences(Preferences::default())
        .await
        .unwrap();
    let mut changed = backup_started_with.value.clone();
    changed.reader_split = 0.6;
    changed.reader_font_size = 24;
    changed.backup_copies = 15;
    let changed = store.save_preferences(changed).await.unwrap();
    // The upload finishes later. Only its timestamp belongs to that operation.
    let completed = store
        .record_backup(
            BackupTarget::from_preferences(&backup_started_with.value),
            2345,
            true,
        )
        .await
        .unwrap();
    assert!(completed.revision > changed.revision);
    assert_eq!(completed.value.reader_split, 0.6);
    assert_eq!(completed.value.reader_font_size, 24);
    assert_eq!(completed.value.backup_copies, 15);
    assert_eq!(completed.value.last_backup, Some(2345));
    assert!(completed.value.backup_ready);
}

#[tokio::test]
async fn a_new_backup_destination_never_inherits_history_or_readiness() {
    let store = Store::memory().unwrap();
    let old = store
        .save_preferences(Preferences {
            backup_folder: "/first".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    let old_target = BackupTarget::from_preferences(&old.value);
    let completed = store
        .record_backup(old_target.clone(), 123, true)
        .await
        .unwrap();
    let mut changed = completed.value;
    changed.backup_folder = "/second".into();
    let saved = store.save_preferences(changed).await.unwrap();
    assert_eq!(saved.value.last_backup, None);
    assert!(!saved.value.backup_ready);
    let late = store.record_backup(old_target, 456, true).await.unwrap();
    assert_eq!(late.value.backup_folder, "/second");
    assert_eq!(late.value.last_backup, None);
    assert!(!late.value.backup_ready);
}

#[tokio::test]
async fn old_ui_cannot_restore_a_previous_google_connection_or_backup_metadata() {
    use shep::model::BackupDestination;
    let store = Store::memory().unwrap();
    let initial = store
        .save_preferences(Preferences {
            backup_destination: BackupDestination::GoogleDrive,
            ..Default::default()
        })
        .await
        .unwrap();
    let previous = BackupTarget::from_preferences(&initial.value);
    let mut old_ui = store
        .record_backup(previous.clone(), 12, true)
        .await
        .unwrap()
        .value;
    store
        .update_preferences(|p| {
            p.google_connection_id = "new-authorization".into();
            p.last_backup = None;
            p.backup_ready = false;
        })
        .await
        .unwrap();
    old_ui.appearance = Appearance::Dark;
    let saved = store.save_preferences(old_ui).await.unwrap();
    assert_eq!(saved.value.google_connection_id, "new-authorization");
    assert_eq!(saved.value.last_backup, None);
    assert!(!saved.value.backup_ready);
    assert_eq!(saved.value.appearance, Appearance::Dark);
    let late = store.record_backup(previous, 24, true).await.unwrap();
    assert_eq!(late.value.last_backup, None);
}

#[tokio::test]
async fn reconnecting_the_same_drive_account_preserves_history_but_other_accounts_do_not() {
    use shep::model::BackupDestination;
    let store = Store::memory().unwrap();
    store
        .save_preferences(Preferences {
            google_client_id: "client".into(),
            backup_destination: BackupDestination::GoogleDrive,
            ..Default::default()
        })
        .await
        .unwrap();
    let connected = store
        .record_google_connection("client".into(), "drive:first".into())
        .await
        .unwrap();
    let target = BackupTarget::from_preferences(&connected.value);
    store
        .record_backup(target.clone(), 123, true)
        .await
        .unwrap();
    let reconnected = store
        .record_google_connection("client".into(), "drive:first".into())
        .await
        .unwrap();
    assert_eq!(reconnected.value.last_backup, Some(123));
    assert!(reconnected.value.backup_ready);
    assert_eq!(BackupTarget::from_preferences(&reconnected.value), target);
    let other = store
        .record_google_connection("client".into(), "drive:other".into())
        .await
        .unwrap();
    assert_eq!(other.value.last_backup, None);
    assert!(!other.value.backup_ready);
    let revision = other.revision;
    assert!(
        store
            .record_google_connection("stale-client".into(), "drive:first".into())
            .await
            .is_err()
    );
    let saved = store.workspace().await.unwrap();
    assert_eq!(saved.preferences_revision, revision);
    assert_eq!(saved.preferences.google_connection_id, "drive:other");
}

#[tokio::test]
async fn preferences_revision_survives_reopen_and_failed_validation_is_atomic() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("prefs.sqlite");
    let store = Store::open(&path).unwrap();
    let saved = store
        .save_preferences(Preferences {
            reader_font_size: 19,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(
        store
            .save_preferences(Preferences {
                reader_font_size: 0,
                ..Default::default()
            })
            .await
            .is_err()
    );
    drop(store);
    let store = Store::open(&path).unwrap();
    let workspace = store.workspace().await.unwrap();
    assert_eq!(workspace.preferences_revision, saved.revision);
    assert_eq!(workspace.preferences.reader_font_size, 19);
    let next = store
        .save_preferences(Preferences {
            reader_font_size: 22,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(next.revision, saved.revision + 1);
}
