use super::*;
use crate::store::Store;

#[tokio::test]
async fn google_login_waits_for_saved_permissions_and_rejects_changed_choices() {
    for changed in [false, true] {
        let (mut app, _) = App::new();
        app.google_client = Some("fixture-client".into());
        app.preferences.google_services = Some(GoogleServices {
            drive: true,
            calendar: GoogleCalendarRequest::Off,
        });
        app.settings_fields();
        let (saves, mut saved) = engine::CommandSender::persistence_test_channel();
        app.tx = Some(saves);
        let _ = app.handle(Message::GoogleLogin(true));
        let Command::SavePreferences(request, write) = saved.try_recv().unwrap() else {
            panic!()
        };
        let prefs = write.value;
        assert_eq!(prefs.google_services, app.preferences.google_services);
        assert!(app.pending_google_login.is_some());
        if changed {
            let _ = app.handle(Message::GoogleCalendarAccess(
                GoogleCalendarRequest::ReadOnly,
            ));
            assert!(matches!(
                saved.try_recv().unwrap(),
                Command::SavePreferences(..)
            ));
        }
        let (network, mut commands) = engine::CommandSender::network_test_channel();
        app.tx = Some(network);
        let _ = app.handle(Message::Backend(Event::PreferencesSaved(
            request,
            Arc::new(crate::store::PreferenceSnapshot {
                revision: 1,
                value: prefs.clone(),
            }),
        )));
        assert!(app.pending_google_login.is_none());
        if changed {
            assert!(commands.try_recv().is_err());
            assert!(
                app.notice
                    .as_ref()
                    .unwrap()
                    .0
                    .contains("Google setup changed")
            );
            assert_eq!(
                app.preferences.requested_google_services().calendar,
                GoogleCalendarRequest::ReadOnly
            );
        } else {
            let Command::GoogleLogin(actual, true, _) = commands.try_recv().unwrap() else {
                panic!()
            };
            assert_eq!(actual.google_services, prefs.google_services);
            assert_eq!(actual.google_grant, prefs.google_grant);
        }
    }
}

#[test]
fn google_sign_in_with_no_selected_services_never_enters_the_work_queue() {
    let (mut app, _) = App::new();
    app.google_client = Some("fixture-client".into());
    app.preferences.google_services = Some(GoogleServices::default());
    app.settings_fields();
    let (saves, mut saved) = engine::CommandSender::persistence_test_channel();
    app.tx = Some(saves);
    let _ = app.handle(Message::GoogleLogin(true));
    assert!(saved.try_recv().is_err());
    assert!(app.pending_google_login.is_none());
    assert!(
        app.notice
            .as_ref()
            .unwrap()
            .0
            .contains("Choose Drive backup or Calendar access")
    );
}

#[test]
fn a_build_without_a_google_client_disables_sign_in_and_never_saves_or_queues() {
    let (mut app, _) = App::new();
    app.google_client = None;
    app.preferences.google_services = Some(GoogleServices {
        drive: true,
        calendar: GoogleCalendarRequest::ReadOnly,
    });
    app.settings_fields();
    assert!(!app.google_sign_in_enabled());
    assert_eq!(app.google_sign_in_state()["available"], false);
    assert_eq!(app.google_sign_in_label(), "Sign in with Google");
    let (saves, mut saved) = engine::CommandSender::persistence_test_channel();
    app.tx = Some(saves);
    for retry in [true, false] {
        let _ = app.handle(Message::GoogleLogin(retry));
        assert!(saved.try_recv().is_err());
        assert!(app.pending_google_login.is_none());
        assert_eq!(
            app.notice.as_ref().unwrap().0,
            "Google sign-in is not configured in this build."
        );
    }
    app.google_client = Some("built-in".into());
    assert!(app.google_sign_in_enabled());
}

#[test]
fn cancelling_a_waiting_sign_in_cancels_the_queued_login_and_closing_does_too() {
    for close in [false, true] {
        let (mut app, _) = App::new();
        app.google_client = Some("built-in".into());
        app.preferences.google_services = Some(GoogleServices {
            drive: true,
            calendar: GoogleCalendarRequest::Off,
        });
        let (network, mut commands) = engine::CommandSender::network_test_channel();
        app.tx = Some(network);
        app.start_google_login(app.preferences.clone(), true);
        let Command::GoogleLogin(_, true, cancel) = commands.try_recv().unwrap() else {
            panic!()
        };
        assert!(app.google_waiting() && app.busy.contains("google"));
        assert!(!app.google_sign_in_enabled());
        assert_eq!(app.google_sign_in_state()["waiting"], true);
        if close {
            let _ = app.handle(Message::WindowClose(iced::window::Id::unique()));
        } else {
            let _ = app.handle(Message::CancelGoogleSignIn);
        }
        assert!(cancel.is_cancelled());
        assert!(!app.google_waiting());
        let _ = app.handle(Message::Backend(Event::Busy("google".into(), false)));
        assert!(app.google_sign_in.is_none());
        assert!(app.google_sign_in_enabled());
    }
}

#[test]
fn changed_permissions_after_the_save_do_not_start_the_older_sign_in() {
    let (mut app, _) = App::new();
    app.google_client = Some("built-in".into());
    let saved = app.preferences.clone();
    app.preferences.google_services = Some(GoogleServices {
        drive: true,
        calendar: GoogleCalendarRequest::Off,
    });
    let (network, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(network);
    app.start_google_login(saved, true);
    assert!(commands.try_recv().is_err());
    assert!(app.google_sign_in.is_none());
    assert!(
        app.notice
            .as_ref()
            .unwrap()
            .0
            .contains("Google setup changed")
    );
}

#[test]
fn only_a_connection_from_a_self_configured_client_shows_the_switch_note() {
    let (mut app, _) = App::new();
    app.google_client = Some("built-in".into());
    app.google_connected = true;
    app.preferences.google_grant.client_id = "built-in".into();
    assert!(!app.google_legacy_client());
    assert_eq!(app.google_sign_in_label(), "Reconnect Google");
    app.preferences.google_grant.client_id = "own-client".into();
    assert!(app.google_legacy_client());
    // A grant saved before grants recorded their client uses the stored client.
    app.preferences.google_grant.client_id.clear();
    app.preferences.google_client_id = "own-client".into();
    assert!(app.google_legacy_client());
    app.google_connected = false;
    assert!(!app.google_legacy_client());
}

#[tokio::test]
async fn old_google_status_and_workspace_cannot_reconnect_or_unlock_archived_calendars() {
    let store = Store::memory().unwrap();
    store
        .save_source(CalendarSource {
            id: "google:work".into(),
            name: "Work".into(),
            kind: CalendarKind::Google,
            url: "work@example.test".into(),
            username: String::new(),
            access: Default::default(),
        })
        .await
        .unwrap();
    let before = Arc::new(store.workspace().await.unwrap());
    let (mut app, _) = App::new();
    let _ = app.handle(Message::Backend(Event::Workspace(before.clone())));
    let _ = app.handle(Message::Backend(Event::GoogleStatus(0, true)));
    assert!(app.google_connected);
    app.preferences.appearance = Appearance::Dark;
    app.preference_sync.changed();
    store.disconnect_google(0).await.unwrap();
    let after = Arc::new(store.workspace().await.unwrap());
    let _ = app.handle(Message::Backend(Event::Workspace(after)));
    let _ = app.handle(Message::Backend(Event::GoogleStatus(0, true)));
    let _ = app.handle(Message::Backend(Event::Workspace(before)));
    assert!(!app.google_connected);
    assert!(app.preferences.google_lifecycle.disconnected);
    assert_eq!(app.preferences.appearance, Appearance::Dark);
    assert!(app.workspace.google_archived.contains("google:work"));
    assert!(app.workspace.calendars[0].access.read_only());
}

#[test]
fn google_disconnect_completion_leaves_an_unrelated_editor_open() {
    let (mut app, _) = App::new();
    app.google_disconnect_pending = Some(0);
    app.open(Dialog::Event);
    let _ = app.handle(Message::Field("title", "Keep this event".into()));
    let _ = app.handle(Message::Backend(Event::GoogleDisconnected(0, Ok(()))));
    assert_eq!(app.dialog, Some(Dialog::Event));
    assert_eq!(app.field("title"), "Keep this event");
}

#[tokio::test]
async fn google_partial_grant_metadata_survives_newer_local_edits_and_old_workspace_results() {
    let store = Store::memory().unwrap();
    let before = Arc::new(store.workspace().await.unwrap());
    let (mut app, _) = App::new();
    let _ = app.handle(Message::Backend(Event::Workspace(before.clone())));
    app.preferences.appearance = Appearance::Dark;
    app.preference_sync.changed();
    let prefs: Preferences = store.get("preferences").await.unwrap();
    let grant = GoogleGrant {
        id: "fixture".into(),
        client_id: "fixture-client".into(),
        access: GoogleAccess {
            known: true,
            calendar_read: true,
            ..Default::default()
        },
    };
    store
        .activate_google(prefs, grant.clone(), None, vec![])
        .await
        .unwrap();
    let after = Arc::new(store.workspace().await.unwrap());
    let _ = app.handle(Message::Backend(Event::Workspace(after)));
    let _ = app.handle(Message::Backend(Event::Workspace(before)));
    assert_eq!(app.preferences.google_grant, grant);
    assert_eq!(app.preferences.appearance, Appearance::Dark);
    app.preferences.backup_destination = BackupDestination::GoogleDrive;
    app.settings_fields();
    app.begin_backup_request(backups::BackupAction::List);
    assert!(app.pending_backup.is_none());
    assert!(app.visible_backups().is_empty());
}
