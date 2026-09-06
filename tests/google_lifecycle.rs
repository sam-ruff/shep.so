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
        store
            .record_google_connection("fixture-client".into(), "drive:replacement".into())
            .await
            .is_err()
    );
    store.finish_google_cleanup(1).await.unwrap();
    let saved = store
        .record_google_connection("fixture-client".into(), "drive:original".into())
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
    store
        .record_google_connection("fixture-client".into(), "drive:other".into())
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
