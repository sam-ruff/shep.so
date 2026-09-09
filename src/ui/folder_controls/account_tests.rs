use super::super::tests::{fixture, prepare};
use super::*;

fn aggregate(folder: &str) -> FolderSelection {
    FolderSelection {
        account: None,
        folder: folder.into(),
        sent_only: false,
    }
}

#[tokio::test]
async fn aggregate_account_choice_requires_explicit_account_before_review() {
    let (mut app, _, _) = fixture().await;
    let (tx, mut reads) = engine::CommandSender::foreground_test_channel();
    app.tx = Some(tx);
    app.begin_folder_accounts(aggregate("Archive"), true);
    assert!(app.folder_controls.choosing_account);
    assert_eq!(app.folder_account_choices().len(), 2);
    app.handle_folders(Message::Submit);
    assert!(reads.try_recv().is_err());
    let _ = app.move_folder_account_choice(1);
    app.choose_focused_folder_account();
    assert_eq!(app.folder_controls.account, "b");
    assert!(!app.folder_controls.choosing_account);
    let Command::Folder(Request::Review(_, account, source, action, _, _)) =
        reads.try_recv().unwrap()
    else {
        panic!()
    };
    assert_eq!(
        (account.as_str(), source.as_str(), action),
        ("b", "Archive", Change::Delete)
    );
    assert!(app.folder_controls.pending.is_empty());
}

#[tokio::test]
async fn common_sent_account_choice_resolves_each_configured_wire_path() {
    let (mut app, _, _) = fixture().await;
    let workspace = Arc::make_mut(&mut app.workspace);
    for (account, path) in [("a", "A. Keep"), ("b", "&AMk-l&AOk-ments envoy&AOk-s")] {
        workspace
            .accounts
            .iter_mut()
            .find(|value| value.id == account)
            .unwrap()
            .sent_folder = path.into();
        workspace.folder_trees.insert(
            account.into(),
            Arc::new(crate::folders::Tree::new(&[crate::folders::Mailbox::flat(
                path.into(),
            )])),
        );
    }
    app.begin_folder_accounts(
        FolderSelection {
            sent_only: true,
            ..aggregate("Sent")
        },
        false,
    );
    let choices = app.folder_account_choices();
    assert_eq!(
        choices
            .iter()
            .map(|choice| choice.folder.as_str())
            .collect::<Vec<_>>(),
        ["A. Keep", "&AMk-l&AOk-ments envoy&AOk-s"]
    );
    let (tx, mut reads) = engine::CommandSender::foreground_test_channel();
    app.tx = Some(tx);
    app.select_folder_account("b");
    assert!(
        matches!(reads.try_recv(), Ok(Command::Folder(Request::Options(_, account, folder))) if account == "b" && folder == "&AMk-l&AOk-ments envoy&AOk-s")
    );
    assert!(app.folder_parent_visible());
    // A common shortcut scoped to the selected account must retain that
    // account's configured path, even after a different aggregate choice.
    app.dialog = None;
    app.query.account = Some("a".into());
    let target = app
        .sidebar_folder_context(&crate::ui::Message::SentFolder)
        .unwrap();
    assert_eq!(target.account.as_deref(), Some("a"));
    app.begin_folder_accounts(target, true);
    assert!(!app.folder_controls.choosing_account);
    assert!(
        matches!(reads.try_recv(), Ok(Command::Folder(Request::Review(_, account, folder, Change::Delete, _, _))) if account == "a" && folder == "A. Keep")
    );
}

#[tokio::test]
async fn changing_account_releases_review_and_ignores_its_late_response() {
    let (mut app, store, _) = fixture().await;
    let old = prepare(&mut app, &store, Change::Delete).await;
    app.folder_controls.account_scope = Some(aggregate("Projects"));
    app.folder_controls.account = "a".into();
    let serial = app.folder_controls.serial;
    let (tx, mut reads) = engine::CommandSender::foreground_test_channel();
    app.tx = Some(tx);
    app.show_folder_accounts();
    assert!(
        matches!(reads.try_recv(), Ok(Command::Folder(Request::Release(token))) if token == old.projection.as_ref().unwrap().0)
    );
    app.select_folder_account("b");
    assert!(
        matches!(reads.try_recv(), Ok(Command::Folder(Request::Review(_, account, _, _, _, _))) if account == "b")
    );
    app.folder_event(FolderEvent::Review(serial, Ok(old)));
    assert_eq!(app.folder_controls.account, "b");
    assert!(app.folder_controls.preview.is_none());
    assert!(app.folder_controls.loading);
    assert!(matches!(
        reads.try_recv(),
        Ok(Command::Folder(Request::Release(_)))
    ));
}

#[tokio::test]
async fn aggregate_chooser_survives_removed_choice_and_can_use_another_account() {
    for focus_survives in [false, true] {
        let (mut app, _, _) = fixture().await;
        app.begin_folder_accounts(aggregate("Archive"), true);
        if focus_survives {
            let _ = app.move_folder_account_choice(1);
        }
        Arc::make_mut(&mut app.workspace)
            .accounts
            .retain(|account| account.id != "a");
        app.reconcile_folder_accounts();
        assert_eq!(app.dialog, Some(Dialog::FolderChange));
        if !focus_survives {
            assert!(app.folder_controls.account_focus.is_none());
            app.choose_focused_folder_account();
            assert!(
                app.folder_controls.choosing_account,
                "Removing a highlighted account must not silently choose another one"
            );
            let _ = app.move_folder_account_choice(1);
        }
        assert_eq!(app.folder_controls.account_focus.as_deref(), Some("b"));
        assert_eq!(app.folder_controls.account_index, 0);
        app.choose_focused_folder_account();
        assert_eq!(app.folder_controls.account, "b");
        assert!(!app.folder_controls.choosing_account);
    }
}

#[tokio::test]
async fn aggregate_chooser_rejects_busy_account_and_preserves_its_pending_change() {
    let (mut app, store, _commands) = fixture().await;
    prepare(&mut app, &store, Change::Delete).await;
    app.handle_folders(Message::Submit);
    assert!(app.folder_busy("a"));
    app.begin_folder_accounts(aggregate("Archive"), false);
    app.select_folder_account("a");
    assert!(app.folder_controls.choosing_account);
    assert!(
        app.folder_controls
            .error
            .as_ref()
            .unwrap()
            .contains("pending")
    );
    app.select_folder_account("b");
    assert_eq!(app.folder_controls.account, "b");
    assert!(app.folder_busy("a"));
    assert_eq!(app.folder_controls.pending.len(), 1);
}
