//! State invariants for delayed backend results. Mouse flagging/moving is covered
//! separately by the native MCP suite; these tests control result arrival order.
use super::*;

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
