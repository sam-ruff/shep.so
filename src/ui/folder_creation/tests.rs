use super::*;

fn admitted(id: String, request: &engine::folder_creation::Request) -> crate::store::CreationJob {
    crate::store::CreationJob {
        id,
        account: request.account.clone(),
        connection: request.connection.clone(),
        parent: request.parent.clone(),
        name: request.name.clone(),
        stage: crate::store::CreationStage::Queued,
        target: None,
        receipt: None,
        provider_acknowledged: false,
        error: None,
        revision: 1,
    }
}

async fn fixture() -> (App, tokio::sync::mpsc::Receiver<Command>) {
    let store = crate::store::Store::memory().expect("fixture store");
    for id in ["a", "b"] {
        let account: Account = serde_json::from_value(serde_json::json!({
            "id":id,"name":id,"email":format!("{id}@example.test"),"protocol":"Pop3",
            "host":"localhost","port":995,"username":id,"smtp_host":"localhost","smtp_port":465
        }))
        .expect("fixture account");
        store
            .save_account(account)
            .await
            .expect("save fixture account");
    }
    let (tx, rx) = engine::CommandSender::network_test_channel();
    let (mut app, _) = App::new();
    app.workspace = Arc::new(store.workspace().await.expect("fixture workspace"));
    app.tx = Some(tx);
    app.query.account = Some("b".into());
    (app, rx)
}

#[tokio::test]
async fn late_workspace_cannot_hide_admission_or_restore_a_completed_or_removed_request() {
    let (mut app, mut commands) = fixture().await;
    let stale = app.workspace.clone();
    let _ = app.handle_folder_creation(Message::Open);
    let _ = app.handle_folder_creation(Message::Name("Receipts".into()));
    let _ = app.handle_folder_creation(Message::Submit);
    let Some(Command::AdmitFolderCreation(id, request)) = commands.recv().await else {
        panic!("admission");
    };
    let job = admitted(id.clone(), &request);
    app.creation_admitted(request.serial, id, Ok(job.clone()));
    let _ = app.handle(super::super::Message::Backend(Event::Workspace(stale)));
    assert_eq!(app.workspace.creation_jobs, vec![job.clone()]);
    let stale = app.workspace.clone();
    let mut done = job.clone();
    done.stage = crate::store::CreationStage::Succeeded;
    done.revision += 1;
    app.creation_changed(done);
    let _ = app.handle(super::super::Message::Backend(Event::Workspace(
        stale.clone(),
    )));
    assert!(app.workspace.creation_jobs.is_empty());
    app.removal.target = Some(crate::store::ConnectionRef {
        kind: crate::store::ConnectionKind::Account,
        id: job.account.clone(),
    });
    app.removal.removing = Some(12);
    app.connection_removed(12, Ok(0));
    let mut late = job;
    late.revision += 10;
    app.creation_changed(late);
    let _ = app.handle(super::super::Message::Backend(Event::Workspace(stale)));
    assert!(app.workspace.creation_jobs.is_empty());
}

#[tokio::test]
async fn saved_admission_closes_form_and_projects_only_a_logical_sidebar_entry() {
    let (mut app, mut commands) = fixture().await;
    let _ = app.handle_folder_creation(Message::Open);
    let _ = app.handle_folder_creation(Message::Name("Receipts".into()));
    let _ = app.handle_folder_creation(Message::Submit);
    let Some(Command::AdmitFolderCreation(id, request)) = commands.recv().await else {
        panic!("admit");
    };
    let job = crate::store::CreationJob {
        id: id.clone(),
        account: request.account.clone(),
        connection: request.connection.clone(),
        parent: None,
        name: request.name.clone(),
        stage: crate::store::CreationStage::Queued,
        target: None,
        receipt: None,
        provider_acknowledged: false,
        error: None,
        revision: 1,
    };
    app.creation_admitted(request.serial, id, Ok(job.clone()));
    assert!(app.dialog.is_none());
    assert!(!app.folder_creation.busy);
    assert!(app.sidebar_items().iter().any(|item|item.label=="Receipts · Saved" && matches!(&item.action,super::super::Message::FolderCreation(Message::Review(shown)) if shown==&job.id)));
    assert!(
        app.workspace
            .account_folders
            .values()
            .all(|folders| !folders.contains(&"Receipts".into()))
    );
    assert!(matches!(commands.recv().await, Some(Command::BulkRun(_))));
    let _ = app.handle_folder_creation(Message::Open);
    let _ = app.handle_folder_creation(Message::Name("Newer folder".into()));
    let mut failed = job.clone();
    failed.stage = crate::store::CreationStage::Rejected;
    failed.error = Some("NO".into());
    failed.revision += 1;
    app.creation_changed(failed.clone());
    assert_eq!(app.folder_creation.name, "Newer folder");
    app.creation_changed(job);
    assert_eq!(app.workspace.creation_jobs[0], failed);
}

#[tokio::test]
async fn visible_sidebar_action_uses_selected_account_and_rejects_blank_names() {
    let (mut app, mut commands) = fixture().await;
    assert!(
        app.sidebar_items()
            .iter()
            .any(|item| item.label == "New folder")
    );
    let _ = app.handle_folder_creation(Message::Open);
    assert_eq!(app.folder_creation.account, "b");
    let _ = app.handle_folder_creation(Message::Submit);
    assert!(app.folder_creation.error.is_some());
    assert!(commands.try_recv().is_err());
    assert!(!app.folder_creation.busy);
}

#[tokio::test]
async fn creation_keeps_explicit_account_parent_and_name_through_error_retry() {
    let (mut app, mut commands) = fixture().await;
    let _ = app.handle_folder_creation(Message::Open);
    let _ = app.handle_folder_creation(Message::Account(Choice("a".into(), "A".into())));
    let _ = app.handle_folder_creation(Message::Parent(Choice(
        "Projects".into(),
        "Projects".into(),
    )));
    let _ = app.handle_folder_creation(Message::Name("  Receipts  ".into()));
    let _ = app.handle_folder_creation(Message::Submit);
    let Some(Command::AdmitFolderCreation(id, first)) = commands.recv().await else {
        panic!("creation command")
    };
    assert_eq!(first.account, "a");
    assert_eq!(first.parent.as_deref(), Some("Projects"));
    assert_eq!(first.name, "Receipts");
    assert!(app.has_required_close_work());
    app.creation_admitted(first.serial, id, Err("Local save failed".into()));
    assert!(!app.folder_creation.busy);
    assert_eq!(app.folder_creation.name, "  Receipts  ");
    let _ = app.handle_folder_creation(Message::Submit);
    let Some(Command::AdmitFolderCreation(_, second)) = commands.recv().await else {
        panic!("retry command")
    };
    assert_eq!(first.account, second.account);
    assert_eq!(first.parent, second.parent);
    assert_eq!(first.name, second.name);
    assert!(second.serial > first.serial);
}

#[tokio::test]
async fn closing_dialog_keeps_pending_creation_and_late_completion_preserves_navigation() {
    let (mut app, mut commands) = fixture().await;
    let _ = app.handle_folder_creation(Message::Open);
    let _ = app.handle_folder_creation(Message::Name("Receipts".into()));
    let _ = app.handle_folder_creation(Message::Submit);
    let Some(Command::AdmitFolderCreation(id, request)) = commands.recv().await else {
        panic!("creation command")
    };
    let _ = app.handle(super::super::Message::Close);
    assert!(app.folder_creation.busy);
    app.dialog = Some(Dialog::Move);
    app.query.folder = "Archive".into();
    app.creation_admitted(
        request.serial.wrapping_sub(1),
        id.clone(),
        Ok(admitted(id.clone(), &request)),
    );
    assert!(app.folder_creation.busy);
    app.creation_admitted(request.serial, id.clone(), Ok(admitted(id, &request)));
    assert!(!app.folder_creation.busy);
    assert_eq!(app.dialog, Some(Dialog::Move));
    assert_eq!(app.query.folder, "Archive");
}

#[tokio::test]
async fn legacy_reopening_restores_saved_request_and_new_name_does_not_replace_it() {
    let (mut app, mut commands) = fixture().await;
    let account = &app.workspace.accounts[1];
    let saved = crate::store::PendingCreation {
        account: account.id.clone(),
        connection: crate::mail_actions::connection_key(account),
        parent: Some("Home.Plans".into()),
        name: "Receipts".into(),
    };
    Arc::make_mut(&mut app.workspace)
        .folder_creations
        .push(saved.clone());
    let _ = app.handle_folder_creation(Message::Open);
    assert_eq!(app.folder_creation.name, "Receipts");
    assert_eq!(app.folder_creation.parent, "Home.Plans");
    assert!(app.folder_creation.error.is_some());
    let _ = app.handle_folder_creation(Message::Name("Something else".into()));
    assert_eq!(app.workspace.folder_creations, vec![saved.clone()]);
    let _ = app.handle_folder_creation(Message::Resume(saved.clone()));
    let _ = app.handle_folder_creation(Message::Submit);
    let Some(Command::AdmitFolderCreation(_, request)) = commands.recv().await else {
        panic!("saved request")
    };
    assert_eq!(request.connection, saved.connection);
    assert_eq!(request.name, saved.name);
    assert_eq!(request.parent, saved.parent);
}

#[tokio::test]
async fn account_change_cannot_silently_rebind_a_saved_request() {
    let (mut app, mut commands) = fixture().await;
    let _ = app.handle_folder_creation(Message::Open);
    let original = app.folder_creation.connection.clone();
    Arc::make_mut(&mut app.workspace).accounts[1].host = "changed.example.test".into();
    let _ = app.handle_folder_creation(Message::Name("Receipts".into()));
    let _ = app.handle_folder_creation(Message::Submit);
    let Some(Command::AdmitFolderCreation(_, request)) = commands.recv().await else {
        panic!("bound request")
    };
    assert_eq!(request.connection, original);
    assert_ne!(
        request.connection,
        crate::mail_actions::connection_key(&app.workspace.accounts[1])
    );
}

#[tokio::test]
async fn repeated_window_close_waits_for_creation_even_after_bulk_has_stopped() {
    for failed in [false, true] {
        let (mut app, mut commands) = fixture().await;
        let _ = app.handle_folder_creation(Message::Open);
        let _ = app.handle_folder_creation(Message::Name("Receipts".into()));
        let _ = app.handle_folder_creation(Message::Submit);
        let Some(Command::AdmitFolderCreation(id, request)) = commands.recv().await else {
            panic!("creation")
        };
        app.bulk.stopped = true;
        let window = iced::window::Id::unique();
        for _ in 0..2 {
            let _ = app.update(super::super::Message::WindowClose(window));
            assert_eq!(app.pending_close, Some(window));
            assert!(app.folder_creation.busy);
            assert!(!app.tray.exiting);
        }
        let result = if failed {
            Err("Creation admission could not be saved".into())
        } else {
            Ok(admitted(id.clone(), &request))
        };
        let _ = app.update(super::super::Message::Backend(Event::CreationAdmitted(
            request.serial,
            id,
            result,
        )));
        assert!(!app.folder_creation.busy);
        assert!(app.pending_close.is_none());
        assert_eq!(app.tray.exiting, !failed);
        if failed {
            assert_eq!(app.dialog, Some(Dialog::FolderCreation));
            assert!(app.folder_creation.error.is_some());
        }
    }
}
