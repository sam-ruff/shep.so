use shep::{
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
        .update_preferences(|p| p.last_backup = Some(2345))
        .await
        .unwrap();
    assert!(completed.revision > changed.revision);
    assert_eq!(completed.value.reader_split, 0.6);
    assert_eq!(completed.value.reader_font_size, 24);
    assert_eq!(completed.value.backup_copies, 15);
    assert_eq!(completed.value.last_backup, Some(2345));
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
