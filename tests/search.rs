use shep::{model::*, store::Store};

fn message(id: usize, subject: &str, body: &str) -> StoredMail {
    let mut mail = parse_mail("work", &id.to_string(), "INBOX", format!("From: Alex <alex@example.test>\r\nTo: morgan@example.test\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}").into_bytes(), true, false).unwrap();
    mail.summary.timestamp = id as i64;
    mail
}

#[tokio::test]
async fn exact_short_body_beats_newer_weak_and_typo_matches_with_stable_pages_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("search.sqlite");
    let store = Store::open(&path).unwrap();
    let mut messages = vec![
        message(0, "Quick note", "test"),
        message(200, "Camping", "tent"),
        message(201, "Testing", "testing"),
    ];
    messages.push(message(202, "Keyword repetition", &"test ".repeat(500)));
    messages.extend((1..=105).map(|id| {
        message(
            id,
            "Project update",
            &format!(
                "Here is the test plan for the next phase. {}",
                "Other project notes. ".repeat(30)
            ),
        )
    }));
    store.upsert(messages).await.unwrap();
    let query = MailQuery {
        folder: "INBOX".into(),
        search: "test".into(),
        sort: MailSort::Relevance,
        ..Default::default()
    };
    let mut ids = Vec::new();
    for offset in [0, 50, 100] {
        let page = store
            .query(MailQuery {
                offset,
                ..query.clone()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 109);
        assert_eq!(page.unread, 109);
        ids.extend(page.rows.into_iter().map(|row| row.id));
    }
    assert_eq!(ids[0], "work:INBOX:0");
    assert!(
        !ids[..106]
            .iter()
            .any(|id| id == "work:INBOX:200" || id == "work:INBOX:201")
    );
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        109
    );
    drop(store);
    let store = Store::open(path).unwrap();
    assert_eq!(store.query(query.clone()).await.unwrap().rows[0].id, ids[0]);
    for sort in MailSort::BROWSE {
        let page = store
            .query(MailQuery {
                sort,
                ..query.clone()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 109);
        if sort == MailSort::Newest {
            assert_eq!(page.rows[0].remote_id, "202");
        }
    }
}

#[tokio::test]
async fn relevance_respects_combined_folder_scopes_flags_unicode_and_literal_query_text() {
    let store = Store::memory().unwrap();
    let mut other = message(3, "Café", "Architecture planning");
    other.summary.account_id = "personal".into();
    other.summary.id = "personal:3".into();
    other.summary.folder = "Projects".into();
    other.summary.starred = true;
    store
        .upsert(vec![
            message(1, "Café", "Architecture planning"),
            message(2, "Other", "unrelated"),
            other,
        ])
        .await
        .unwrap();
    for search in [
        "cafe",
        "CAFÉ",
        "cafe\u{301}",
        "architecutre",
        "architctur",
        "planning café",
    ] {
        let query = MailQuery {
            folder: "".into(),
            search: search.into(),
            sort: MailSort::Relevance,
            ..Default::default()
        };
        assert_eq!(
            store.query(query.clone()).await.unwrap().total,
            2,
            "{search}"
        );
        let page = store
            .query(MailQuery {
                starred_only: true,
                folders: Some(vec![FolderSelection {
                    account: Some("personal".into()),
                    folder: "Projects".into(),
                    sent_only: false,
                }]),
                ..query
            })
            .await
            .unwrap();
        assert_eq!(page.total, 1, "{search}");
        assert_eq!(page.rows[0].id, "personal:3");
    }
    for search in ["***", "😊", "\" OR *", "'; DROP TABLE messages; --"] {
        let page = store
            .query(MailQuery {
                search: search.into(),
                sort: MailSort::Relevance,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 0, "{search}");
    }
    assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 3);
}

#[test]
fn move_matches_accents_nested_words_transpositions_and_abbreviations() {
    use shep::fuzzy::ranked;
    let choices = || {
        [
            "Archive",
            "Projects/Archive",
            "Projects/Archived",
            "Café",
            "Work",
        ]
        .map(str::to_owned)
    };
    for query in ["archvie", "archive"] {
        assert_eq!(ranked(query, choices())[0], "Archive");
    }
    for query in ["cafe", "CAFE\u{301}"] {
        assert_eq!(ranked(query, choices())[0], "Café");
    }
    assert_eq!(ranked("proejcts arc", choices())[0], "Projects/Archive");
    assert_eq!(ranked("pjarch", choices())[0], "Projects/Archive");
    assert!(ranked("qzxwv", choices()).is_empty());
    assert!(ranked("!!!", choices()).is_empty());
}

#[tokio::test]
async fn mail_search_preserves_japanese_marks_and_hangul_syllables() {
    let store = Store::memory().unwrap();
    store
        .upsert(vec![
            message(1, "旅行の計画", "がんばる"),
            message(2, "안녕하세요", "주말"),
        ])
        .await
        .unwrap();
    for search in ["がんばる", "안녕하세요"] {
        let page = store
            .query(MailQuery {
                search: search.into(),
                sort: MailSort::Relevance,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 1, "{search}");
    }
}

#[tokio::test]
async fn a_short_body_with_line_endings_beats_repeated_keywords() {
    let store = Store::memory().unwrap();
    store
        .upsert(vec![
            message(0, "Quick note", " \ttest\r\n"),
            message(1, "Test plan", &"test ".repeat(500)),
        ])
        .await
        .unwrap();
    for search in ["test", "TEST", " test "] {
        let page = store
            .query(MailQuery {
                search: search.into(),
                sort: MailSort::Relevance,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.rows[0].remote_id, "0", "{search}");
    }
}
