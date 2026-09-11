use super::*;
use crate::store::Store;

async fn destination(app: &mut App, store: &Store, account: &str, folder: &str) {
    app.query.account = Some(account.into());
    app.query.folder = folder.into();
    let mut query = app.query.clone();
    query.project_moves = app.mail_actions.projected_moves();
    query.observe = app.mail_actions.observed_ids();
    let page = Arc::new(store.query(query).await.unwrap());
    let _ = app.handle(Message::Backend(Event::Page(app.generation, page, false)));
}
async fn stored_original() -> crate::store::Store {
    let store = Store::memory().unwrap();
    store
        .upsert(vec![
            parse_mail(
                "fixture",
                "42.7",
                "INBOX",
                b"From: fixture@example.test\r\nSubject: Actions\r\n\r\nSelectable unchanged body"
                    .to_vec(),
                true,
                false,
            )
            .unwrap(),
        ])
        .await
        .unwrap();
    store
}

#[tokio::test]
async fn pending_destination_keeps_body_and_rekeys_selected_reader_only_after_ack() {
    for cross in [false, true] {
        let (mut app, mut commands, detail) = super::super::tests::fixture().await;
        let store = stored_original().await;
        let original = detail.summary.clone();
        app.cache_detail(detail.clone());
        let account = if cross { "personal" } else { "fixture" };
        if cross {
            app.transfer_mail(original.clone(), account.into(), "Keep".into());
        } else {
            app.move_mail(original.clone(), "Keep".into());
        }
        let request = match commands.try_recv().unwrap() {
            Command::Move(n, ..) | Command::Transfer(n, ..) => n,
            _ => panic!("Expected move"),
        };
        assert_eq!(app.page.total, 0);
        destination(&mut app, &store, account, "Keep").await;
        assert_eq!((app.page.total, app.page.unread), (1, 1));
        assert!(app.page.is_placeholder(&original.id));
        assert!(
            app.query.project_moves.is_empty(),
            "Display hints never alter selection scope"
        );
        assert_eq!(app.detail.as_ref().unwrap().body, detail.body);
        assert_eq!(app.detail.as_ref().unwrap().summary.folder, "Keep");
        assert!(app.detail.as_ref().unwrap().summary.remote_id.is_empty());
        assert!(app.action_mail().is_none());
        app.select_for_read(original.id.clone());
        assert!(app.mail_actions.read_candidate.is_none());
        app.toggle_mail_flag(app.page.rows[0].clone(), false);
        assert!(
            commands.try_recv().is_err(),
            "No provider call on a placeholder"
        );
        let _ = app.handle(Message::Backend(Event::Changed));
        destination(&mut app, &store, account, "Keep").await;
        assert_eq!(
            app.detail_cache.len(),
            1,
            "A background refresh keeps pending bodies"
        );
        let receipt = Arc::new(MoveReceipt::server(
            &original,
            account,
            "Keep",
            Some("91.6".into()),
            crate::mail_actions::Fingerprint::of(b"raw"),
        ));
        let current = receipt.current.as_ref().unwrap().clone();
        store
            .relocate_mail(original.clone(), current.clone())
            .await
            .unwrap();
        let old_generation = app.generation;
        let old_page = app.page.clone();
        if cross {
            let _ = app.transfer_receipt(request, original.clone(), Ok(receipt));
        } else {
            let _ = app.move_receipt(request, original.clone(), "Keep".into(), Ok(receipt));
        }
        assert_eq!((app.page.total, app.page.rows.len()), (1, 1));
        assert!(!app.page.is_placeholder(&current.id));
        assert_eq!(app.selected.as_ref(), Some(&current.id));
        assert_eq!(app.action_mail().unwrap().remote_id, "91.6");
        assert_eq!(app.detail.as_ref().unwrap().body, detail.body);
        let _ = app.handle(Message::Backend(Event::Page(
            old_generation,
            old_page,
            false,
        )));
        assert_eq!(app.selected.as_ref(), Some(&current.id));
        destination(&mut app, &store, account, "Keep").await;
        assert_eq!(app.page.total, 1);
    }
}

#[tokio::test]
async fn pending_destination_failure_or_undo_removes_destination_and_retains_original() {
    for undo in [false, true] {
        let (mut app, mut commands, detail) = super::super::tests::fixture().await;
        let store = stored_original().await;
        let original = detail.summary.clone();
        app.cache_detail(detail);
        app.move_mail(original.clone(), "Keep".into());
        let Command::Move(request, ..) = commands.try_recv().unwrap() else {
            panic!("Expected move")
        };
        destination(&mut app, &store, "fixture", "Keep").await;
        if undo {
            let tokens = app.action_toasts.current.as_ref().unwrap().undo_tokens();
            app.undo_actions(tokens);
            assert!(app.mail_actions.projected_moves().is_empty());
        } else {
            let _ = app.move_receipt(
                request,
                original.clone(),
                "Keep".into(),
                Err("Rejected by server".into()),
            );
            assert!(
                app.notice
                    .as_ref()
                    .unwrap()
                    .0
                    .contains("Rejected by server")
            );
        }
        assert_eq!(app.page.total, 0);
        assert!(app.selected.is_none());
        destination(&mut app, &store, "fixture", "INBOX").await;
        assert_eq!((app.page.total, app.page.rows.len()), (1, 1));
        assert_eq!(
            store.detail(original.id).await.unwrap().summary.folder,
            "INBOX"
        );
    }
}

#[tokio::test]
async fn projected_reads_do_not_request_obsolete_bodies_and_query_carries_intent() {
    let (mut app, mut commands, detail) = super::super::tests::fixture().await;
    let store = stored_original().await;
    app.move_mail(detail.summary.clone(), "Keep".into());
    commands.try_recv().unwrap();
    destination(&mut app, &store, "fixture", "Keep").await;
    let (sender, mut reads) = engine::CommandSender::foreground_test_channel();
    app.tx = Some(sender);
    app.select(detail.summary.id.clone());
    app.preload(detail.summary.id.clone());
    app.request_conversation(None);
    assert!(reads.try_recv().is_err());
    let _ = app.handle(Message::Backend(Event::Changed));
    let Command::Query(_, query, false) = reads.try_recv().unwrap() else {
        panic!("Expected query")
    };
    assert_eq!(query.project_moves.len(), 1);
    assert_eq!(query.project_moves[0].folder, "Keep");
    assert!(
        reads.try_recv().is_err(),
        "Refresh cannot request the obsolete reader identity"
    );
}

#[tokio::test]
async fn all_folder_scope_and_cache_commit_before_receipt_keep_one_destination_row() {
    let (mut app, mut commands, detail) = super::super::tests::fixture().await;
    let store = stored_original().await;
    app.query.folder.clear();
    app.move_mail(detail.summary.clone(), "Keep".into());
    let Command::Move(request, ..) = commands.try_recv().unwrap() else {
        panic!("Expected move")
    };
    assert_eq!(
        (app.page.total, app.page.rows[0].folder.as_str()),
        (1, "Keep")
    );
    let receipt = MoveReceipt::local(&detail.summary, "Keep");
    store
        .relocate_mail(detail.summary.clone(), receipt.current.clone().unwrap())
        .await
        .unwrap();
    destination(&mut app, &store, "fixture", "Keep").await;
    assert_eq!((app.page.total, app.page.rows.len()), (1, 1));
    assert!(!app.page.is_placeholder(&detail.summary.id));
    let _ = app.move_receipt(
        request,
        detail.summary.clone(),
        "Keep".into(),
        Ok(Arc::new(receipt)),
    );
    assert_eq!((app.page.total, app.page.rows.len()), (1, 1));
    assert_eq!(app.action_mail().unwrap().folder, "Keep");
}

#[tokio::test]
async fn undo_of_move_waiting_for_flags_clears_an_already_opened_destination() {
    let (mut app, mut commands, detail) = super::super::tests::fixture().await;
    let store = stored_original().await;
    app.toggle_mail_flag(detail.summary.clone(), false);
    let Command::Flags(request, mail, _) = commands.try_recv().unwrap() else {
        panic!("Expected flag")
    };
    app.move_mail(detail.summary.clone(), "Keep".into());
    assert!(commands.try_recv().is_err());
    destination(&mut app, &store, "fixture", "Keep").await;
    assert_eq!(app.page.total, 1);
    app.undo_actions(app.action_toasts.current.as_ref().unwrap().undo_tokens());
    assert_eq!(app.page.total, 0);
    assert!(app.mail_actions.projected_moves().is_empty());
    store.flags(mail.clone()).await.unwrap();
    let _ = app.flags_finished(request, mail, Ok(()));
    assert!(
        commands.try_recv().is_err(),
        "The cancelled move must never reach its provider"
    );
    destination(&mut app, &store, "fixture", "INBOX").await;
    assert_eq!(app.page.total, 1);
    assert!(
        app.page.rows[0].starred,
        "Undo preserves the separate flag intent"
    );
}

#[tokio::test]
async fn transfer_between_inboxes_keeps_membership_when_the_inbox_is_unified() {
    let (mut app, mut commands, detail) = super::super::tests::fixture().await;
    assert!(app.query.account.is_none());
    app.transfer_mail(detail.summary.clone(), "personal".into(), "INBOX".into());
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::Transfer(..)
    ));
    assert_eq!((app.page.total, app.page.rows.len()), (1, 1));
    assert_eq!(app.page.rows[0].account_id, "personal");
    assert!(app.page.is_placeholder(&detail.summary.id));
    assert_eq!(app.page.inbox_unread["fixture"], 0);
    assert_eq!(app.page.inbox_unread["personal"], 1);
    assert!(app.action_mail().is_none());
}

#[tokio::test]
async fn missing_uid_ack_keeps_destination_reader_and_cold_recovery_body_loads_without_provider_identity()
 {
    use crate::mail_actions::journal::{MoveRecord, MoveStage};
    let (mut app, mut commands, detail) = super::super::tests::fixture().await;
    let store = stored_original().await;
    let original = detail.summary.clone();
    app.cache_detail(detail.clone());
    app.move_mail(original.clone(), "Keep".into());
    let Command::Move(request, ..) = commands.try_recv().unwrap() else {
        panic!("Expected move")
    };
    destination(&mut app, &store, "fixture", "Keep").await;
    let record = MoveRecord::new(
        original.clone(),
        MoveReceipt::server(
            &original,
            "fixture",
            "Keep",
            None,
            store
                .message_fingerprint(original.id.clone())
                .await
                .unwrap(),
        ),
    );
    store.prepare_mail_move(record.clone()).await.unwrap();
    let record = store
        .checkpoint_mail_move(record.clone(), MoveStage::Committed, record.receipt)
        .await
        .unwrap();
    let _ = app.move_receipt(
        request,
        original.clone(),
        "Keep".into(),
        Ok(Arc::new(record.receipt.clone())),
    );
    assert_eq!(app.selected.as_ref(), Some(&original.id));
    assert_eq!(app.detail.as_ref().unwrap().body, detail.body);
    assert_eq!(app.page.total, 1);
    assert!(app.page.is_placeholder(&original.id));
    assert!(!app.page.is_transient_placeholder(&original.id));
    destination(&mut app, &store, "fixture", "Keep").await;
    app.detail_cache.clear();
    app.detail = None;
    app.pending_details.clear();
    while commands.try_recv().is_ok() {}
    let (sender, mut reads) = engine::CommandSender::foreground_test_channel();
    app.tx = Some(sender);
    app.select(original.id.clone());
    let mut load = None;
    while let Ok(command) = reads.try_recv() {
        if let Command::Detail {
            revision,
            id,
            prefetch: false,
            ..
        } = command
        {
            load = Some((revision, id));
        } else {
            assert!(!matches!(
                command,
                Command::Move(..) | Command::Flags(..) | Command::Transfer(..)
            ));
        }
    }
    let (revision, id) = load.expect("Cold recovery must read protected cached MIME");
    assert_eq!(id, original.id);
    let loaded = Arc::new(store.detail(id.clone()).await.unwrap());
    let _ = app.handle(Message::Backend(Event::Detail {
        revision,
        id,
        result: Ok(loaded),
        prefetch: false,
    }));
    assert_eq!(app.detail.as_ref().unwrap().body, detail.body);
    assert!(app.action_mail().is_none());
    app.select_for_read(original.id.clone());
    assert!(app.mail_actions.read_candidate.is_none());
    // A destination page may overtake its typed recovery event. Its alias
    // retains the open reader and cached body instead of selecting another row.
    let resolved = parse_mail(
        "fixture",
        "91.5",
        "Keep",
        store.raw_message(original.id.clone()).await.unwrap(),
        false,
        true,
    )
    .unwrap();
    let located = store.resolve_mail_move(record, resolved).await.unwrap();
    let current = located.receipt.current.as_ref().unwrap();
    let mut query = app.query.clone();
    query.observe = vec![original.id.clone()];
    let page = Arc::new(store.query(query).await.unwrap());
    let _ = app.handle(Message::Backend(Event::Page(app.generation, page, false)));
    assert_eq!(app.selected.as_ref(), Some(&current.id));
    assert_eq!(app.detail.as_ref().unwrap().summary.id, current.id);
    assert_eq!(app.detail.as_ref().unwrap().body, detail.body);
    app.move_recovered(&located);
    assert_eq!(app.page.rows.len(), 1);
    assert!(!app.page.is_placeholder(&current.id));
    assert!(app.action_mail().is_some());
}

#[tokio::test]
async fn keeping_local_copy_retires_original_server_undo_and_rekeys_the_reader() {
    use crate::mail_actions::journal::{MoveRecord, MoveStage};
    let (mut app, _, detail) = super::super::tests::fixture().await;
    let store = stored_original().await;
    let record = MoveRecord::new(
        detail.summary.clone(),
        MoveReceipt::server(
            &detail.summary,
            "fixture",
            "Keep",
            None,
            store
                .message_fingerprint(detail.summary.id.clone())
                .await
                .unwrap(),
        ),
    );
    store.prepare_mail_move(record.clone()).await.unwrap();
    app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
    let token = app.action_toasts.add("fixture", "Keep", Instant::now());
    app.remember_move(token, &detail.summary);
    app.mail_actions.undo.get_mut(&token).unwrap().receipt = Some(Arc::new(record.receipt.clone()));
    let kept = store.keep_mail_move(record, true).await.unwrap();
    assert_eq!(kept.stage, MoveStage::Kept);
    app.move_recovered(&kept);
    let retained = kept.retained.unwrap();
    assert_eq!(app.selected.as_ref(), Some(&retained.id));
    assert_eq!(app.detail.as_ref().unwrap().summary.id, retained.id);
    assert_eq!(app.detail.as_ref().unwrap().body, detail.body);
    assert!(!app.mail_actions.undo.contains_key(&token));
    assert!(app.action_toasts.current.is_none());
    assert!(!app.page.is_placeholder(&retained.id));
    assert!(app.action_mail().unwrap().is_local_copy());
    assert_eq!(app.page.inbox_unread.get("fixture"), Some(&1));
}

#[tokio::test]
async fn combined_filtered_counts_exclude_pending_removed_rows_before_and_after_receipts() {
    for marking_unread in [false, true] {
        for success in [false, true] {
            for cache_first in [false, true] {
                let (mut app, mut commands, detail) = super::super::tests::fixture().await;
                let store = stored_original().await;
                let mut original = detail.summary.clone();
                original.unread = !marking_unread;
                original.starred = true;
                store.flags(original.clone()).await.unwrap();
                app.query.folder.clear();
                app.query.folders = Some(vec![
                    FolderSelection {
                        account: None,
                        folder: "INBOX".into(),
                        sent_only: false,
                    },
                    FolderSelection {
                        account: Some("fixture".into()),
                        folder: "INBOX".into(),
                        sent_only: false,
                    },
                    FolderSelection {
                        account: Some("fixture".into()),
                        folder: "Keep".into(),
                        sent_only: false,
                    },
                ]);
                app.query.read_only = marking_unread;
                app.query.starred_only = !marking_unread;
                app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
                assert_eq!(
                    app.page.total, 1,
                    "Overlapping folder selections must count once"
                );
                app.toggle_mail_flag(original.clone(), marking_unread);
                let Command::Flags(request, sent, _) = commands.try_recv().unwrap() else {
                    panic!("Expected pending flags");
                };
                assert_eq!(
                    (app.page.rows.len(), app.page.total, app.page.unread),
                    (0, 0, 0),
                    "Removed unread rows cannot remain in the filtered header count"
                );
                assert_eq!(
                    app.page.inbox_unread["fixture"], 1,
                    "The global Inbox badge remains independent of filtered membership"
                );
                if cache_first && success {
                    store.flags(sent.clone()).await.unwrap();
                }
                // A combined-scope refresh can arrive before the write's receipt.
                let mut query = app.query.clone();
                query.observe = app.mail_actions.observed_ids();
                app.set_mail_page(Arc::new(store.query(query).await.unwrap()));
                assert_eq!(
                    (app.page.rows.len(), app.page.total, app.page.unread),
                    (0, 0, 0)
                );
                if success && !cache_first {
                    store.flags(sent.clone()).await.unwrap();
                }
                let _ = app.flags_finished(
                    request,
                    sent,
                    if success {
                        Ok(())
                    } else {
                        Err("Fixture rejected flags".into())
                    },
                );
                let expected = usize::from(!success);
                assert_eq!(
                    (app.page.total, app.page.unread),
                    (expected, expected * usize::from(!marking_unread))
                );
                app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
                assert_eq!(
                    (app.page.rows.len(), app.page.total, app.page.unread),
                    (expected, expected, expected * usize::from(!marking_unread))
                );
                if !success {
                    assert!(
                        app.notice.as_ref().is_some_and(
                            |(message, error, _)| *error && message.contains("restored")
                        )
                    );
                }
            }
        }
    }
}
