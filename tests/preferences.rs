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
    let connected = connect_google(&store, "client", "drive:first")
        .await
        .unwrap();
    let target = BackupTarget::from_preferences(&connected.value);
    store
        .record_backup(target.clone(), 123, true)
        .await
        .unwrap();
    let reconnected = connect_google(&store, "client", "drive:first")
        .await
        .unwrap();
    assert_eq!(reconnected.value.last_backup, Some(123));
    assert!(reconnected.value.backup_ready);
    assert_eq!(BackupTarget::from_preferences(&reconnected.value), target);
    let other = connect_google(&store, "client", "drive:other")
        .await
        .unwrap();
    assert_eq!(other.value.last_backup, None);
    assert!(!other.value.backup_ready);
    let revision = other.revision;
    assert!(
        connect_google(&store, "stale-client", "drive:first")
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
            mail_check_seconds: 5,
            sidebar_width: Some(312.),
            window_size: Some(shep::model::WindowSize {
                width: 1234.,
                height: 789.,
            }),
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
    assert_eq!(workspace.preferences.mail_check_seconds, 5);
    assert_eq!(workspace.preferences.sidebar_width, Some(312.));
    assert_eq!(
        workspace.preferences.window_size,
        Some(shep::model::WindowSize {
            width: 1234.,
            height: 789.
        })
    );
    let next = store
        .save_preferences(Preferences {
            reader_font_size: 22,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(next.revision, saved.revision + 1);
}

async fn connect_google(
    store: &Store,
    client: &str,
    identity: &str,
) -> anyhow::Result<shep::store::PreferenceSnapshot> {
    use shep::model::{CalendarKind, GoogleAccess, GoogleGrant};
    let mut prefs: Preferences = store.get("preferences").await?;
    prefs.google_client_id = client.into();
    let sources = store
        .workspace()
        .await?
        .calendars
        .into_iter()
        .filter(|s| s.kind == CalendarKind::Google)
        .collect();
    store
        .activate_google(
            prefs,
            GoogleGrant {
                id: uuid::Uuid::new_v4().to_string(),
                client_id: client.into(),
                access: GoogleAccess {
                    known: true,
                    drive: true,
                    calendar_read: true,
                    calendar_write: true,
                },
            },
            Some(identity.into()),
            sources,
        )
        .await
}

#[test]
fn older_settings_gain_frequent_mail_checks_and_new_values_round_trip() {
    let old = serde_json::json!({"sync_minutes": 5});
    let mut preferences: shep::model::Preferences = serde_json::from_value(old).unwrap();
    assert_eq!(preferences.mail_check_seconds, 15);
    preferences.mail_check_seconds = 5;
    preferences.validate().unwrap();
    let reloaded: shep::model::Preferences =
        serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
    assert_eq!(reloaded.mail_check_seconds, 5);
    preferences.mail_check_seconds = 0;
    assert!(preferences.validate().is_err());
}

#[tokio::test]
async fn multiple_backup_destinations_keep_independent_history_across_edits_and_restart() {
    use shep::backup::config;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.sqlite");
    let store = Store::open(&path).unwrap();
    let mut prefs = Preferences {
        backup_folder: directory.path().join("one").to_string_lossy().into(),
        auto_backup: true,
        backup_copies: 3,
        ..Default::default()
    };
    store.save_preferences(prefs.clone()).await.unwrap();
    let first = BackupTarget::from_preferences(&prefs);
    prefs = store
        .record_backup(first.clone(), 10, true)
        .await
        .unwrap()
        .value;
    config::add(&mut prefs).unwrap();
    prefs.backup_folder = directory.path().join("two").to_string_lossy().into();
    prefs.backup_copies = 9;
    prefs.auto_backup = true;
    config::capture_editor(&mut prefs);
    let second = BackupTarget::from_preferences(&prefs);
    let mut stale = store.save_preferences(prefs).await.unwrap().value;
    store.record_backup(second.clone(), 20, true).await.unwrap();
    stale.appearance = Appearance::Dark;
    let saved = store.save_preferences(stale).await.unwrap().value;
    let one = config::resolve(&saved, &first).unwrap();
    let two = config::resolve(&saved, &second).unwrap();
    assert_eq!(
        (one.backup_copies, one.last_backup, one.backup_ready),
        (3, Some(10), true)
    );
    assert_eq!(
        (two.backup_copies, two.last_backup, two.backup_ready),
        (9, Some(20), true)
    );
    let mut changed = saved;
    changed.backup_folder = directory.path().join("three").to_string_lossy().into();
    config::capture_editor(&mut changed);
    let saved = store.save_preferences(changed).await.unwrap().value;
    assert_eq!(saved.last_backup, None);
    assert!(!saved.backup_ready);
    let after = store.record_backup(second, 30, true).await.unwrap().value;
    assert_eq!(after.last_backup, None);
    assert_eq!(
        config::resolve(&after, &first).unwrap().last_backup,
        Some(10)
    );
    drop(store);
    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        reopened.get::<Preferences>("preferences").await.unwrap(),
        after
    );
}

#[tokio::test]
async fn multiple_backup_destinations_reject_duplicate_drive_and_folder_aliases() {
    use shep::{backup::config, model::BackupDestination};
    let directory = tempfile::tempdir().unwrap();
    let mut prefs = Preferences {
        backup_folder: directory.path().to_string_lossy().into(),
        ..Default::default()
    };
    config::add(&mut prefs).unwrap();
    prefs.backup_folder = format!("{}/./", directory.path().display());
    config::capture_editor(&mut prefs);
    assert!(
        prefs
            .validate()
            .unwrap_err()
            .to_string()
            .contains("already configured")
    );
    for destination in &mut prefs.backup_destinations {
        destination.destination = BackupDestination::GoogleDrive;
    }
    assert!(prefs.validate().is_err());
    #[cfg(unix)]
    {
        let original = directory.path().join("original");
        std::fs::create_dir(&original).unwrap();
        let alias = directory.path().join("alias");
        std::os::unix::fs::symlink(&original, &alias).unwrap();
        prefs.backup_destinations[0].destination = BackupDestination::Local;
        prefs.backup_destinations[0].folder = original.to_string_lossy().into();
        prefs.backup_destinations[1].destination = BackupDestination::Local;
        prefs.backup_destinations[1].folder = alias.to_string_lossy().into();
        let second = prefs.backup_destinations[1].clone();
        second.apply(&mut prefs);
        let store = Store::memory().unwrap();
        assert!(
            store
                .save_preferences(prefs)
                .await
                .unwrap_err()
                .to_string()
                .contains("same folder")
        );
    }
}

#[tokio::test]
async fn multiple_backup_form_save_preserves_remote_preferences_and_other_target_receipts() {
    let store = Store::memory().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let first = directory
        .path()
        .join("first")
        .to_string_lossy()
        .into_owned();
    let second = directory
        .path()
        .join("second")
        .to_string_lossy()
        .into_owned();
    let mut form = Preferences {
        backup_folder: first.clone(),
        ..Default::default()
    };
    shep::backup::config::add(&mut form).unwrap();
    form.backup_folder = second.clone();
    let initial = store.save_preferences(form).await.unwrap();
    let mut stale_form = initial.value;
    // Another device changes an unrelated portable field, then the other local
    // destination finishes its already-started upload while this form is open.
    store
        .update_preferences(|p| p.tooltips = false)
        .await
        .unwrap();
    store
        .record_backup(BackupTarget::Local(first), 4567, true)
        .await
        .unwrap();
    stale_form.backup_copies = 19;
    let saved = store
        .save_preferences(shep::preference_edits::Write {
            value: stale_form,
            portable: Default::default(),
        })
        .await
        .unwrap()
        .value;
    assert!(!saved.tooltips);
    assert_eq!(saved.backup_folder, second);
    assert_eq!(saved.backup_copies, 19);
    assert_eq!(saved.backup_destinations[1].copies, 19);
    assert_eq!(saved.backup_destinations[0].last_backup, Some(4567));
    assert!(saved.backup_destinations[0].ready);
    assert_eq!(saved.backup_destinations[1].last_backup, None);
}
