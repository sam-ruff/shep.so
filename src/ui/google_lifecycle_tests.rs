use super::*;
use crate::store::Store;

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
