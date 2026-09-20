use super::*;

#[tokio::test]
async fn sibling_progress_preserves_group_failure_cooldown_and_other_work() {
    let store = Store::memory().expect("fixture store");
    for id in ["failed", "healthy"] {
        let mail = parse_mail(
            id,
            "1",
            "INBOX",
            b"Subject: Retry\r\n\r\nBody".to_vec(),
            true,
            false,
        )
        .expect("fixture mail");
        let original = mail.summary.clone();
        store.upsert(vec![mail]).await.expect("save mail");
        store
            .start_individual_mail_action(
                id.into(),
                original,
                crate::bulk::Action::Flags(crate::mail_actions::Flags {
                    unread: Some(false),
                    starred: None,
                }),
            )
            .await
            .expect("admit action");
    }
    store
        .action_work_progress(0, "failed".into(), false)
        .await
        .expect("retain failure delay");
    store
        .run(|c| {
            c.execute("UPDATE scratch.action_backoff SET until=unixepoch()+60", [])?;
            Ok(())
        })
        .await
        .expect("hold deterministic deadline");
    store
        .action_work_progress(0, "failed".into(), true)
        .await
        .expect("sibling completes");
    let ready = store
        .next_action_work(0, String::new(), vec![], vec![], false)
        .await
        .expect("read next action")
        .expect("independent action");
    assert_eq!(ready.work.id(), "healthy");
    store
        .run(|c| {
            c.execute("UPDATE scratch.action_backoff SET until=unixepoch()-1", [])?;
            Ok(())
        })
        .await
        .expect("expire deadline");
    assert!(store.expire_action_backoff().await.expect("expire delay"));
    let ready = store
        .next_action_work(0, String::new(), vec![], vec![], false)
        .await
        .expect("read retry")
        .expect("retry action");
    assert_eq!(ready.work.id(), "failed");
}

async fn large_store() -> Store {
    let store = Store::memory().unwrap();
    store.run(|c| {
        let tx = c.transaction()?;
        let original = parse_mail("blocked", "1", "INBOX", b"Subject: Candidate\r\n\r\nBody".to_vec(), true, false)?.summary;
        tx.execute("INSERT INTO bulk_jobs(id,action,source,created) VALUES('large',?,'selection',0)", [serde_json::to_string(&crate::bulk::Action::Flags(crate::mail_actions::Flags { unread: Some(false), starred: None }))?])?;
        tx.execute("WITH RECURSIVE n(x) AS(SELECT 0 UNION ALL SELECT x+1 FROM n WHERE x<99999)
            INSERT INTO bulk_items(job,position,id,original,status)
            SELECT 'large',x,'mail-'||x,json_set(?,'$.id','mail-'||x,'$.account_id',CASE WHEN x IN (99,99999) THEN 'free' ELSE 'blocked' END),'queued' FROM n",
            [serde_json::to_string(&original)?])?;
        tx.commit()?;
        Ok(())
    }).await.unwrap();
    store
}

#[tokio::test]
async fn reopening_version_seven_adds_the_ready_index_without_changing_owned_items() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    let mail = parse_mail(
        "account",
        "1",
        "INBOX",
        b"Subject: Migration\r\n\r\nKeep".to_vec(),
        true,
        false,
    )
    .unwrap();
    let original = mail.summary.clone();
    store.upsert(vec![mail]).await.unwrap();
    store
        .start_individual_mail_action(
            "migration".into(),
            original,
            crate::bulk::Action::Flags(crate::mail_actions::Flags {
                unread: Some(false),
                starred: None,
            }),
        )
        .await
        .unwrap();
    assert!(
        store
            .claim_bulk_item("migration".into())
            .await
            .unwrap()
            .is_some()
    );
    store
        .run(|c| {
            c.execute_batch("DROP INDEX bulk_ready_seek; PRAGMA user_version=7;")?;
            Ok(())
        })
        .await
        .unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.bulk_job("migration".into()).await.unwrap().running, 1);
    store.run(|c| {
        assert_eq!(c.query_row("PRAGMA user_version",[],|r|r.get::<_,u32>(0))?,crate::store::DATABASE_VERSION);
        assert_eq!(c.query_row("SELECT count(*) FROM sqlite_schema WHERE type='index' AND name='bulk_ready_seek'",[],|r|r.get::<_,i64>(0))?,1);
        Ok(())
    }).await.unwrap();
}

#[tokio::test]
async fn ready_key_pages_seek_the_index_without_sorting_or_scanning_large_membership() {
    let store = large_store().await;
    store
        .run(|c| {
            let plan = c
                .prepare(&format!("EXPLAIN QUERY PLAN {MAIL_KEYS}"))?
                .query_map(params!["1large:00000000000000089999", "2"], |r| {
                    r.get::<_, String>(3)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .join("\n");
            assert!(
                plan.contains("SEARCH bulk_items USING INDEX bulk_ready_seek"),
                "{plan}"
            );
            assert!(!plan.contains("TEMP B-TREE"), "{plan}");
            for after in ["", "1large:00000000000000089999"] {
                let mut statement = c.prepare(MAIL_KEYS)?;
                let rows = statement
                    .query_map(params![after, "2"], |r| r.get::<_, i64>(1))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                assert_eq!(rows.len(), 50);
                assert_eq!(rows[0], if after.is_empty() { 0 } else { 90000 });
                assert_eq!(
                    statement.get_status(rusqlite::StatementStatus::FullscanStep),
                    0
                );
                assert_eq!(statement.get_status(rusqlite::StatementStatus::Sort), 0);
                assert!(statement.get_status(rusqlite::StatementStatus::VmStep) < 2500);
            }
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn blocked_pages_continue_after_fifty_and_leave_local_writes_and_later_accounts_available() {
    let store = large_store().await;
    let blocked = vec!["mail:blocked".into()];
    let WorkPage::More(after) = store
        .scan_action_work(0, String::new(), blocked.clone(), vec![], false)
        .await
        .unwrap()
    else {
        panic!("bounded continuation")
    };
    assert_eq!(after, "1large:00000000000000000049");
    let draft = Draft {
        id: "independent-draft".into(),
        body: "Saved between candidate pages".into(),
        ..Default::default()
    };
    store.save_draft(draft.clone()).await.unwrap();
    let WorkPage::Ready(ready) = store
        .scan_action_work(0, after, blocked.clone(), vec![], false)
        .await
        .unwrap()
    else {
        panic!("independent account in following page")
    };
    assert!(matches!(ready.work, Work::Mail { position: 99, .. }));
    assert_eq!(
        store.draft_state().await.unwrap().drafts[0].body,
        draft.body
    );
    let WorkPage::Ready(last) = store
        .scan_action_work(
            0,
            "1large:00000000000000099949".into(),
            blocked,
            vec![],
            false,
        )
        .await
        .unwrap()
    else {
        panic!("last account")
    };
    assert!(matches!(
        last.work,
        Work::Mail {
            position: 99999,
            ..
        }
    ));
    assert!(matches!(
        store
            .scan_action_work(0, last.cursor, vec![], vec![], false)
            .await
            .unwrap(),
        WorkPage::Done
    ));
}

#[tokio::test]
async fn repair_pages_keep_priority_and_reset_revisits_newly_eligible_older_keys() {
    let store = large_store().await;
    store.run(|c| {
        c.execute("UPDATE bulk_items SET status='repair' WHERE job='large' AND position BETWEEN 90000 AND 90099", [])?;
        c.execute("UPDATE bulk_items SET original=json_set(original,'$.account_id','free') WHERE job='large' AND position=90099", [])?;
        Ok(())
    }).await.unwrap();
    let WorkPage::More(after) = store
        .scan_action_work(0, String::new(), vec!["mail:blocked".into()], vec![], true)
        .await
        .unwrap()
    else {
        panic!("repair continuation")
    };
    assert_eq!(after, "0large:00000000000000090049");
    let WorkPage::Ready(repair) = store
        .scan_action_work(0, after, vec!["mail:blocked".into()], vec![], true)
        .await
        .unwrap()
    else {
        panic!("next repair page")
    };
    assert!(matches!(
        repair.work,
        Work::Mail {
            position: 90099,
            ..
        }
    ));
    let WorkPage::More(after) = store
        .scan_action_work(0, repair.cursor, vec!["mail:blocked".into()], vec![], false)
        .await
        .unwrap()
    else {
        panic!("bounded queued page after repair phase")
    };
    let WorkPage::Ready(queued) = store
        .scan_action_work(0, after, vec!["mail:blocked".into()], vec![], false)
        .await
        .unwrap()
    else {
        panic!("queued candidate on following page")
    };
    assert!(matches!(queued.work, Work::Mail { position: 99, .. }));
    let WorkPage::Ready(older) = store
        .scan_action_work(0, String::new(), vec![], vec![], true)
        .await
        .unwrap()
    else {
        panic!("released account revisited")
    };
    assert!(matches!(
        older.work,
        Work::Mail {
            position: 90000,
            ..
        }
    ));
}
