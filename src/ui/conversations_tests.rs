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
    assert_eq!(app.field("to"), "parent@example.com");
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
