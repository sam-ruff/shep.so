use shep::{
    model::*,
    store::{CONVERSATION_PAGE_SIZE, Store},
};

fn mail(
    account: &str,
    remote: &str,
    folder: &str,
    identity: &str,
    references: &str,
    timestamp: i64,
) -> StoredMail {
    let raw = format!(
        "From: Writer <writer@example.com>\r\nTo: reader@example.com\r\nSubject: Shared subject\r\nMessage-ID: {identity}\r\nReferences: {references}\r\n\r\nMessage {remote}."
    );
    let mut mail = parse_mail(account, remote, folder, raw.into_bytes(), false, false).unwrap();
    mail.summary.timestamp = timestamp;
    mail
}

#[tokio::test]
async fn late_parents_merge_branches_without_subject_or_account_collisions() {
    let store = Store::memory().unwrap();
    let first = mail(
        "work",
        "1",
        "INBOX",
        "<one@example.com>",
        "<root@example.com>",
        1,
    );
    let second = mail(
        "work",
        "2",
        "Sent",
        "<two@example.com>",
        "<bridge@example.com>",
        2,
    );
    let separate = mail("work", "other", "INBOX", "<other@example.com>", "", 3);
    let personal = mail(
        "personal",
        "1",
        "INBOX",
        "<one@example.com>",
        "<root@example.com>",
        1,
    );
    store
        .upsert(vec![
            first.clone(),
            second.clone(),
            separate.clone(),
            personal.clone(),
        ])
        .await
        .unwrap();
    assert_eq!(
        store
            .conversation(first.summary.id.clone(), None)
            .await
            .unwrap()
            .total,
        1
    );
    let bridge = mail(
        "work",
        "3",
        "Archive",
        "<bridge@example.com>",
        "<root@example.com>",
        0,
    );
    store.upsert(vec![bridge.clone()]).await.unwrap();
    let page = store
        .conversation(first.summary.id.clone(), None)
        .await
        .unwrap();
    assert_eq!(
        page.rows
            .iter()
            .map(|mail| mail.remote_id.as_str())
            .collect::<Vec<_>>(),
        ["3", "1", "2"]
    );
    let root = mail("work", "root", "Archive", "<root@example.com>", "", -1);
    store.upsert(vec![root]).await.unwrap();
    assert_eq!(
        store
            .conversation(second.summary.id.clone(), None)
            .await
            .unwrap()
            .total,
        4
    );
    assert_eq!(
        store
            .conversation(separate.summary.id, None)
            .await
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        store
            .conversation(personal.summary.id, None)
            .await
            .unwrap()
            .total,
        1
    );
    store.remove(bridge.summary.id).await.unwrap();
    assert_eq!(
        store
            .conversation(second.summary.id, None)
            .await
            .unwrap()
            .total,
        3,
        "Deleting an intermediary must not split its surviving replies"
    );
}

#[tokio::test]
async fn duplicate_folder_copies_keep_the_selected_uid_and_current_flags() {
    let store = Store::memory().unwrap();
    let inbox = mail("work", "inbox-copy", "INBOX", "<same@example.com>", "", 1);
    let archive = mail(
        "work",
        "archive-copy",
        "Archive",
        "<same@example.com>",
        "",
        1,
    );
    let reply = mail(
        "work",
        "reply",
        "Sent",
        "<reply@example.com>",
        "<same@example.com>",
        2,
    );
    store
        .upsert(vec![inbox.clone(), archive.clone(), reply.clone()])
        .await
        .unwrap();
    let mut flag = archive.summary.clone();
    flag.starred = true;
    flag.unread = true;
    store.flags(flag).await.unwrap();
    let page = store
        .conversation(archive.summary.id.clone(), None)
        .await
        .unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.rows[0].id, archive.summary.id);
    assert!(page.rows[0].starred && page.rows[0].unread);
    let page = store.conversation(reply.summary.id, None).await.unwrap();
    assert_eq!(page.rows[0].id, inbox.summary.id);
    assert!(!page.rows[0].starred);
    store
        .move_local(archive.summary.id.clone(), "Projects".into())
        .await
        .unwrap();
    let page = store.conversation(archive.summary.id, None).await.unwrap();
    assert_eq!(page.rows[0].folder, "Projects");
}

#[tokio::test]
async fn long_conversations_page_without_omitting_the_anchor_or_older_messages() {
    let store = Store::memory().unwrap();
    let messages: Vec<_> = (0..55)
        .map(|i| {
            mail(
                "work",
                &i.to_string(),
                if i == 54 { "INBOX" } else { "Archive" },
                &format!("<{i}@example.com>"),
                "<root@example.com>",
                i,
            )
        })
        .collect();
    let anchor = messages.last().unwrap().summary.id.clone();
    store.upsert(messages).await.unwrap();
    let last = store.conversation(anchor.clone(), None).await.unwrap();
    assert_eq!((last.total, last.offset, last.rows.len()), (55, 40, 15));
    assert_eq!(last.rows.last().unwrap().id, anchor);
    let focused = store
        .conversation_around(anchor.clone(), Some("work:Archive:17".into()), None)
        .await
        .unwrap();
    assert_eq!(focused.offset, 0);
    assert!(focused.rows.iter().any(|mail| mail.remote_id == "17"));
    let mut seen = Vec::new();
    for offset in [0, 20, 40] {
        let page = store
            .conversation(anchor.clone(), Some(offset))
            .await
            .unwrap();
        assert!(page.rows.len() <= CONVERSATION_PAGE_SIZE);
        seen.extend(page.rows.into_iter().map(|mail| mail.id));
    }
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 55);
    assert_eq!(
        store
            .conversation(anchor, Some(usize::MAX))
            .await
            .unwrap()
            .offset,
        40
    );
}

#[tokio::test]
async fn legacy_indexing_resumes_after_reopen_and_does_not_change_original_mail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    let messages: Vec<_> = (0..70)
        .map(|i| {
            mail(
                "work",
                &i.to_string(),
                "INBOX",
                &format!("<{i}@example.com>"),
                "<root@example.com>",
                i,
            )
        })
        .collect();
    let anchor = messages[69].summary.id.clone();
    let original = messages[69].raw.clone();
    store.upsert(messages).await.unwrap();
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("DROP TABLE conversation_members; DROP TABLE conversation_tokens;")
        .unwrap();
    drop(conn);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .conversation(anchor.clone(), None)
            .await
            .unwrap()
            .total,
        1
    );
    assert!(store.index_conversation_batch().await.unwrap());
    drop(store);
    let store = Store::open(path).unwrap();
    while store.index_conversation_batch().await.unwrap() {}
    assert_eq!(
        store
            .conversation(anchor.clone(), None)
            .await
            .unwrap()
            .total,
        70
    );
    assert_eq!(store.raw_message(anchor).await.unwrap(), original);
    assert!(!store.index_conversation_batch().await.unwrap());
}

#[tokio::test]
async fn missing_or_invalid_identifiers_do_not_merge_equal_subjects() {
    let store = Store::memory().unwrap();
    let first = mail("work", "1", "INBOX", "not-a-message-id", "", 1);
    let second = mail("work", "2", "INBOX", "", "also invalid", 2);
    store
        .upsert(vec![first.clone(), second.clone()])
        .await
        .unwrap();
    assert_eq!(
        store
            .conversation(first.summary.id, None)
            .await
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        store
            .conversation(second.summary.id, None)
            .await
            .unwrap()
            .total,
        1
    );
}
