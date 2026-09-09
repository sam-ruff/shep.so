use super::super::tests::{fixture, prepare};
use super::*;
use crate::{
    folder_actions::Outcome,
    store::{MailSelectionId, Store},
};

fn choice(account: &str, folder: &str) -> FolderSelection {
    FolderSelection {
        account: Some(account.into()),
        folder: folder.into(),
        sent_only: false,
    }
}
async fn combined_fixture() -> (App, Store, tokio::sync::mpsc::Receiver<Command>) {
    let (mut app, store, commands) = fixture().await;
    let mut messages = Vec::new();
    for index in 0..80 {
        messages.push(
            parse_mail(
                "a",
                &format!("42.{index}"),
                "Projects",
                format!("Subject: Delete {index:03}\r\n\r\nFictional cached letter").into_bytes(),
                index % 2 == 0,
                index % 2 == 0,
            )
            .unwrap(),
        );
    }
    for (account, folder, subject, unread) in [
        ("a", "Projects/Design", "Delete child", true),
        ("a", "Teams", "A retained reader", false),
        ("b", "Projects", "B retained account", true),
    ] {
        messages.push(
            parse_mail(
                account,
                "42.90",
                folder,
                format!("Subject: {subject}\r\n\r\nKeep this complete cached body").into_bytes(),
                unread,
                true,
            )
            .unwrap(),
        );
    }
    store.upsert(messages).await.unwrap();
    app.query.account = None;
    app.query.folder.clear();
    app.query.sort = MailSort::Subject;
    app.query.folders = Some(vec![
        choice("a", "Projects"),
        choice("a", "Projects/Design"),
        choice("a", "Teams"),
        choice("b", "Projects"),
    ]);
    app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
    let reader = app
        .page
        .rows
        .iter()
        .find(|mail| mail.subject == "A retained reader")
        .unwrap()
        .id
        .clone();
    app.selected = Some(reader.clone());
    app.detail = Some(Arc::new(store.detail(reader).await.unwrap()));
    (app, store, commands)
}

#[tokio::test]
async fn combined_delete_projects_all_counts_and_preserves_reader_through_cache_receipt_ordering() {
    for outcome in [
        Outcome::Applied,
        Outcome::Rejected("Rejected deletion".into()),
        Outcome::Uncertain("Lost acknowledgment".into()),
    ] {
        let (mut app, store, mut commands) = combined_fixture().await;
        assert_eq!(
            (app.page.total, app.page.unread, app.page.rows.len()),
            (83, 42, 50)
        );
        let reader = app.selected.clone();
        let body = app.detail.as_ref().unwrap().body.clone();
        let original_query = app.query.clone();
        let original_page = app.page.clone();
        let original_generation = app.generation;
        let preview = prepare(&mut app, &store, Change::Delete).await;
        app.handle_folders(Message::Submit);
        let Command::Folder(Request::Start(id, _, _)) = commands.try_recv().unwrap() else {
            panic!("Expected folder delete");
        };
        // This is the optimistic update, before staging or any cache/provider write.
        assert_eq!(
            (app.page.total, app.page.unread, app.page.rows.len()),
            (2, 1, 2)
        );
        assert_eq!(
            app.query.folders.as_ref().unwrap(),
            &vec![choice("a", "Teams"), choice("b", "Projects")]
        );
        assert_eq!(app.selected, reader);
        assert_eq!(app.detail.as_ref().unwrap().body, body);
        let _ = app.handle(super::super::super::Message::Backend(engine::Event::Page(
            original_generation,
            original_page,
            false,
        )));
        assert_eq!(
            app.page.total, 2,
            "Old reads cannot restore the deleted scope"
        );
        let job = store
            .start_folder_change_scoped(
                id.clone(),
                (*preview.review).clone(),
                preview.projection.as_ref().map(|(token, _)| token.clone()),
            )
            .await
            .unwrap();
        app.folder_event(FolderEvent::Started(id.clone(), Ok(Arc::new(job))));
        let captured = store
            .capture_selection(
                MailSelectionId::default(),
                1,
                app.query.clone(),
                true,
                vec![],
            )
            .await
            .unwrap();
        assert_eq!(
            captured.selected, 2,
            "Select All must use the visible combined scope"
        );
        let lease = store.folder_lease(id.clone()).await.unwrap();
        while let Some(step) = store.claim_folder_step(&lease).await.unwrap() {
            let pending = store.query(app.query.clone()).await.unwrap();
            assert_eq!((pending.total, pending.unread), (2, 1));
            store
                .record_folder_outcome(&lease, step.position, outcome.clone())
                .await
                .unwrap();
            if outcome != Outcome::Applied {
                break;
            }
            store
                .commit_folder_step(&lease, step.position)
                .await
                .unwrap();
            app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
            assert_eq!(
                (app.page.total, app.page.unread),
                (2, 1),
                "Committed cache pages cannot apply the deletion twice"
            );
        }
        let mut job = store.folder_job(id.clone()).await.unwrap();
        job.query_counts = store.folder_projection_counts(id.clone()).await.unwrap();
        app.folder_event(FolderEvent::Finished(id.clone(), Ok(Arc::new(job))));
        let expected = if outcome == Outcome::Applied {
            (2, 1)
        } else {
            (83, 42)
        };
        assert_eq!((app.page.total, app.page.unread), expected);
        assert!(app.query.exclude_folders.is_empty());
        assert_eq!(app.selected, reader);
        assert_eq!(app.detail.as_ref().unwrap().body, body);
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        assert_eq!((app.page.total, app.page.unread), expected);
        if matches!(outcome, Outcome::Uncertain(_)) {
            assert!(store.stop_folder_change(&lease, false).await.is_err());
            let job = store.stop_folder_change(&lease, true).await.unwrap();
            app.folder_event(FolderEvent::Finished(id, Ok(Arc::new(job))));
            assert_eq!((app.page.total, app.page.unread), (83, 42));
            assert_eq!(store.query(original_query).await.unwrap().total, 83);
            assert!(
                app.notice
                    .as_ref()
                    .unwrap()
                    .0
                    .contains("Unconfirmed cached mail is kept")
            );
        }
    }
}

#[tokio::test]
async fn combined_delete_rejection_preserves_newer_folder_choice_and_reader() {
    let (mut app, store, mut commands) = combined_fixture().await;
    prepare(&mut app, &store, Change::Delete).await;
    app.handle_folders(Message::Submit);
    let Command::Folder(Request::Start(id, _, _)) = commands.try_recv().unwrap() else {
        panic!();
    };
    app.toggle_folder_selection(choice("a", "Teams"));
    app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
    let reader = app.page.rows[0].id.clone();
    app.selected = Some(reader.clone());
    app.detail = Some(Arc::new(store.detail(reader.clone()).await.unwrap()));
    app.folder_event(FolderEvent::Started(id, Err("Changed review".into())));
    assert_eq!(
        app.query.folders.as_ref().unwrap(),
        &vec![choice("b", "Projects")]
    );
    assert_eq!((app.page.total, app.page.unread), (1, 1));
    assert_eq!(app.selected.as_deref(), Some(reader.as_str()));
    assert_eq!(app.detail.as_ref().unwrap().summary.id, reader);
    assert!(app.query.exclude_folders.is_empty());
}

#[tokio::test]
async fn combined_delete_excludes_exact_account_folders_in_search_and_capture() {
    let (mut app, store, mut commands) = combined_fixture().await;
    app.query.folders = Some(vec![choice("a", "Projects"), choice("b", "Projects")]);
    app.query.search_all_folders = true;
    app.query.search = "cached".into();
    app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
    assert_eq!(app.page.total, 83);
    prepare(&mut app, &store, Change::Delete).await;
    app.handle_folders(Message::Submit);
    let Command::Folder(Request::Start(id, _, _)) = commands.try_recv().unwrap() else {
        panic!();
    };
    assert_eq!((app.page.total, app.page.unread), (2, 1));
    let page = store.query(app.query.clone()).await.unwrap();
    assert_eq!((page.total, page.unread), (2, 1));
    let selected = store
        .capture_selection(
            MailSelectionId::default(),
            1,
            app.query.clone(),
            true,
            vec![],
        )
        .await
        .unwrap();
    assert_eq!(selected.selected, 2);
    assert!(
        selected
            .groups
            .iter()
            .all(|group| group.account == "b" || group.folder == "Teams")
    );
    app.folder_event(FolderEvent::Started(id, Err("Changed review".into())));
    assert_eq!(store.query(app.query.clone()).await.unwrap().total, 83);
}

async fn receive_page(app: &mut App, store: &Store) {
    let mut query = app.query.clone();
    query.observe = app.selected.iter().cloned().collect();
    let page = Arc::new(store.query(query).await.unwrap());
    let _ = app.handle(super::super::super::Message::Backend(engine::Event::Page(
        app.generation,
        page,
        false,
    )));
}

#[tokio::test]
async fn combined_delete_failure_keeps_newer_reader_beyond_restored_first_page() {
    let (mut app, store, mut commands) = combined_fixture().await;
    // Deleted mail sorts before the surviving reader, outside the original 50.
    app.query.sort = MailSort::Subject;
    let mut additional = Vec::new();
    for index in 0..60 {
        additional.push(
            parse_mail(
                "a",
                &format!("42.late{index}"),
                "Teams",
                format!("Subject: Z retained {index:03}\r\n\r\nRetained complete body")
                    .into_bytes(),
                false,
                false,
            )
            .unwrap(),
        );
    }
    store.upsert(additional).await.unwrap();
    app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
    prepare(&mut app, &store, Change::Delete).await;
    app.handle_folders(Message::Submit);
    let Command::Folder(Request::Start(id, _, _)) = commands.try_recv().unwrap() else {
        panic!()
    };
    receive_page(&mut app, &store).await;
    let reader = app
        .page
        .rows
        .iter()
        .find(|m| m.subject == "Z retained 010")
        .unwrap()
        .id
        .clone();
    app.select(reader.clone());
    app.detail = Some(Arc::new(store.detail(reader.clone()).await.unwrap()));
    app.folder_event(FolderEvent::Started(id, Err("Definite rejection".into())));
    receive_page(&mut app, &store).await;
    assert!(!app.page.rows.iter().any(|m| m.id == reader));
    assert_eq!(app.selected.as_ref(), Some(&reader));
    assert_eq!(app.detail.as_ref().unwrap().summary.id, reader);
    // A subsequent refresh keeps that reader, but actual scope navigation wins.
    app.request_page();
    receive_page(&mut app, &store).await;
    assert_eq!(app.selected.as_ref(), Some(&reader));
    app.query.folders = Some(vec![choice("b", "Projects")]);
    app.request_page();
    receive_page(&mut app, &store).await;
    assert_ne!(app.selected.as_ref(), Some(&reader));
}

#[tokio::test]
async fn combined_delete_partial_rejection_restores_only_uncommitted_members() {
    let (mut app, store, mut commands) = combined_fixture().await;
    let preview = prepare(&mut app, &store, Change::Delete).await;
    app.handle_folders(Message::Submit);
    let Command::Folder(Request::Start(id, _, _)) = commands.try_recv().unwrap() else {
        panic!()
    };
    store
        .start_folder_change_scoped(
            id.clone(),
            (*preview.review).clone(),
            preview.projection.as_ref().map(|(token, _)| token.clone()),
        )
        .await
        .unwrap();
    let lease = store.folder_lease(id.clone()).await.unwrap();
    let child = store.claim_folder_step(&lease).await.unwrap().unwrap();
    assert!(matches!(&child.step, Step::Delete { source } if source == "Projects/Design"));
    store
        .record_folder_outcome(&lease, child.position, Outcome::Applied)
        .await
        .unwrap();
    store
        .commit_folder_step(&lease, child.position)
        .await
        .unwrap();
    let parent = store.claim_folder_step(&lease).await.unwrap().unwrap();
    store
        .record_folder_outcome(
            &lease,
            parent.position,
            Outcome::Rejected("Parent retained".into()),
        )
        .await
        .unwrap();
    let mut job = store.folder_job(id.clone()).await.unwrap();
    job.query_counts = store.folder_projection_counts(id.clone()).await.unwrap();
    app.folder_event(FolderEvent::Finished(id, Ok(Arc::new(job))));
    assert_eq!((app.page.total, app.page.unread), (82, 41));
    assert_eq!(
        app.query.folders.as_ref().unwrap(),
        &vec![
            choice("a", "Projects"),
            choice("a", "Teams"),
            choice("b", "Projects")
        ]
    );
    receive_page(&mut app, &store).await;
    assert_eq!((app.page.total, app.page.unread), (82, 41));
}

#[tokio::test]
async fn combined_delete_after_flag_and_move_receipts_uses_updated_group_counts() {
    let (mut app, store, _) = combined_fixture().await;
    let (tx, mut network) = engine::CommandSender::network_test_channel();
    app.tx = Some(tx);
    let original = app
        .page
        .rows
        .iter()
        .find(|m| m.subject == "Delete 000")
        .unwrap()
        .clone();
    app.toggle_mail_flag(original.clone(), true);
    let Command::Flags(request, sent, _) = network.try_recv().unwrap() else {
        panic!()
    };
    store.flags(sent.clone()).await.unwrap();
    let _ = app.flags_finished(request, sent, Ok(()));
    assert_eq!(app.mail_actions.base_page.unread, 41);
    // Reconcile an acknowledged source row into another selected folder before
    // the next cache page arrives, just as move/Undo receipts do.
    let original = app
        .page
        .rows
        .iter()
        .find(|m| m.subject == "Delete 001")
        .unwrap()
        .clone();
    let mut current = original.clone();
    store
        .move_local(original.id.clone(), "Teams".into())
        .await
        .unwrap();
    current.folder = "Teams".into();
    app.move_mail(original.clone(), "Teams".into());
    let Command::Move(request, sent, destination) = network.try_recv().unwrap() else {
        panic!()
    };
    let _ = app.move_receipt(
        request,
        sent,
        destination,
        Ok(Arc::new(crate::mail_actions::MoveReceipt {
            current: Some(current.clone()),
            ..crate::mail_actions::MoveReceipt::local(&original, "Teams")
        })),
    );
    let (tx, _folder_commands) = engine::CommandSender::selection_test_channel();
    app.tx = Some(tx);
    prepare(&mut app, &store, Change::Delete).await;
    app.handle_folders(Message::Submit);
    assert_eq!((app.page.total, app.page.unread), (3, 1));
    assert!(app.page.rows.iter().any(|m| m.id == current.id));
}

#[tokio::test]
async fn combined_delete_splits_aggregate_membership_without_removing_other_account() {
    let (mut app, store, mut commands) = combined_fixture().await;
    let aggregate = FolderSelection {
        account: None,
        folder: "Projects".into(),
        sent_only: false,
    };
    app.query.folders = Some(vec![aggregate.clone(), choice("a", "Teams")]);
    app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
    assert_eq!((app.page.total, app.page.unread), (82, 41));
    prepare(&mut app, &store, Change::Delete).await;
    app.handle_folders(Message::Submit);
    let Command::Folder(Request::Start(id, _, _)) = commands.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(
        app.query.folders,
        Some(vec![choice("b", "Projects"), choice("a", "Teams")])
    );
    assert_eq!((app.page.total, app.page.unread), (2, 1));
    app.folder_event(FolderEvent::Started(id, Err("Definite rejection".into())));
    assert_eq!(
        app.query.folders,
        Some(vec![aggregate, choice("a", "Teams")])
    );
    assert_eq!((app.page.total, app.page.unread), (82, 41));
}

#[tokio::test]
async fn combined_delete_rejection_does_not_restore_newer_removed_unaffected_membership() {
    let (mut app, store, mut commands) = combined_fixture().await;
    prepare(&mut app, &store, Change::Delete).await;
    app.handle_folders(Message::Submit);
    let Command::Folder(Request::Start(id, _, _)) = commands.try_recv().unwrap() else {
        panic!()
    };
    let original = app
        .page
        .rows
        .iter()
        .find(|m| m.account_id == "b")
        .unwrap()
        .clone();
    let (tx, mut network) = engine::CommandSender::network_test_channel();
    app.tx = Some(tx);
    app.move_mail(original.clone(), "Archive".into());
    let Command::Move(request, sent, destination) = network.try_recv().unwrap() else {
        panic!()
    };
    store
        .move_local(original.id.clone(), "Archive".into())
        .await
        .unwrap();
    let _ = app.move_receipt(
        request,
        sent,
        destination,
        Ok(Arc::new(crate::mail_actions::MoveReceipt::local(
            &original, "Archive",
        ))),
    );
    assert_eq!((app.page.total, app.page.unread), (1, 0));
    app.folder_event(FolderEvent::Started(id, Err("Definite rejection".into())));
    assert_eq!((app.page.total, app.page.unread), (82, 41));
    assert!(!app.page.rows.iter().any(|m| m.id == original.id));
    receive_page(&mut app, &store).await;
    assert_eq!((app.page.total, app.page.unread), (82, 41));
}

#[tokio::test]
async fn combined_delete_refreshes_scalar_when_a_flag_receipt_follows_review() {
    let (mut app, store, _) = combined_fixture().await;
    let original = app
        .page
        .rows
        .iter()
        .find(|mail| mail.subject == "Delete 000")
        .unwrap()
        .clone();
    let (tx, mut network) = engine::CommandSender::network_test_channel();
    app.tx = Some(tx);
    app.toggle_mail_flag(original, true);
    let Command::Flags(request, sent, _) = network.try_recv().unwrap() else {
        panic!()
    };
    let old = prepare(&mut app, &store, Change::Delete).await;
    assert_eq!(app.mail_actions.base_page.folder_count, Some((81, 41)));
    store.flags(sent.clone()).await.unwrap();
    let _ = app.flags_finished(request, sent, Ok(()));
    assert_eq!(app.mail_actions.base_page.folder_count, None);
    let (tx, mut reads) = engine::CommandSender::foreground_test_channel();
    app.tx = Some(tx);
    app.handle_folders(Message::Submit);
    assert!(app.folder_controls.pending.is_empty());
    assert!(app.folder_controls.loading);
    assert!(matches!(
        reads.try_recv(),
        Ok(Command::Folder(Request::Release(_)))
    ));
    assert!(matches!(
        reads.try_recv(),
        Ok(Command::Folder(Request::Review(..)))
    ));
    store
        .release_folder_projection(old.projection.as_ref().unwrap().0.clone())
        .await
        .unwrap();
    let (tx, mut commands) = engine::CommandSender::selection_test_channel();
    app.tx = Some(tx);
    app.folder_controls.loading = false;
    prepare(&mut app, &store, Change::Delete).await;
    app.handle_folders(Message::Submit);
    assert!(matches!(
        commands.try_recv(),
        Ok(Command::Folder(Request::Start(..)))
    ));
    assert_eq!((app.page.total, app.page.unread), (2, 1));
}
