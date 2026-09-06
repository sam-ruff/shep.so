//! State invariants for delayed backend results. Mouse flagging/moving is covered
//! separately by the native MCP suite; these tests control result arrival order.
use super::*;

#[test]
fn recovered_mail_sync_clears_its_error_but_preserves_other_action_errors() {
    let (mut app, _) = App::new();
    let _ = app.handle(Message::Backend(Event::MailSyncFinished(Err(
        "Mail server unavailable. Try Refresh again.".into(),
    ))));
    assert!(app.notice.as_ref().unwrap().1);
    let _ = app.handle(Message::Backend(Event::MailSyncFinished(Ok(()))));
    assert!(app.notice.is_none());
    let _ = app.handle(Message::Backend(Event::MailSyncFinished(Err(
        "Mail server unavailable.".into(),
    ))));
    // Give the unrelated notice a distinct, deterministic identity.
    app.sync_notice = Some(Instant::now() - std::time::Duration::from_secs(1));
    app.notice("Archive failed. The message was restored.", true);
    let _ = app.handle(Message::Backend(Event::MailSyncFinished(Ok(()))));
    assert_eq!(
        app.notice.as_ref().unwrap().0,
        "Archive failed. The message was restored."
    );
    assert!(app.sync_notice.is_none());
}

#[tokio::test]
async fn stale_prefetch_cannot_restore_flags_or_errors_after_a_mail_change() {
    let store = crate::store::Store::memory().unwrap();
    let mail = crate::model::parse_mail("test", "1", "INBOX",
        b"From: Test <test@example.com>\r\nTo: reader@example.com\r\nSubject: Current flags\r\n\r\nMessage body.".to_vec(), false, false).unwrap();
    let id = mail.summary.id.clone();
    store.upsert(vec![mail.clone()]).await.unwrap();
    let old = Arc::new(store.detail(id.clone()).await.unwrap());
    let mut changed = mail.summary;
    changed.starred = true;
    store.flags(changed).await.unwrap();
    let current = Arc::new(store.detail(id.clone()).await.unwrap());
    let (mut app, _) = App::new();
    app.selected = Some(id.clone());
    app.detail = Some(old.clone());
    app.pending_details.insert(id.clone());
    let _ = app.handle(Message::Backend(Event::Changed));
    assert!(app.pending_details.is_empty());
    let revision = app.detail_revision;
    let _ = app.handle(Message::Backend(Event::Detail {
        revision,
        id: id.clone(),
        result: Ok(current),
        prefetch: false,
    }));
    app.notice = None;
    app.pending_details.insert(id.clone());
    let _ = app.handle(Message::Backend(Event::Detail {
        revision: revision - 1,
        id: id.clone(),
        result: Ok(old),
        prefetch: true,
    }));
    let _ = app.handle(Message::Backend(Event::Detail {
        revision: revision - 1,
        id: id.clone(),
        result: Err("No longer in the old folder".into()),
        prefetch: false,
    }));
    assert!(app.detail.as_ref().unwrap().summary.starred);
    assert!(app.detail_cache.front().unwrap().summary.starred);
    assert!(app.pending_details.contains(&id));
    assert!(app.notice.is_none());
}

#[test]
fn a_failed_background_prefetch_clears_its_marker_without_an_unrelated_error() {
    let (mut app, _) = App::new();
    app.pending_details.insert("other-message".into());
    let _ = app.handle(Message::Backend(Event::Detail {
        revision: app.detail_revision,
        id: "other-message".into(),
        result: Err("Message was moved".into()),
        prefetch: true,
    }));
    assert!(app.pending_details.is_empty());
    assert!(app.notice.is_none());
}

#[tokio::test]
async fn context_reply_waits_for_clicked_body_and_navigation_cancels_it() {
    use super::context_menu::MailAction;
    let store = crate::store::Store::memory().unwrap();
    let mut messages = vec![];
    for (id, sender) in [("1", "first@example.com"), ("2", "second@example.com")] {
        messages.push(parse_mail("test", id, "INBOX", format!("From: {sender}\r\nTo: reader@example.com\r\nSubject: Message {id}\r\n\r\nBody {id}").into_bytes(), false, false).unwrap());
    }
    store.upsert(messages.clone()).await.unwrap();
    let first = Arc::new(store.detail(messages[0].summary.id.clone()).await.unwrap());
    let second = Arc::new(store.detail(messages[1].summary.id.clone()).await.unwrap());
    let (mut app, _) = App::new();
    app.page = Arc::new(MailPage {
        rows: messages.iter().map(|m| m.summary.clone()).collect(),
        ..Default::default()
    });
    app.selected = Some(first.summary.id.clone());
    app.detail = Some(first.clone());
    let _ = app.handle(Message::MailContext(
        second.summary.id.clone(),
        iced::Point::ORIGIN,
    ));
    let _ = app.handle(Message::MailContextAction(MailAction::Reply));
    assert!(app.dialog.is_none());
    let _ = app.handle(Message::Backend(Event::Detail {
        revision: app.detail_revision,
        id: first.summary.id.clone(),
        result: Ok(first.clone()),
        prefetch: true,
    }));
    assert!(app.dialog.is_none());
    assert!(app.pending_mail_action.is_some());
    app.select(first.summary.id.clone());
    let _ = app.handle(Message::Backend(Event::Detail {
        revision: app.detail_revision,
        id: second.summary.id.clone(),
        result: Ok(second.clone()),
        prefetch: false,
    }));
    assert!(app.dialog.is_none());
    assert!(app.pending_mail_action.is_none());
    let _ = app.handle(Message::MailContext(
        second.summary.id.clone(),
        iced::Point::ORIGIN,
    ));
    let _ = app.handle(Message::MailContextAction(MailAction::Reply));
    assert_eq!(app.dialog, Some(Dialog::Compose));
    assert_eq!(app.field("to"), "second@example.com");
    assert_eq!(app.field("subject"), "Re: Message 2");
}

#[tokio::test]
async fn mail_refresh_preserves_context_target_and_updates_its_flags() {
    let mail = parse_mail(
        "test",
        "1",
        "INBOX",
        b"From: fixture@example.test\r\nSubject: Keep menu open\r\n\r\nBody".to_vec(),
        true,
        false,
    )
    .unwrap()
    .summary;
    let (mut app, _) = App::new();
    app.page = Arc::new(MailPage {
        rows: vec![mail.clone()],
        ..Default::default()
    });
    let _ = app.handle(Message::MailContext(
        mail.id.clone(),
        iced::Point::new(400., 250.),
    ));
    let _ = app.handle(Message::Backend(Event::Changed));
    assert_eq!(app.context_menu.as_ref().unwrap().mail.id, mail.id);
    let mut updated = mail;
    updated.unread = false;
    updated.starred = true;
    let page = Arc::new(MailPage {
        rows: vec![updated],
        ..Default::default()
    });
    let _ = app.handle(Message::Backend(Event::Page(app.generation, page, false)));
    let menu = app.context_menu.as_ref().unwrap();
    assert!(!menu.mail.unread);
    assert!(menu.mail.starred);
    let _ = app.handle(Message::DismissContext);
    assert!(app.context_menu.is_none());
}

#[tokio::test]
async fn tooltip_hints_use_only_primary_and_can_be_disabled() {
    let (mut app, _) = App::new();
    assert!(
        app.shortcut_hint("Archive", Action::Archive)
            .contains("Backspace")
    );
    assert!(
        !app.shortcut_hint("Archive", Action::Archive)
            .contains("Delete")
    );
    app.preferences.shortcut_tooltips = false;
    assert_eq!(app.shortcut_hint("Archive", Action::Archive), "Archive");
    let _ = app.key(
        Key::Named(keyboard::key::Named::Backspace),
        keyboard::Modifiers::default(),
        true,
    );
    let _ = app.key(Key::Character("d".into()), keyboard::Modifiers::CTRL, true);
    assert!(app.notice.is_none());
}
