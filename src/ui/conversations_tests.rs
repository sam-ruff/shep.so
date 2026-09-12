use super::*;

async fn mail(store: &crate::store::Store, id: &str) -> Arc<MailDetail> {
    let raw = format!(
        "From: {id}@example.com\r\nTo: reader@example.com\r\nSubject: {id}\r\n\r\nBody for {id}"
    );
    let mail = parse_mail("work", id, "INBOX", raw.into_bytes(), false, false).unwrap();
    let id = mail.summary.id.clone();
    store.upsert(vec![mail]).await.unwrap();
    Arc::new(store.detail(id).await.unwrap())
}

#[tokio::test]
async fn conversation_actions_wait_for_current_detail_and_keep_a_collapsed_target() {
    let store = crate::store::Store::memory().unwrap();
    let anchor = mail(&store, "anchor").await;
    let older = mail(&store, "older").await;
    let (mut app, _) = App::new();
    app.selected = Some(anchor.summary.id.clone());
    app.conversation.focus = Some(older.summary.id.clone());
    app.detail = Some(anchor);
    assert!(app.conversation_action_detail().is_none());
    app.detail = None;
    assert!(app.conversation_action_detail().is_none());
    app.detail = Some(older.clone());
    app.conversation.collapsed = true;
    assert_eq!(
        app.conversation_action_detail()
            .map(|detail| &detail.summary.id),
        Some(&older.summary.id)
    );
    app.selected = None;
    assert!(app.conversation_action_detail().is_none());
}

#[tokio::test]
async fn opening_a_related_message_keeps_the_inbox_anchor_and_rejects_late_bodies() {
    let store = crate::store::Store::memory().unwrap();
    let anchor = mail(&store, "anchor").await;
    let parent = mail(&store, "parent").await;
    let (mut app, _) = App::new();
    app.selected = Some(anchor.summary.id.clone());
    app.detail = Some(anchor.clone());
    app.conversation.generation = 2;
    let page = Arc::new(ConversationPage {
        anchor: anchor.summary.id.clone(),
        rows: vec![parent.summary.clone(), anchor.summary.clone()],
        total: 2,
        offset: 0,
    });
    let _ = app.conversation_result(2, anchor.summary.id.clone(), Ok(page));
    let _ = app.handle(Message::ConversationMessage(parent.summary.id.clone()));
    assert_eq!(app.selected.as_deref(), Some(anchor.summary.id.as_str()));
    assert_eq!(app.reader_id(), Some(parent.summary.id.as_str()));
    let _ = app.handle(Message::Backend(Event::Detail {
        revision: 0,
        id: anchor.summary.id.clone(),
        result: Ok(anchor.clone()),
        prefetch: false,
    }));
    assert!(app.detail.is_none());
    let _ = app.handle(Message::Backend(Event::Detail {
        revision: 0,
        id: parent.summary.id.clone(),
        result: Ok(parent.clone()),
        prefetch: false,
    }));
    assert_eq!(app.detail.as_ref().unwrap().summary.id, parent.summary.id);
    let _ = app.handle(Message::Reply);
    assert_eq!(app.compose_field("to"), "parent@example.com");
}

#[tokio::test]
async fn stale_conversations_cannot_replace_current_selection_or_reenable_grouping() {
    let store = crate::store::Store::memory().unwrap();
    let anchor = mail(&store, "anchor").await;
    let (mut app, _) = App::new();
    app.selected = Some(anchor.summary.id.clone());
    app.conversation.generation = 4;
    let page = Arc::new(ConversationPage {
        anchor: anchor.summary.id.clone(),
        rows: vec![anchor.summary.clone()],
        total: 5,
        offset: 0,
    });
    let _ = app.conversation_result(3, anchor.summary.id.clone(), Ok(page.clone()));
    assert_eq!(app.conversation.page.total, 0);
    let _ = app.conversation_result(4, "different-inbox-message".into(), Ok(page.clone()));
    assert_eq!(app.conversation.page.total, 0);
    let _ = app.conversation_result(4, anchor.summary.id.clone(), Ok(page.clone()));
    assert_eq!(app.conversation.page.total, 5);
    let _ = app.handle(Message::PrefConversations(false));
    let current = app.conversation.generation;
    let _ = app.conversation_result(current, anchor.summary.id.clone(), Ok(page));
    assert_eq!(app.conversation.page.total, 0);
    assert!(!app.conversation_visible());
    assert_eq!(app.reader_id(), Some(anchor.summary.id.as_str()));
}

#[tokio::test]
async fn refreshing_the_same_conversation_does_not_schedule_a_scroll_reset() {
    let store = crate::store::Store::memory().unwrap();
    let anchor = mail(&store, "anchor").await;
    let parent = mail(&store, "parent").await;
    let (mut app, _) = App::new();
    app.selected = Some(anchor.summary.id.clone());
    app.detail = Some(anchor.clone());
    app.conversation.generation = 2;
    let page = Arc::new(ConversationPage {
        anchor: anchor.summary.id.clone(),
        rows: vec![parent.summary.clone(), anchor.summary.clone()],
        total: 2,
        offset: 0,
    });
    assert!(
        app.conversation_result(2, anchor.summary.id.clone(), Ok(page.clone()))
            .units()
            > 0
    );
    app.conversation.generation = 3;
    assert_eq!(
        app.conversation_result(3, anchor.summary.id.clone(), Ok(page.clone()))
            .units(),
        0
    );
    let mut updated = (*page).clone();
    updated.rows[0].starred = true;
    assert_eq!(
        app.conversation_result(3, anchor.summary.id.clone(), Ok(Arc::new(updated)))
            .units(),
        0
    );
    assert!(app.conversation.page.rows[0].starred);
}

#[tokio::test]
async fn moving_an_expanded_reply_preserves_anchor_and_newer_reader_intent() {
    for success in [false, true] {
        for newer_focus in [false, true] {
            let store = crate::store::Store::memory().unwrap();
            let anchor = mail(&store, "anchor").await;
            let first = mail(&store, "first").await;
            let older = mail(&store, "older").await;
            let (sender, mut commands) = engine::CommandSender::network_test_channel();
            let (mut app, _) = App::new();
            app.tx = Some(sender);
            app.selected = Some(anchor.summary.id.clone());
            app.detail = Some(older.clone());
            app.detail_cache.push_back(anchor.clone());
            app.detail_cache.push_back(first.clone());
            app.conversation.focus = Some(older.summary.id.clone());
            let page = ConversationPage {
                anchor: anchor.summary.id.clone(),
                rows: vec![
                    first.summary.clone(),
                    older.summary.clone(),
                    anchor.summary.clone(),
                ],
                total: 3,
                offset: 0,
            };
            app.conversation.page = Arc::new(page.clone());
            app.move_mail(older.summary.clone(), "Projects".into());
            let Command::Move(request, source, _) = commands.try_recv().unwrap() else {
                panic!("Expected pending reply move");
            };
            let (sender, mut reads) = engine::CommandSender::foreground_test_channel();
            app.tx = Some(sender);
            assert_eq!(app.selected.as_ref(), Some(&anchor.summary.id));
            assert_eq!(app.reader_id(), Some(older.summary.id.as_str()));
            // A pre-commit refresh must not change the expanded reply.
            let _ = app.conversation_result(
                app.conversation.generation,
                anchor.summary.id.clone(),
                Ok(Arc::new(page.clone())),
            );
            assert_eq!(app.reader_id(), Some(older.summary.id.as_str()));
            if newer_focus {
                app.focus_conversation_message(first.summary.id.clone());
            }
            let mut refreshed = page;
            let result = if success {
                let receipt = crate::mail_actions::MoveReceipt::server(
                    &source,
                    &source.account_id,
                    "Projects",
                    Some("91.2".into()),
                    crate::mail_actions::Fingerprint::of(b"reply"),
                );
                refreshed.rows[1] = receipt.current.as_ref().unwrap().clone();
                Ok(Arc::new(receipt))
            } else {
                Err("Fixture move rejected".into())
            };
            let _ = app.move_receipt(request, source, "Projects".into(), result);
            let _ = app.conversation_result(
                app.conversation.generation,
                anchor.summary.id.clone(),
                Ok(Arc::new(refreshed)),
            );
            let expected = if newer_focus {
                &first
            } else if success {
                &anchor
            } else {
                &older
            };
            assert_eq!(app.selected.as_ref(), Some(&anchor.summary.id));
            assert_eq!(app.reader_id(), Some(expected.summary.id.as_str()));
            // The acknowledgment invalidates body caches; complete the actual
            // requested body before delivering an obsolete reply body.
            let mut requested = false;
            while let Ok(command) = reads.try_recv() {
                requested |= matches!(command, Command::Detail { id, prefetch: false, .. }
                    if id == expected.summary.id);
            }
            assert!(requested, "Expected the retained reader's body request");
            let _ = app.handle(Message::Backend(Event::Detail {
                revision: app.detail_revision,
                id: expected.summary.id.clone(),
                result: Ok(expected.clone()),
                prefetch: false,
            }));
            assert_eq!(app.detail.as_ref().unwrap().summary.id, expected.summary.id);
            if !success {
                assert!(
                    app.notice
                        .as_ref()
                        .is_some_and(|(text, error, _)| *error && text.contains("remains in Inbox"))
                );
            }
            // An earlier body result cannot displace the resulting reader.
            let _ = app.handle(Message::Backend(Event::Detail {
                revision: app.detail_revision,
                id: older.summary.id.clone(),
                result: Ok(older.clone()),
                prefetch: false,
            }));
            assert_eq!(app.detail.as_ref().unwrap().summary.id, expected.summary.id);
        }
    }
}

#[tokio::test]
async fn explicit_conversation_paging_opens_first_row_even_when_anchor_is_on_page() {
    let store = crate::store::Store::memory().unwrap();
    let anchor = mail(&store, "anchor").await;
    let previous = mail(&store, "previous").await;
    let first = mail(&store, "first").await;
    let (mut app, _) = App::new();
    app.selected = Some(anchor.summary.id.clone());
    app.conversation.focus = Some(previous.summary.id.clone());
    app.conversation.page = Arc::new(ConversationPage {
        anchor: anchor.summary.id.clone(),
        rows: vec![previous.summary.clone()],
        total: 25,
        offset: 0,
    });
    let _ = app.conversation_result(
        0,
        anchor.summary.id.clone(),
        Ok(Arc::new(ConversationPage {
            anchor: anchor.summary.id.clone(),
            rows: vec![first.summary.clone(), anchor.summary.clone()],
            total: 25,
            offset: 20,
        })),
    );
    assert_eq!(app.reader_id(), Some(first.summary.id.as_str()));
}
