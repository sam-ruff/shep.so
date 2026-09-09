use super::*;
use crate::{
    folder_actions::Action,
    folders::{Mailbox, NameEncoding},
};

async fn fixture() -> Store {
    let store = Store::memory().unwrap();
    let account: Account = serde_json::from_value(serde_json::json!({"id":"a","name":"Fixture","email":"fixture@example.test","protocol":"Imap","host":"localhost","port":993,"username":"fixture","smtp_host":"localhost","smtp_port":465})).unwrap();
    store.save_account(account).await.unwrap();
    store
        .save_folder_catalog(
            "a".into(),
            ["INBOX", "Projects", "Projects/Design", "Teams"]
                .into_iter()
                .map(|name| Mailbox {
                    delimiter: Some('/'),
                    encoding: NameEncoding::Utf8,
                    ..Mailbox::flat(name.into())
                })
                .collect(),
        )
        .await
        .unwrap();
    let mut mail = Vec::new();
    for index in 0..80 {
        mail.push(
            parse_mail(
                "a",
                &format!("42.{index}"),
                "Projects",
                format!("Subject: Target {index:03}\r\n\r\nFictional scoped contents").into_bytes(),
                index % 2 == 0,
                index % 2 == 0,
            )
            .unwrap(),
        );
    }
    mail.push(
        parse_mail(
            "a",
            "42.child",
            "Projects/Design",
            b"Subject: Child\r\n\r\nFictional scoped contents".to_vec(),
            true,
            true,
        )
        .unwrap(),
    );
    for index in 0..4096 {
        mail.push(
            parse_mail(
                "a",
                &format!("42.other{index}"),
                &format!("Other {index:04}"),
                b"Subject: Unrelated\r\n\r\nFictional unrelated contents".to_vec(),
                true,
                false,
            )
            .unwrap(),
        );
    }
    store.upsert(mail).await.unwrap();
    store
}

async fn capture(store: &Store, query: MailQuery) -> (String, MailPage) {
    let review = Arc::new(
        store
            .folder_review("a".into(), "Projects".into(), Action::Delete)
            .await
            .unwrap(),
    );
    let token = uuid::Uuid::new_v4().to_string();
    let page = store
        .query_folder_projection(query, Some((token.clone(), 1, review)))
        .await
        .unwrap();
    (token, page)
}

#[tokio::test]
async fn affected_counts_stay_scalar_with_thousands_of_unrelated_folders() {
    let store = fixture().await;
    let query = MailQuery {
        folder: String::new(),
        ..Default::default()
    };
    let page = store.query(query.clone()).await.unwrap();
    assert_eq!((page.total, page.unread, page.rows.len()), (4177, 4137, 50));
    assert_eq!(page.folder_count, None);
    let (token, projected) = capture(&store, query).await;
    assert_eq!(
        (projected.total, projected.unread, projected.rows.len()),
        (4177, 4137, 50)
    );
    assert_eq!(projected.folder_count, Some((81, 41)));
    store.release_folder_projection(token).await.unwrap();
    store
        .run(|c| {
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM scratch.folder_projection_scopes",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                0
            );
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM scratch.folder_projection_folders",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn affected_counts_follow_current_flags_filters_and_partial_cache_deletion() {
    let store = fixture().await;
    let query = MailQuery {
        folder: String::new(),
        search: "scoped".into(),
        unread_only: true,
        starred_only: true,
        ..Default::default()
    };
    let (token, page) = capture(&store, query).await;
    assert_eq!(page.folder_count, Some((41, 41)));
    store.run(move |c| bind(c, &token, "job")).await.unwrap();
    let child = store
        .query(MailQuery {
            account: Some("a".into()),
            folder: "Projects/Design".into(),
            ..Default::default()
        })
        .await
        .unwrap()
        .rows
        .remove(0);
    store.remove(child.id).await.unwrap();
    assert_eq!(
        store.folder_projection_counts("job".into()).await.unwrap(),
        Some((40, 40))
    );
    let mut mail = store
        .query(MailQuery {
            account: Some("a".into()),
            folder: "Projects".into(),
            unread_only: true,
            ..Default::default()
        })
        .await
        .unwrap()
        .rows
        .remove(0);
    mail.unread = false;
    store.flags(mail.clone()).await.unwrap();
    assert_eq!(
        store.folder_projection_counts("job".into()).await.unwrap(),
        Some((39, 39))
    );
    mail.unread = true;
    store.flags(mail).await.unwrap();
    assert_eq!(
        store.folder_projection_counts("job".into()).await.unwrap(),
        Some((40, 40))
    );
}

#[tokio::test]
async fn affected_count_plans_never_create_temporary_grouping_btrees() {
    let store = fixture().await;
    let query = MailQuery {
        folder: String::new(),
        ..Default::default()
    };
    let (token, _) = capture(&store, query.clone()).await;
    store
        .run(move |c| {
            read_moves::prepare(c, &[])?;
            let plan = mail_query::Plan::new(c, &query)?;
            let (sql, values) = plan.affected_query(&token);
            for source in [
                "messages",
                "visible_mail",
                "recovered_mail",
                "recovered_bulk",
                "read_visible_mail",
                "read_visible_bulk",
                "read_recovered_mail",
                "read_recovered_bulk",
            ] {
                let sql = sql.replace(
                    "FROM messages AS messages",
                    &format!("FROM {source} AS messages"),
                );
                let explanation: Vec<String> = c
                    .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?
                    .query_map(rusqlite::params_from_iter(&values), |row| row.get(3))?
                    .collect::<rusqlite::Result<_>>()?;
                assert!(
                    !explanation.iter().any(
                        |line| line.contains("USE TEMP B-TREE") || line.contains("MATERIALIZE")
                    ),
                    "{source}: {explanation:?}"
                );
                assert!(
                    explanation
                        .iter()
                        .any(|line| line.contains("folder_projection_folders")
                            && line.contains("INDEX")),
                    "{source}: {explanation:?}"
                );
                let _: (i64, i64) =
                    c.query_row(&sql, rusqlite::params_from_iter(&values), |row| {
                        Ok((row.get(0)?, row.get(1)?))
                    })?;
            }
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn folder_projection_replacement_and_late_release_preserve_newer_review() {
    let store = fixture().await;
    let (old, _) = capture(&store, MailQuery::default()).await;
    let review = Arc::new(
        store
            .folder_review("a".into(), "Projects".into(), Action::Delete)
            .await
            .unwrap(),
    );
    store
        .query_folder_projection(
            MailQuery::default(),
            Some(("new".into(), 3, review.clone())),
        )
        .await
        .unwrap();
    assert!(
        store
            .query_folder_projection(MailQuery::default(), Some(("late".into(), 2, review)))
            .await
            .is_err()
    );
    store.release_folder_projection(old).await.unwrap();
    store
        .run(|c| {
            assert_eq!(
                c.query_row(
                    "SELECT token FROM scratch.folder_projection_scopes",
                    [],
                    |row| row.get::<_, String>(0)
                )?,
                "new"
            );
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM scratch.folder_projection_folders",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                2
            );
            bind(c, "new", "job")?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        store.finish_folder_projection("job".into()).await.unwrap(),
        Some((81, 41))
    );
    store
        .run(|c| {
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM scratch.folder_projection_scopes",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                0
            );
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM scratch.folder_projection_folders",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn folder_projection_count_error_releases_snapshot_without_changing_job() {
    let store = fixture().await;
    let (token, _) = capture(&store, MailQuery::default()).await;
    let review = store
        .folder_review("a".into(), "Projects".into(), Action::Delete)
        .await
        .unwrap();
    let job = store
        .start_folder_change_scoped("job".into(), review, Some(token))
        .await
        .unwrap();
    store.run(|c| {
        c.execute("UPDATE scratch.folder_projection_scopes SET query='invalid fixture query' WHERE job='job'", [])?;
        Ok(())
    }).await.unwrap();
    assert!(store.finish_folder_projection("job".into()).await.is_err());
    assert_eq!(
        store.folder_projection_counts("job".into()).await.unwrap(),
        None
    );
    let current = store.folder_jobs(0).await.unwrap().remove(0);
    assert_eq!(current.revision, job.revision);
    assert_eq!(current.closed, job.closed);
    assert_eq!(
        current
            .steps
            .iter()
            .map(|step| &step.status)
            .collect::<Vec<_>>(),
        job.steps
            .iter()
            .map(|step| &step.status)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn folder_projection_disk_snapshot_is_encrypted_and_disappears_on_restart() {
    use crate::cache_cipher::Key;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let key = Arc::new(Key::generate().unwrap());
    let store = Store::open_encrypted(&path, key.clone()).unwrap();
    let source = fixture().await;
    let review = Arc::new(
        source
            .folder_review("a".into(), "Projects".into(), Action::Delete)
            .await
            .unwrap(),
    );
    let query = MailQuery {
        folder: "confidential-folder-fixture".into(),
        ..Default::default()
    };
    store
        .query_folder_projection(query, Some(("snapshot".into(), 1, review)))
        .await
        .unwrap();
    let scratch = store
        .run(|c| {
            assert_eq!(
                c.query_row("PRAGMA scratch.cache_size", [], |row| row.get::<_, i64>(0))?,
                -2048
            );
            Ok(std::path::PathBuf::from(c.query_row(
                "SELECT file FROM pragma_database_list WHERE name='scratch'",
                [],
                |row| row.get::<_, String>(0),
            )?))
        })
        .await
        .unwrap();
    let bytes = std::fs::read(&scratch).unwrap();
    assert!(!bytes.starts_with(b"SQLite format 3\0"));
    assert!(
        !bytes
            .windows(b"confidential-folder-fixture".len())
            .any(|part| part == b"confidential-folder-fixture")
    );
    drop(store);
    let reopened = Store::open_encrypted(&path, key).unwrap();
    reopened
        .run(|c| {
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM scratch.folder_projection_scopes",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                0
            );
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM scratch.folder_projection_folders",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn folder_projection_admission_is_bounded_and_completion_frees_capacity() {
    let store = fixture().await;
    let review = store
        .folder_review("a".into(), "Projects".into(), Action::Delete)
        .await
        .unwrap();
    store
        .run(move |c| {
            let tx = c.transaction()?;
            for index in 0..32 {
                let token = format!("snapshot-{index}");
                let job = format!("job-{index}");
                // Distinct fictional accounts exercise the existing per-account
                // job constraint without making provider calls.
                tx.execute(
                    "INSERT INTO folder_jobs(id,account,review,created) VALUES(?1,?1,'{}',0)",
                    [&job],
                )?;
                super::capture(&tx, &token, &review, index, &MailQuery::default())?;
                bind(&tx, &token, &job)?;
            }
            super::capture(&tx, "next", &review, 32, &MailQuery::default())?;
            assert!(bind(&tx, "next", "job-next").is_err());
            assert_eq!(
                tx.query_row(
                    "SELECT COUNT(*) FROM scratch.folder_projection_scopes WHERE job IS NOT NULL",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                32
            );
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        store
            .finish_folder_projection("job-0".into())
            .await
            .unwrap(),
        Some((81, 41))
    );
    store
        .run(|c| {
            bind(c, "next", "job-next")?;
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM scratch.folder_projection_scopes",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                32
            );
            assert_eq!(
                c.query_row(
                    "SELECT COUNT(*) FROM scratch.folder_projection_folders",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                64
            );
            Ok(())
        })
        .await
        .unwrap();
}
