use super::*;
use crate::{
    folders::{Mailbox, NameEncoding},
    store::Store,
};

pub(super) async fn fixture() -> (App, Store, tokio::sync::mpsc::Receiver<Command>) {
    let store = Store::memory().unwrap();
    for account in ["a", "b"] {
        let config:Account=serde_json::from_value(serde_json::json!({"id":account,"name":account,"email":format!("{account}@example.test"),"protocol":"Imap","host":"localhost","port":993,"username":account,"smtp_host":"localhost","smtp_port":465})).unwrap();
        store.save_account(config).await.unwrap();
        store
            .save_folder_catalog(
                account.into(),
                ["INBOX", "Projects", "Projects/Design", "Archive", "Teams"]
                    .into_iter()
                    .map(|name| Mailbox {
                        delimiter: Some('/'),
                        encoding: NameEncoding::Utf8,
                        ..Mailbox::flat(name.into())
                    })
                    .collect(),
            )
            .await
            .unwrap();
    }
    let (tx, commands) = engine::CommandSender::selection_test_channel();
    let (mut app, _) = App::new();
    app.tx = Some(tx);
    app.workspace = Arc::new(store.workspace().await.unwrap());
    app.query.account = Some("a".into());
    app.query.folder = "Projects".into();
    (app, store, commands)
}
pub(super) async fn prepare(app: &mut App, store: &Store, action: Change) -> Arc<Preview> {
    let review = store
        .folder_review("a".into(), "Projects".into(), action.clone())
        .await
        .unwrap();
    let catalog = store.current_folder_catalog("a".into()).await.unwrap();
    let tree = crate::folders::Tree::new(&review.plan.project(&catalog, &review.plan.steps()));
    let originals = review
        .plan
        .members
        .iter()
        .filter_map(|m| {
            Some((
                crate::folder_actions::Plan::wire_destination(m)?,
                m.mailbox.name.clone(),
            ))
        })
        .collect();
    let projection = if action == Change::Delete {
        let token = uuid::Uuid::new_v4().to_string();
        let page = Arc::new(
            store
                .query_folder_projection(
                    app.folder_count_query(),
                    Some((token.clone(), app.generation, Arc::new(review.clone()))),
                )
                .await
                .unwrap(),
        );
        app.set_mail_page(page.clone());
        Some((token, page))
    } else {
        None
    };
    let preview = Arc::new(Preview {
        generation: app.generation,
        projection,
        review: Arc::new(review),
        tree: Arc::new(tree),
        originals,
    });
    app.dialog = Some(Dialog::FolderChange);
    app.folder_controls.action = Some(action);
    app.folder_controls.preview = Some(preview.clone());
    preview
}
#[tokio::test]
async fn move_projection_browses_original_cache_and_rolls_back_without_waiting() {
    let (mut app, store, mut commands) = fixture().await;
    prepare(
        &mut app,
        &store,
        Change::Move {
            parent: Some("Archive".into()),
        },
    )
    .await;
    app.handle_folders(Message::Submit);
    let Command::Folder(Request::Start(id, _, _)) = commands.try_recv().unwrap() else {
        panic!("Wrong queue")
    };
    assert!(app.folder_tree("a").unwrap().node("Projects").is_none());
    assert!(
        app.folder_tree("a")
            .unwrap()
            .node("Archive/Projects/Design")
            .is_some()
    );
    assert_eq!(app.original_folder("a", "Archive/Projects"), "Projects");
    assert_eq!(app.query.folder, "Projects");
    assert!(app.notice.as_ref().unwrap().0.contains("Moving"));
    assert!(app.folder_staging());
    app.folder_event(FolderEvent::Started(
        id,
        Err("Rejected before staging".into()),
    ));
    assert!(!app.folder_staging());
    assert!(app.folder_tree("a").unwrap().node("Projects").is_some());
    assert!(app.notice.as_ref().unwrap().1);
}
#[tokio::test]
async fn rejected_delete_restores_origin_but_preserves_later_account_navigation() {
    for navigate in [false, true] {
        let (mut app, store, mut commands) = fixture().await;
        prepare(&mut app, &store, Change::Delete).await;
        app.handle_folders(Message::Submit);
        let Command::Folder(Request::Start(id, _, _)) = commands.try_recv().unwrap() else {
            panic!()
        };
        assert_eq!(app.query.folder, "INBOX");
        assert!(app.folder_tree("a").unwrap().node("Projects").is_none());
        if navigate {
            app.query.account = Some("b".into());
            app.query.folder = "Teams".into();
        }
        app.folder_event(FolderEvent::Started(id, Err("Changed review".into())));
        assert_eq!(
            app.query.folder,
            if navigate { "Teams" } else { "Projects" }
        );
    }
}
#[tokio::test]
async fn stale_review_and_history_cannot_replace_newer_input_or_receipts() {
    let (mut app, store, _) = fixture().await;
    let preview = prepare(&mut app, &store, Change::Delete).await;
    app.folder_controls.serial = 3;
    app.folder_controls.preview = None;
    app.folder_controls.loading = true;
    app.folder_event(FolderEvent::Review(2, Ok(preview.clone())));
    assert!(app.folder_controls.preview.is_none());
    assert!(app.folder_controls.loading);
    let old = store
        .start_folder_change("job".into(), (*preview.review).clone())
        .await
        .unwrap();
    let mut current = old.clone();
    current.revision += 1;
    current.steps[0].status = Status::Done;
    app.observe_folder_job(Arc::new(current.clone()));
    app.folder_event(FolderEvent::History(0, Ok(Arc::new(vec![old]))));
    assert_eq!(app.folder_controls.jobs[0].revision, current.revision);
    assert_eq!(app.folder_controls.jobs[0].steps[0].status, Status::Done);
    assert!(
        app.folder_controls.loading,
        "A history reply must not unlock a pending folder review"
    );
}
#[tokio::test]
async fn folder_destination_exact_match_wins_and_enter_only_opens_review() {
    let (mut app, _, _commands) = fixture().await;
    let (tx, mut reads) = engine::CommandSender::foreground_test_channel();
    app.tx = Some(tx);
    app.dialog = Some(Dialog::FolderChange);
    app.folder_controls.account = "a".into();
    app.folder_controls.source = "Projects".into();
    app.folder_controls.options = Arc::new(vec![
        Destination {
            path: Some("Archive/Old".into()),
            label: "Archive/Old".into(),
        },
        Destination {
            path: Some("Archive".into()),
            label: "Archive".into(),
        },
    ]);
    app.handle_folders(Message::Query("archive".into()));
    assert_eq!(app.folder_controls.filtered.first(), Some(&1));
    app.handle_folders(Message::Submit);
    assert!(app.folder_controls.loading);
    assert_eq!(
        app.folder_controls.action,
        Some(Change::Move {
            parent: Some("Archive".into())
        })
    );
    assert!(
        app.folder_controls.pending.is_empty(),
        "The destination's Enter must not skip the confirmation review"
    );
    assert!(
        matches!(reads.try_recv(), Ok(Command::Folder(Request::Review(..)))),
        "Review belongs on the read queue"
    );
}

#[tokio::test]
async fn accepting_uncertainty_stops_work_without_claiming_the_folder_was_deleted() {
    let (mut app, store, _) = fixture().await;
    let preview = prepare(&mut app, &store, Change::Delete).await;
    let mut job = store
        .start_folder_change("uncertain".into(), (*preview.review).clone())
        .await
        .unwrap();
    job.closed = true;
    job.steps[0].status = Status::Accepted;
    job.steps[1].status = Status::Cancelled;
    app.folder_event(FolderEvent::Finished(job.id.clone(), Ok(Arc::new(job))));
    let notice = &app.notice.as_ref().unwrap().0;
    assert!(notice.contains("stopped") && notice.contains("Unconfirmed"));
    assert!(!notice.contains("deleted") && !notice.contains("moved"));
}

#[tokio::test]
async fn folder_rollback_does_not_steal_a_later_visit_to_the_same_inbox() {
    let (mut app, store, mut commands) = fixture().await;
    prepare(&mut app, &store, Change::Delete).await;
    app.handle_folders(Message::Submit);
    let Command::Folder(Request::Start(id, _, _)) = commands.try_recv().unwrap() else {
        panic!()
    };
    app.query.folder = "Teams".into();
    app.request_page();
    app.query.folder = "INBOX".into();
    app.request_page();
    app.folder_event(FolderEvent::Started(id, Err("Changed review".into())));
    assert_eq!(app.query.folder, "INBOX");
}

#[tokio::test]
async fn folder_recovery_handlers_cannot_bypass_uncertain_or_inflight_guards() {
    let (mut app, store, mut commands) = fixture().await;
    let preview = prepare(&mut app, &store, Change::Delete).await;
    let mut job = store
        .start_folder_change("uncertain".into(), (*preview.review).clone())
        .await
        .unwrap();
    job.steps[0].status = Status::Uncertain;
    app.folder_controls.selected = Some(job.id.clone());
    app.folder_controls.jobs = Arc::new(vec![job]);
    app.dialog = Some(Dialog::FolderHistory);
    app.handle_folders(Message::Retry);
    app.handle_folders(Message::Stop);
    assert!(commands.try_recv().is_err());
    app.handle_folders(Message::Accept(true));
    app.handle_folders(Message::Stop);
    assert!(matches!(
        commands.try_recv(),
        Ok(Command::Folder(Request::Stop(_, true)))
    ));
    app.folder_event(FolderEvent::History(
        0,
        Ok(app.folder_controls.jobs.clone()),
    ));
    app.handle_folders(Message::Stop);
    assert!(
        commands.try_recv().is_err(),
        "An old history response must not unlock the active request"
    );
}

#[tokio::test]
async fn stale_workspace_cannot_restore_a_committed_folder_tree() {
    let (mut app, store, _) = fixture().await;
    let before = app.workspace.clone();
    let preview = prepare(
        &mut app,
        &store,
        Change::Move {
            parent: Some("Archive".into()),
        },
    )
    .await;
    store
        .start_folder_change("workspace-order".into(), (*preview.review).clone())
        .await
        .unwrap();
    let lease = store.folder_lease("workspace-order".into()).await.unwrap();
    store.claim_folder_step(&lease).await.unwrap();
    store
        .record_folder_outcome(&lease, 0, crate::folder_actions::Outcome::Applied)
        .await
        .unwrap();
    store.commit_folder_step(&lease, 0).await.unwrap();
    let after = Arc::new(store.workspace().await.unwrap());
    assert!(after.connections_revision > before.connections_revision);
    let _ = app.handle(super::super::Message::Backend(engine::Event::Workspace(
        after,
    )));
    let _ = app.handle(super::super::Message::Backend(engine::Event::Workspace(
        before,
    )));
    assert!(app.workspace.folder_trees["a"].node("Projects").is_none());
    assert!(
        app.workspace.folder_trees["a"]
            .node("Archive/Projects")
            .is_some()
    );
    assert!(app.workspace.account_folders["a"].contains(&"Archive/Projects".into()));
}

#[tokio::test]
async fn combined_folder_choices_browse_pending_sources_and_follow_only_committed_renames() {
    for success in [false, true] {
        let (mut app, store, mut commands) = fixture().await;
        let mail = [
            ("parent", "Projects"),
            ("child", "Projects/Design"),
            ("other", "Teams"),
        ]
        .into_iter()
        .map(|(id, folder)| {
            parse_mail(
                "a",
                id,
                folder,
                format!("Subject: {id}\r\n\r\nCached {id}").into_bytes(),
                true,
                false,
            )
            .unwrap()
        })
        .collect();
        store.upsert(mail).await.unwrap();
        app.toggle_folder_selection(FolderSelection {
            account: Some("a".into()),
            folder: "Projects/Design".into(),
            sent_only: false,
        });
        assert_eq!(store.query(app.query.clone()).await.unwrap().total, 2);
        let preview = prepare(
            &mut app,
            &store,
            Change::Move {
                parent: Some("Archive".into()),
            },
        )
        .await;
        app.handle_folders(Message::Submit);
        let Command::Folder(Request::Start(id, _, _)) = commands.try_recv().unwrap() else {
            panic!("Expected reviewed folder change");
        };
        let mut job = store
            .start_folder_change(id.clone(), (*preview.review).clone())
            .await
            .unwrap();
        app.folder_event(FolderEvent::Started(id.clone(), Ok(Arc::new(job.clone()))));
        let action = super::super::Message::AccountFolder("a".into(), "Archive/Projects".into());
        let choice = app.sidebar_folder(&action).unwrap();
        assert_eq!(
            choice.folder, "Projects",
            "Ctrl-click must query the same cached source as plain click"
        );
        assert!(app.sidebar_items().iter().any(|item| matches!(&item.action,
            super::super::Message::AccountFolder(account, folder)
                if account == "a" && folder == "Archive/Projects")
            && item.active));
        app.toggle_folder_selection(choice.clone());
        assert_eq!(app.query.folders.as_ref().unwrap().len(), 1);
        assert_eq!(store.query(app.query.clone()).await.unwrap().total, 1);
        app.toggle_folder_selection(choice);
        let page = store.query(app.query.clone()).await.unwrap();
        assert_eq!((page.total, page.unread), (2, 2));
        // A newer choice outside the changed subtree must survive its receipt.
        app.toggle_folder_selection(FolderSelection {
            account: Some("a".into()),
            folder: "Teams".into(),
            sent_only: false,
        });
        let lease = store.folder_lease(id.clone()).await.unwrap();
        store.claim_folder_step(&lease).await.unwrap();
        store
            .record_folder_outcome(
                &lease,
                0,
                if success {
                    crate::folder_actions::Outcome::Applied
                } else {
                    crate::folder_actions::Outcome::Rejected("Fixture rejected rename".into())
                },
            )
            .await
            .unwrap();
        if success {
            store.commit_folder_step(&lease, 0).await.unwrap();
        }
        job = store.folder_job(id.clone()).await.unwrap();
        app.folder_event(FolderEvent::Finished(id, Ok(Arc::new(job))));
        let folders = app.query.folders.as_ref().unwrap();
        let parent = if success {
            "Archive/Projects"
        } else {
            "Projects"
        };
        assert!(folders.iter().any(|f| f.folder == parent));
        assert!(
            folders
                .iter()
                .any(|f| f.folder == format!("{parent}/Design"))
        );
        assert!(folders.iter().any(|f| f.folder == "Teams"));
        let page = store.query(app.query.clone()).await.unwrap();
        assert_eq!((page.total, page.unread), (3, 3));
    }
}

#[tokio::test]
async fn delete_review_requires_current_page_and_cancel_releases_its_snapshot() {
    let (mut app, store, _) = fixture().await;
    let preview = prepare(&mut app, &store, Change::Delete).await;
    let token = preview.projection.as_ref().unwrap().0.clone();
    let (tx, mut reads) = engine::CommandSender::foreground_test_channel();
    app.tx = Some(tx);
    app.generation += 1;
    app.handle_folders(Message::Submit);
    assert!(app.folder_controls.pending.is_empty());
    assert!(app.folder_controls.preview.is_none());
    assert!(app.folder_controls.loading);
    assert!(
        matches!(reads.try_recv(), Ok(Command::Folder(Request::Release(value))) if value == token)
    );
    assert!(matches!(
        reads.try_recv(),
        Ok(Command::Folder(Request::Review(..)))
    ));
    app.folder_event(FolderEvent::Review(
        app.folder_controls.serial - 1,
        Ok(preview),
    ));
    assert!(app.folder_controls.preview.is_none());
    assert!(
        matches!(reads.try_recv(), Ok(Command::Folder(Request::Release(value))) if value == token)
    );
    store.release_folder_projection(token).await.unwrap();
    let current = prepare(&mut app, &store, Change::Delete).await;
    app.release_folder_preview();
    assert!(
        matches!(reads.try_recv(), Ok(Command::Folder(Request::Release(value))) if value == current.projection.as_ref().unwrap().0)
    );
}
