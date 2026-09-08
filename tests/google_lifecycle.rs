use shep::{model::*, store::Store};

fn source(id: &str, kind: CalendarKind) -> CalendarSource {
    CalendarSource {
        id: id.into(),
        name: id.into(),
        kind,
        url: format!("https://calendar.example.test/{id}/"),
        username: "alex".into(),
        access: Default::default(),
    }
}
async fn seed(store: &Store) -> Preferences {
    let prefs = Preferences {
        google_client_id: "fixture-client".into(),
        google_connection_id: "drive:original".into(),
        backup_destination: BackupDestination::GoogleDrive,
        auto_backup: true,
        backup_ready: true,
        last_backup: Some(42),
        ..Default::default()
    };
    store.put("preferences", prefs.clone()).await.unwrap();
    store
        .save_sources(vec![
            source("google:work", CalendarKind::Google),
            source("home", CalendarKind::CalDav),
        ])
        .await
        .unwrap();
    let start = chrono::DateTime::parse_from_rfc3339("2026-09-06T09:00:00Z")
        .unwrap()
        .to_utc();
    store
        .save_event(CalendarEvent {
            id: "event".into(),
            source_id: "google:work".into(),
            title: "Keep this event".into(),
            start,
            end: start + chrono::Duration::hours(1),
            location: String::new(),
            description: String::new(),
            all_day: false,
            etag: None,
            remote_url: None,
        })
        .await
        .unwrap();
    prefs
}

#[tokio::test]
async fn google_disconnect_keeps_cache_and_backup_identity_and_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace.sqlite");
    let store = Store::open(&path).unwrap();
    let old = seed(&store).await;
    let lifecycle = store.disconnect_google(0).await.unwrap();
    assert!(lifecycle.disconnected && lifecycle.cleanup_pending);
    assert_eq!(lifecycle.revision, 1);
    drop(store);
    let store = Store::open(path).unwrap();
    let workspace = store.workspace().await.unwrap();
    assert_eq!(
        workspace.preferences.google_connection_id,
        old.google_connection_id
    );
    assert_eq!(workspace.preferences.last_backup, Some(42));
    assert!(!workspace.preferences.auto_backup && !workspace.preferences.backup_ready);
    assert!(workspace.google_archived.contains("google:work"));
    assert!(
        workspace
            .calendars
            .iter()
            .find(|s| s.id == "google:work")
            .unwrap()
            .access
            .read_only()
    );
    assert!(
        workspace
            .calendars
            .iter()
            .find(|s| s.id == "home")
            .unwrap()
            .access
            .create
    );
    assert_eq!(
        store.calendar_snapshot().await.unwrap().1[0].title,
        "Keep this event"
    );
    let saved = store.save_preferences(old).await.unwrap();
    assert_eq!(saved.value.google_lifecycle, lifecycle);
    assert!(!saved.value.auto_backup && !saved.value.backup_ready);
    assert!(
        store
            .refresh_google_sources(vec![source("google:work", CalendarKind::Google)])
            .await
            .is_err()
    );
    assert!(
        connect_google(&store, "fixture-client", "drive:replacement")
            .await
            .is_err()
    );
    store.finish_google_cleanup(1).await.unwrap();
    let saved = connect_google(&store, "fixture-client", "drive:original")
        .await
        .unwrap();
    assert!(!saved.value.google_lifecycle.disconnected);
    assert!(!saved.value.auto_backup && !saved.value.backup_ready);
    assert!(store.finish_google_cleanup(1).await.is_err());
    assert!(store.disconnect_google(1).await.is_err());
    store
        .refresh_google_sources(vec![source("google:work", CalendarKind::Google)])
        .await
        .unwrap();
    let workspace = store.workspace().await.unwrap();
    assert!(workspace.google_archived.is_empty());
    assert!(
        workspace
            .calendars
            .iter()
            .find(|s| s.id == "google:work")
            .unwrap()
            .access
            .create
    );
    assert_eq!(store.calendar_snapshot().await.unwrap().1.len(), 1);
}

#[tokio::test]
async fn failed_google_disconnect_rolls_back_service_state_and_calendar_access() {
    let store = Store::memory().unwrap();
    let original = seed(&store).await;
    store.run(|c| {c.execute_batch("CREATE TRIGGER fail_archive BEFORE INSERT ON kv WHEN NEW.key='google_archived' BEGIN SELECT RAISE(ABORT,'fixture disk failure'); END;")?;Ok(())}).await.unwrap();
    assert!(store.disconnect_google(0).await.is_err());
    let workspace = store.workspace().await.unwrap();
    assert_eq!(workspace.preferences, original);
    assert!(workspace.google_archived.is_empty());
    assert!(workspace.calendars.iter().all(|s| s.access.create));
}

#[tokio::test]
async fn reconnect_only_reactivates_calendars_in_the_new_complete_list() {
    let store = Store::memory().unwrap();
    seed(&store).await;
    store.disconnect_google(0).await.unwrap();
    store
        .save_source(source("google:restored", CalendarKind::Google))
        .await
        .unwrap();
    assert!(
        store
            .workspace()
            .await
            .unwrap()
            .google_archived
            .contains("google:restored")
    );
    store.finish_google_cleanup(1).await.unwrap();
    connect_google(&store, "fixture-client", "drive:other")
        .await
        .unwrap();
    store
        .refresh_google_sources(vec![source("google:new", CalendarKind::Google)])
        .await
        .unwrap();
    let workspace = store.workspace().await.unwrap();
    assert!(workspace.google_archived.contains("google:work"));
    assert!(!workspace.google_archived.contains("google:new"));
    assert!(workspace.preferences.last_backup.is_none());
    assert_eq!(store.calendar_snapshot().await.unwrap().1.len(), 1);
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

#[tokio::test]
async fn partial_grants_preserve_cached_data_and_late_preferences_cannot_restore_old_access() {
    let store = Store::memory().unwrap();
    let original = seed(&store).await;
    let calendar_grant = GoogleGrant {
        id: "calendar-grant".into(),
        client_id: "fixture-client".into(),
        access: GoogleAccess {
            known: true,
            calendar_read: true,
            ..Default::default()
        },
    };
    let saved = store
        .activate_google(
            original.clone(),
            calendar_grant.clone(),
            None,
            vec![source("google:work", CalendarKind::Google)],
        )
        .await
        .unwrap();
    assert_eq!(saved.value.google_connection_id, "drive:original");
    assert_eq!(saved.value.last_backup, Some(42));
    assert!(!saved.value.auto_backup && !saved.value.backup_ready);
    let late = store.save_preferences(original.clone()).await.unwrap();
    assert_eq!(late.value.google_grant, calendar_grant);
    assert!(!late.value.auto_backup && !late.value.backup_ready);
    assert!(store.workspace().await.unwrap().google_archived.is_empty());
    assert!(
        store
            .workspace()
            .await
            .unwrap()
            .calendars
            .iter()
            .find(|s| s.id == "google:work")
            .unwrap()
            .access
            .read_only()
    );
    // Source setup and subsequent full role refresh cannot bypass the OAuth scope.
    store
        .save_source(source("google:work", CalendarKind::Google))
        .await
        .unwrap();
    store
        .refresh_google_sources(vec![source("google:work", CalendarKind::Google)])
        .await
        .unwrap();
    assert!(
        store
            .workspace()
            .await
            .unwrap()
            .calendars
            .iter()
            .find(|s| s.id == "google:work")
            .unwrap()
            .access
            .read_only()
    );
    let drive_grant = GoogleGrant {
        id: "drive-grant".into(),
        client_id: "fixture-client".into(),
        access: GoogleAccess {
            known: true,
            drive: true,
            ..Default::default()
        },
    };
    let committed = store
        .activate_google(
            late.value,
            drive_grant.clone(),
            Some("drive:original".into()),
            vec![],
        )
        .await
        .unwrap();
    let workspace = store.workspace().await.unwrap();
    assert_eq!(workspace.preferences.google_grant, drive_grant);
    assert_eq!(workspace.preferences.last_backup, Some(42));
    assert!(workspace.google_archived.contains("google:work"));
    assert!(
        workspace
            .calendars
            .iter()
            .find(|s| s.id == "home")
            .unwrap()
            .access
            .create
    );
    assert_eq!(store.calendar_snapshot().await.unwrap().1.len(), 1);
    assert!(
        store
            .refresh_google_sources(vec![source("google:work", CalendarKind::Google)])
            .await
            .is_err()
    );
    assert!(
        store
            .activate_google(original, calendar_grant, None, vec![])
            .await
            .is_err()
    );
    assert_eq!(
        store.get::<Preferences>("preferences").await.unwrap(),
        committed.value
    );
}

#[tokio::test]
async fn changed_oauth_settings_reject_staged_activation_without_changing_working_credentials() {
    let store = Store::memory().unwrap();
    let original = seed(&store).await;
    let old = connect_google(&store, "fixture-client", "drive:original")
        .await
        .unwrap()
        .value;
    let target = shep::backup::BackupTarget::from_preferences(&old);
    let mut edited = old.clone();
    edited.google_client_id = "next-client".into();
    edited.google_client_secret = "next-secret".into();
    let edited = store.save_preferences(edited).await.unwrap().value;
    assert_eq!(edited.google_grant, old.google_grant);
    assert_eq!(
        shep::backup::BackupTarget::from_preferences(&edited),
        target
    );
    assert_eq!(edited.last_backup, original.last_backup);
    let grant = GoogleGrant {
        id: "staged".into(),
        client_id: "fixture-client".into(),
        access: old.google_grant.access,
    };
    assert!(
        store
            .activate_google(old, grant, Some("drive:other".into()), vec![])
            .await
            .is_err()
    );
    assert_eq!(
        store.get::<Preferences>("preferences").await.unwrap(),
        edited
    );
}

#[tokio::test]
async fn changed_google_permission_choices_reject_late_activation_and_keep_the_active_grant() {
    use shep::model::{GoogleCalendarRequest, GoogleServices};
    let store = Store::memory().unwrap();
    seed(&store).await;
    let old = connect_google(&store, "fixture-client", "drive:original")
        .await
        .unwrap()
        .value;
    let mut edited = old.clone();
    edited.google_services = Some(GoogleServices {
        drive: false,
        calendar: GoogleCalendarRequest::ReadOnly,
    });
    let edited = store.save_preferences(edited).await.unwrap().value;
    assert_eq!(edited.google_grant, old.google_grant);
    assert_eq!(edited.google_lifecycle, old.google_lifecycle);
    let candidate = GoogleGrant {
        id: "candidate".into(),
        ..old.google_grant.clone()
    };
    assert!(
        store
            .activate_google(old, candidate, Some("drive:other".into()), vec![])
            .await
            .is_err()
    );
    assert_eq!(
        store.get::<Preferences>("preferences").await.unwrap(),
        edited
    );
    assert!(store.workspace().await.unwrap().google_archived.is_empty());
}
