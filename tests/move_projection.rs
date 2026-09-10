use shep::{model::*, store::Store};

fn mail(account: &str, uid: &str, folder: &str, time: i64, body: &str) -> StoredMail {
    let mut mail = parse_mail(account,uid,folder,
        format!("From: Sender <sender@example.test>\r\nSubject: Message {uid}\r\nMessage-ID: <{uid}@example.test>\r\n\r\n{body}").into_bytes(),true,false).unwrap();
    mail.summary.timestamp = time;
    mail
}
fn projection(mail: &Mail, account: &str, folder: &str) -> MailMoveProjection {
    MailMoveProjection {
        id: mail.id.clone(),
        source_account: mail.account_id.clone(),
        source_folder: mail.folder.clone(),
        account: account.into(),
        folder: folder.into(),
        unread: mail.unread,
        starred: mail.starred,
    }
}
#[tokio::test]
async fn pending_move_is_visible_in_destination_search_without_changing_provider_identity() {
    let store = Store::memory().unwrap();
    let source = mail(
        "work",
        "1.1",
        "INBOX",
        100,
        "A unique fullbodyword after the preview",
    );
    let original = source.summary.clone();
    store.upsert(vec![source]).await.unwrap();
    let change = projection(&original, "personal", "Keep");
    let query = MailQuery {
        account: Some("personal".into()),
        folder: "Keep".into(),
        search: "fullbodyword".into(),
        sort: MailSort::Relevance,
        project_moves: vec![change.clone()],
        observe: vec![original.id.clone()],
        ..Default::default()
    };
    let page = store.query(query.clone()).await.unwrap();
    assert_eq!((page.total, page.unread, page.rows.len()), (1, 1, 1));
    assert_eq!(page.rows[0].account_id, "personal");
    assert_eq!(page.rows[0].folder, "Keep");
    assert!(
        page.rows[0].remote_id.is_empty(),
        "Display metadata cannot carry an obsolete server UID"
    );
    assert!(page.move_placeholders.contains(&original.id));
    assert_eq!(
        page.observed[&original.id].as_ref().unwrap().folder,
        "INBOX"
    );
    assert_eq!(
        store
            .mail_metadata(original.id.clone())
            .await
            .unwrap()
            .folder,
        "INBOX"
    );
    assert_eq!(
        store
            .detail(original.id.clone())
            .await
            .unwrap()
            .summary
            .remote_id,
        "1.1"
    );
    let origin = store
        .query(MailQuery {
            folder: "INBOX".into(),
            project_moves: vec![change],
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(origin.total, 0);
    let plain = store
        .query(MailQuery {
            folder: "INBOX".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(
        plain.total, 1,
        "A projection cannot escape its read transaction"
    );
    assert_eq!(
        store
            .query(MailQuery {
                project_moves: vec![],
                ..query
            })
            .await
            .unwrap()
            .total,
        0
    );
}

#[tokio::test]
async fn projected_folders_keep_exact_order_page_size_filters_and_combined_membership() {
    let store = Store::memory().unwrap();
    let mut messages: Vec<_> = (0..60)
        .map(|i| mail("work", &format!("1.{}", i + 1), "Keep", i, "Body"))
        .collect();
    let source = mail("work", "1.100", "INBOX", 5, "Body");
    let mut change = projection(&source.summary, "work", "Keep");
    change.starred = true;
    change.unread = false;
    messages.push(source);
    store.upsert(messages).await.unwrap();
    let query = MailQuery {
        folders: Some(vec![FolderSelection {
            account: Some("work".into()),
            folder: "Keep".into(),
            sent_only: false,
        }]),
        sort: MailSort::Newest,
        project_moves: vec![change.clone()],
        ..Default::default()
    };
    let first = store.query(query.clone()).await.unwrap();
    assert_eq!((first.total, first.rows.len(), first.unread), (61, 50, 60));
    assert!(!first.rows.iter().any(|m| m.id == change.id));
    let second = store
        .query(MailQuery {
            offset: 50,
            ..query.clone()
        })
        .await
        .unwrap();
    assert_eq!(second.rows.len(), 11);
    let found = second.rows.iter().find(|m| m.id == change.id).unwrap();
    assert!(found.starred && !found.unread);
    assert!(
        second
            .rows
            .windows(2)
            .all(|w| w[0].timestamp >= w[1].timestamp)
    );
    let flagged = store
        .query(MailQuery {
            starred_only: true,
            ..query.clone()
        })
        .await
        .unwrap();
    assert_eq!(
        (flagged.total, flagged.rows[0].id.as_str()),
        (1, change.id.as_str())
    );
    let unread = store
        .query(MailQuery {
            unread_only: true,
            ..query
        })
        .await
        .unwrap();
    assert_eq!(unread.total, 60);
}

#[tokio::test]
async fn cache_acknowledgment_and_failed_query_cannot_duplicate_or_leak_projected_rows() {
    let store = Store::memory().unwrap();
    let source = mail("work", "1.1", "INBOX", 100, "Body");
    let original = source.summary.clone();
    let change = projection(&original, "work", "Keep");
    store.upsert(vec![source]).await.unwrap();
    let query = MailQuery {
        folder: "Keep".into(),
        project_moves: vec![change.clone()],
        ..Default::default()
    };
    assert_eq!(store.query(query.clone()).await.unwrap().total, 1);
    assert!(
        store
            .query(MailQuery {
                project_moves: vec![change.clone(), change.clone()],
                ..query.clone()
            })
            .await
            .is_err()
    );
    assert_eq!(
        store
            .query(MailQuery {
                folder: "Keep".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        0
    );
    let mut destination = original.clone();
    destination.id = "work:Keep:2.8".into();
    destination.remote_id = "2.8".into();
    destination.folder = "Keep".into();
    store
        .relocate_mail(original, destination.clone())
        .await
        .unwrap();
    let page = store.query(query).await.unwrap();
    assert_eq!((page.total, page.rows.len()), (1, 1));
    assert_eq!(page.rows[0].id, destination.id);
    assert!(page.move_placeholders.is_empty());
}

#[tokio::test]
async fn selection_capture_rejects_display_identity_without_replacing_an_existing_selection() {
    use shep::store::MailSelectionId;
    let store = Store::memory().unwrap();
    let source = mail("work", "1.1", "INBOX", 1, "Body");
    let change = projection(&source.summary, "work", "Keep");
    store.upsert(vec![source]).await.unwrap();
    let id = MailSelectionId::default();
    let query = MailQuery {
        folder: "INBOX".into(),
        ..Default::default()
    };
    let before = store
        .capture_selection(id, 1, query.clone(), true, vec![])
        .await
        .unwrap();
    assert_eq!(before.selected, 1);
    assert!(
        store
            .capture_selection(
                id,
                2,
                MailQuery {
                    project_moves: vec![change],
                    ..query
                },
                true,
                vec![]
            )
            .await
            .is_err()
    );
    let after = store.selection_snapshot(id, vec![]).await.unwrap();
    assert_eq!(after.selected, 1);
}
