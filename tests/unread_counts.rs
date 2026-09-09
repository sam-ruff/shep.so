use shep::{
    model::{MailQuery, parse_mail},
    store::Store,
};

#[tokio::test]
async fn reopened_cache_counts_unread_accounts_without_mail_row_reads_or_sorting() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.sqlite");
    let store = Store::open(&path).unwrap();
    // Model the earlier cache schema; reopening must add the covering index.
    store
        .run(|c| {
            c.execute_batch("DROP INDEX mail_inbox_badge_counts")?;
            Ok(())
        })
        .await
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    store.run(|c| {
        let steps = c.prepare("EXPLAIN QUERY PLAN SELECT account,COUNT(*) FROM messages WHERE folder='INBOX' AND unread=1 GROUP BY account")?
            .query_map([],|r|r.get::<_,String>(3))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        assert!(steps.iter().any(|s|s.contains("COVERING INDEX")),"{steps:?}");
        assert!(!steps.iter().any(|s|s.contains("TEMP B-TREE")),"{steps:?}");
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn global_counts_observe_pending_mail_outside_the_filtered_page_and_after_rekey() {
    let store = Store::memory().unwrap();
    let first = parse_mail(
        "work",
        "7.1",
        "INBOX",
        b"Subject: First\r\n\r\nBody".to_vec(),
        true,
        false,
    )
    .unwrap();
    let source = first.summary.clone();
    let other = parse_mail(
        "personal",
        "7.2",
        "INBOX",
        b"Subject: Second\r\n\r\nBody".to_vec(),
        true,
        false,
    )
    .unwrap();
    store.upsert(vec![first, other]).await.unwrap();
    let query = MailQuery {
        folder: "Archive".into(),
        search: "absent".into(),
        observe: vec![source.id.clone(), "missing".into()],
        ..Default::default()
    };
    let page = store.query(query.clone()).await.unwrap();
    assert!(page.rows.is_empty());
    assert_eq!(page.inbox_unread.values().sum::<usize>(), 2);
    assert_eq!(page.observed[&source.id].as_ref().unwrap().folder, "INBOX");
    assert_eq!(page.observed["missing"], None);
    let mut moved = source.clone();
    moved.id = "work:Archive:9.4".into();
    moved.remote_id = "9.4".into();
    moved.folder = "Archive".into();
    store
        .relocate_mail(source.clone(), moved.clone())
        .await
        .unwrap();
    let mut query = query;
    query.observe.push(moved.id.clone());
    let page = store.query(query).await.unwrap();
    assert_eq!(page.inbox_unread.values().sum::<usize>(), 1);
    assert_eq!(page.observed[&source.id], None);
    assert_eq!(page.observed[&moved.id].as_ref().unwrap().folder, "Archive");
}

#[tokio::test]
async fn unread_badge_defaults_on_and_disabled_preference_survives_reopen() {
    let old: shep::model::Preferences = serde_json::from_str("{}").unwrap();
    assert!(old.unread_badge);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.sqlite");
    let store = Store::open(&path).unwrap();
    let mut prefs = old;
    prefs.unread_badge = false;
    store.save_preferences(prefs).await.unwrap();
    drop(store);
    let restored: shep::model::Preferences =
        Store::open(path).unwrap().get("preferences").await.unwrap();
    assert!(!restored.unread_badge);
}
