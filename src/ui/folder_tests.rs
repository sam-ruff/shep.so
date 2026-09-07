use super::*;
use crate::{
    folders::{Mailbox, NameEncoding},
    store::Store,
};
async fn fixture() -> (App, Store, tokio::sync::mpsc::Receiver<Command>) {
    let store = Store::memory().unwrap();
    for id in ["a", "b"] {
        let account:Account=serde_json::from_value(serde_json::json!({"id":id,"name":id,"email":format!("{id}@example.test"),"protocol":"Imap","host":"localhost","port":993,"username":id,"smtp_host":"localhost","smtp_port":465})).unwrap();
        store.save_account(account).await.unwrap();
        store
            .save_folder_catalog(
                id.into(),
                [
                    ("INBOX", true),
                    ("INBOX/Child", true),
                    ("Projects", true),
                    ("Projects/Design", true),
                    ("Teams", false),
                    ("Teams/Remote", true),
                    ("Empty", false),
                ]
                .into_iter()
                .map(|(name, selectable)| Mailbox {
                    name: name.into(),
                    delimiter: Some('/'),
                    selectable,
                    encoding: NameEncoding::Utf8,
                    no_inferiors: false,
                    non_existent: false,
                })
                .collect(),
            )
            .await
            .unwrap();
    }
    let (tx, rx) = engine::CommandSender::persistence_test_channel();
    let (mut app, _) = App::new();
    app.tx = Some(tx);
    app.workspace = Arc::new(store.workspace().await.unwrap());
    (app, store, rx)
}
fn index(app: &App, account: &str, path: &str) -> usize {
    app.sidebar_items().iter().position(|item|matches!(&item.action,Message::AccountFolder(a,p)|Message::ToggleFolderGroup(a,p) if a==account && p==path)).unwrap()
}
#[tokio::test]
async fn native_folder_expansion_is_immediate_scoped_and_preserves_newer_saves() {
    let (mut app, store, mut commands) = fixture().await;
    assert!(
        !app.sidebar_items()
            .iter()
            .any(|item| item.label == "Design")
    );
    let before = app.query.clone();
    let _ = app.handle(Message::ToggleFolderGroup("a".into(), "Projects".into()));
    assert!(app.folder_expanded("a", "Projects"));
    assert!(!app.folder_expanded("b", "Projects"));
    assert!(
        app.sidebar_items()
            .iter()
            .any(|item| item.label == "Design")
    );
    assert_eq!(app.query, before);
    let Command::SavePreferences(request, first) = commands.try_recv().unwrap() else {
        panic!("expected a queued preference save")
    };
    let _ = app.handle(Message::ToggleFolderGroup("a".into(), "Projects".into()));
    assert!(!app.folder_expanded("a", "Projects"));
    let snapshot = store.save_preferences(first).await.unwrap();
    let _ = app.handle(Message::Backend(Event::PreferencesSaved(
        request,
        Arc::new(snapshot),
    )));
    assert!(
        !app.folder_expanded("a", "Projects"),
        "an older acknowledgment must not reopen the group"
    );
}
#[tokio::test]
async fn sidebar_tree_keys_expand_and_focus_parents_without_selecting_containers() {
    let (mut app, _, _commands) = fixture().await;
    app.sidebar_index = index(&app, "a", "Projects");
    app.sidebar_focus = true;
    let before = app.query.clone();
    let _ = app.key(
        Key::Named(keyboard::key::Named::ArrowRight),
        keyboard::Modifiers::default(),
        false,
    );
    assert!(app.folder_expanded("a", "Projects"));
    let _ = app.key(
        Key::Named(keyboard::key::Named::ArrowRight),
        keyboard::Modifiers::default(),
        false,
    );
    assert_eq!(app.sidebar_index, index(&app, "a", "Projects/Design"));
    let _ = app.key(
        Key::Named(keyboard::key::Named::ArrowLeft),
        keyboard::Modifiers::default(),
        false,
    );
    assert_eq!(app.sidebar_index, index(&app, "a", "Projects"));
    let _ = app.key(
        Key::Named(keyboard::key::Named::ArrowLeft),
        keyboard::Modifiers::default(),
        false,
    );
    assert!(!app.folder_expanded("a", "Projects"));
    app.sidebar_index = index(&app, "a", "Teams") - 1;
    let _ = app.key(
        Key::Named(keyboard::key::Named::ArrowDown),
        keyboard::Modifiers::default(),
        false,
    );
    assert!(
        !app.folder_expanded("a", "Teams"),
        "passing a container with arrows must not toggle it"
    );
    let _ = app.key(
        Key::Named(keyboard::key::Named::Enter),
        keyboard::Modifiers::default(),
        false,
    );
    assert!(app.folder_expanded("a", "Teams"));
    assert_eq!(app.query, before);
    let container = &app.sidebar_items()[index(&app, "a", "Teams")].action;
    assert!(app.sidebar_folder(container).is_none());
    assert!(app.sidebar_drop_target(container).is_none());
}
#[tokio::test]
async fn unified_shortcuts_do_not_hide_inbox_children_or_make_containers_destinations() {
    let (mut app, _, _commands) = fixture().await;
    let _ = index(&app, "a", "INBOX");
    app.set_folder_expanded("a", "INBOX", true);
    assert!(app.sidebar_items().iter().any(
        |item| matches!(&item.action,Message::AccountFolder(a,p) if a=="a" && p=="INBOX/Child")
    ));
    let group = Message::ToggleFolderGroup("a".into(), "Teams".into());
    assert_eq!(
        app.sidebar_drag_reveal(&group),
        Some(drag_mail::Reveal::Folder("a".into(), "Teams".into()))
    );
    app.set_folder_expanded("a", "Teams", true);
    assert!(app.sidebar_drag_reveal(&group).is_none());
    app.set_folder_expanded("a", "Missing", true);
    assert!(!app.folder_expanded("a", "Missing"));
}

#[tokio::test]
async fn move_feedback_decodes_labels_without_changing_action_or_undo_identity() {
    let (mut app, store, _commands) = fixture().await;
    let wire = "Projects/&ZeVnLIqe-";
    store
        .save_folder_catalog(
            "a".into(),
            vec![Mailbox {
                name: wire.into(),
                delimiter: Some('/'),
                selectable: true,
                encoding: NameEncoding::ImapUtf7,
                no_inferiors: false,
                non_existent: false,
            }],
        )
        .await
        .unwrap();
    app.workspace = Arc::new(store.workspace().await.unwrap());
    let action = crate::bulk::Action::Move {
        account: Some("a".into()),
        folder: wire.into(),
    };
    assert_eq!(
        app.bulk_action_label(&action, None),
        "Move to Projects/日本語"
    );
    assert_eq!(
        app.bulk_action_label(&action, Some(2)),
        "Move 2 messages to Projects/日本語?"
    );
    assert!(matches!(action,crate::bulk::Action::Move{folder,..} if folder==wire));
    let token = app.action_toasts.add("a", wire, Instant::now());
    let toast = app.action_toasts.current.as_ref().unwrap();
    assert_eq!(
        toast.display_label(&app.workspace),
        "Moved 1 message to Projects/日本語"
    );
    assert_eq!(toast.undo_tokens(), vec![token]);
    // The same spelling is literal on a UTF-8 account. Never borrow another
    // account's encoding just because its folder name happens to match.
    app.action_toasts.add("b", wire, Instant::now());
    assert_eq!(
        app.action_toasts
            .current
            .as_ref()
            .unwrap()
            .display_label(&app.workspace),
        "Moved 1 message to Projects/&ZeVnLIqe-"
    );
}
