use shep::{
    model::parse_mail,
    store::{READER_BODY_PAGE, Store},
};

fn message(remote_id: &str, body: &str) -> shep::model::StoredMail {
    parse_mail(
        "reader",
        remote_id,
        "INBOX",
        format!("From: Morgan <morgan@example.test>\r\nTo: alex@example.test\r\nSubject: Report\r\n\r\n{body}")
            .into_bytes(),
        false,
        false,
    )
    .unwrap()
}

#[tokio::test]
async fn long_plain_messages_load_in_pages_until_the_end_is_readable() {
    let store = Store::memory().unwrap();
    let body: String = (0..5000)
        .map(|line| format!("Line {line:05} of the long report.\r\n"))
        .collect();
    let long = message("long", &body);
    let id = long.summary.id.clone();
    store.upsert(vec![long]).await.unwrap();

    let first = store.detail(id.clone()).await.unwrap();
    assert!(first.body_truncated);
    assert_eq!(first.body.chars().count(), READER_BODY_PAGE);
    assert!(!first.body.contains("Line 04999"));

    let second = store
        .detail_limited(id.clone(), 2 * READER_BODY_PAGE)
        .await
        .unwrap();
    assert!(second.body_truncated);
    assert_eq!(second.body.chars().count(), 2 * READER_BODY_PAGE);
    assert!(second.body.starts_with(&first.body));

    let whole = store.detail_limited(id.clone(), usize::MAX).await.unwrap();
    assert!(!whole.body_truncated);
    assert!(whole.body.contains("Line 04999"));
    assert!(whole.body.starts_with(&second.body));

    // A smaller request never shrinks the reader below one page.
    let clamped = store.detail_limited(id, 10).await.unwrap();
    assert_eq!(clamped.body.chars().count(), READER_BODY_PAGE);
}

#[tokio::test]
async fn short_messages_are_complete_at_any_page_size() {
    let store = Store::memory().unwrap();
    let short = message("short", "A short note.");
    let id = short.summary.id.clone();
    store.upsert(vec![short]).await.unwrap();
    let detail = store
        .detail_limited(id, 3 * READER_BODY_PAGE)
        .await
        .unwrap();
    assert!(!detail.body_truncated);
    assert!(detail.body.contains("A short note."));
}
