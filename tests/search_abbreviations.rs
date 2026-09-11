use shep::{
    fuzzy::{Matcher, WordMatcher, ranked, ranked_labels, score},
    model::*,
    store::{MailSelectionId, Store},
};

#[test]
fn catalogue_word_scoring_reuses_queries_and_keeps_identifier_and_typo_rules() {
    let mut matcher = WordMatcher::new("prf ntfctns archvie S3 2026");
    for (query, candidate) in [
        ("prf", "profiles"),
        ("ntfctns", "notifications"),
        ("archvie", "archive"),
        ("s3", "s3"),
        ("2026", "2026"),
    ] {
        assert!(matcher.score_normalized(query, candidate).is_some());
    }
    for (query, candidate) in [
        ("s3", "s30"),
        ("2026", "20260"),
        ("prf", "archive"),
        ("absent", "absent"),
    ] {
        assert_eq!(matcher.score_normalized(query, candidate), None);
    }
}

#[test]
fn labels_and_path_leaves_outrank_abbreviations_and_typo_fallbacks() {
    let ranked = ranked(
        "archive",
        [
            "A remote collection holding varied entries",
            "Archives",
            "Projects/Archive",
            "Archive",
            "Archvie",
        ]
        .map(str::to_owned),
    );
    assert_eq!(&ranked[..3], ["Archive", "Projects/Archive", "Archives"]);
    assert_eq!(ranked.last().map(String::as_str), Some("Archvie"));
    assert!(score("pjarch", "Projects/Archive").is_some());
    assert!(score("proejcts arc", "Projects/Archive").is_some());
    assert!(score("sfp fng", "SFTP fingerprint").is_some());
    assert!(score("sys tr", "System tray").is_some());
}

#[test]
fn each_term_is_required_and_numbers_cannot_be_approximate() {
    for (query, candidate) in [
        ("pjarch absent", "Projects/Archive"),
        ("archive 17", "Archive 170"),
        ("17", "170"),
        ("17", "171"),
        ("17", "Folder 1 7"),
        ("17", "Folder 18"),
        ("archvie 17", "Archive 170"),
        ("S3", "S30 storage"),
        ("2026", "Archive 20260"),
    ] {
        assert_eq!(score(query, candidate), None, "{query}: {candidate}");
    }
    assert!(score("archvie 17", "Folder 17/Archive").is_some());
    assert!(score("17", "Folder 17/Archive").is_some());
    assert!(score("S3", "S3 storage").is_some());
}

#[test]
fn punctuation_is_literal_and_never_nucleo_pattern_syntax() {
    for query in [
        "!archive",
        "^archive",
        "archive$",
        "archive | work",
        "'archive",
    ] {
        assert_eq!(score(query, "Archive"), None, "{query}");
    }
    assert!(score("^archive", "Projects/^Archive").is_some());
    assert!(score("!archive", "Projects/!Archive").is_some());
    assert_eq!(score("!!!", "!!!"), None);
}

#[test]
fn reused_matcher_preserves_unicode_and_deterministic_protocol_id_ties() {
    let mut matcher = Matcher::new("cafe");
    for candidate in ["Café", "Cafe\u{301}", "CAFÉ"] {
        assert_eq!(matcher.score(candidate), Some(0));
    }
    assert_eq!(score("がんばる", "かんはる"), None);
    assert_eq!(score("안녕하세요", "안녕하세요"), Some(0));
    assert_eq!(score("कि", "क"), None);
    assert_eq!(
        ranked_labels(
            "arc",
            [
                ("uid-z".into(), "Archive".into()),
                ("uid-a".into(), "Archive".into()),
                ("uid-b".into(), "Archives".into()),
            ],
        ),
        ["uid-a", "uid-z", "uid-b"]
    );
}

fn mail(id: usize, body: &str) -> StoredMail {
    let mut mail = parse_mail(
        "work",
        &id.to_string(),
        "INBOX",
        format!("From: Alex <alex@example.test>\r\nSubject: Notes\r\n\r\n{body}").into_bytes(),
        true,
        false,
    )
    .unwrap();
    mail.summary.timestamp = 100;
    mail
}

#[tokio::test]
async fn phrase_tiers_preserve_paging_capture_and_equal_timestamp_order_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("phrases.sqlite");
    let store = Store::open(&path).unwrap();
    let mut rows = vec![
        mail(0, "architecture plan 17"),
        mail(1, "17 plan architecture"),
        mail(2, "architecture planning 17"),
        mail(3, "architecture plan 170"),
        mail(4, "architecture only 17"),
    ];
    let phrase_body = format!("The architecture plan 17. {}", "Other notes. ".repeat(100));
    rows.extend((10..75).map(|id| mail(id, &phrase_body)));
    store.upsert(rows).await.unwrap();
    let query = MailQuery {
        search: "architecture plan 17".into(),
        sort: MailSort::Relevance,
        folders: Some(vec![FolderSelection {
            account: Some("work".into()),
            folder: "INBOX".into(),
            sent_only: false,
        }]),
        ..Default::default()
    };
    let mut expected = vec!["work:INBOX:0".to_owned()];
    expected.extend((10..75).map(|id| format!("work:INBOX:{id}")));
    expected.extend(["work:INBOX:1".into(), "work:INBOX:2".into()]);
    drop(store);
    for _ in 0..2 {
        let store = Store::open(&path).unwrap();
        let mut actual = Vec::new();
        for offset in [0, PAGE_SIZE] {
            let page = store
                .query(MailQuery {
                    offset,
                    ..query.clone()
                })
                .await
                .unwrap();
            assert_eq!(page.total, expected.len());
            assert!(page.rows.len() <= PAGE_SIZE);
            actual.extend(page.rows.into_iter().map(|row| row.id));
        }
        assert_eq!(actual, expected);
        let id = MailSelectionId::default();
        let capture = store
            .capture_selection(id, 0, query.clone(), true, vec![])
            .await
            .unwrap();
        assert_eq!(capture.selected, expected.len());
        let mut selected = Vec::new();
        let mut after = None;
        loop {
            let page = store.selected_mail_page(id, 0, after).await.unwrap();
            assert!(page.rows.len() <= PAGE_SIZE);
            selected.extend(page.rows.into_iter().map(|row| row.mail.id));
            after = page.next_after;
            if after.is_none() {
                break;
            }
        }
        assert_eq!(selected, expected);
        store.release_selection(id).await.unwrap();
    }
}
