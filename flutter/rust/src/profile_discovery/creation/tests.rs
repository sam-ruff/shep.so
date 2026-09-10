use super::*;
use crate::tests::{account, profile};
fn scope() -> Scope {
    Scope {
        namespace: "so.shep.fixture".into(),
        principal: "drive:fixture-owner".into(),
    }
}
fn specification() -> Specification {
    Specification {
        name: "Work".into(),
        include_accounts: true,
        settings: BTreeMap::from([
            (SettingKey::Appearance, serde_json::json!("Dark")),
            (SettingKey::LeftSwipe, serde_json::json!("archive")),
        ]),
    }
}
#[tokio::test]
async fn frozen_review_retries_keep_ids_and_reject_changed_local_accounts_or_settings() {
    let (_dir, profile) = profile().await;
    profile
        .database
        .write(|db| {
            let account = account();
            db.execute(
                "INSERT INTO accounts VALUES(?,?)",
                params![account.id, serde_json::to_string(&account)?],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let id = Uuid::new_v4();
    let s = specification();
    let clone = s.clone();
    let review = profile
        .database
        .write(move |db| prepare(db, scope(), id, clone))
        .await
        .unwrap();
    assert_eq!(review.accounts, 1);
    assert_eq!(review.settings, 2);
    assert_eq!(review.total, 4);
    let clone = s.clone();
    let retry = profile
        .database
        .write(move |db| prepare(db, scope(), id, clone))
        .await
        .unwrap();
    assert_eq!(review.binding, retry.binding);
    let key = scope().storage_key().unwrap();
    let wrong = BTreeMap::from([(SettingKey::Appearance, serde_json::json!("Light"))]);
    let key1 = key.clone();
    assert!(
        profile
            .database
            .write(move |db| approve(db, &key1, id, wrong))
            .await
            .is_err()
    );
    profile
        .database
        .write(|db| {
            let mut account = account();
            account.name = "New local name".into();
            db.execute(
                "UPDATE accounts SET settings=?",
                [serde_json::to_string(&account)?],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let clone = s.clone();
    let key1 = key.clone();
    assert!(
        profile
            .database
            .write(move |db| approve(db, &key1, id, clone.settings))
            .await
            .is_err()
    );
    profile
        .database
        .write(|db| {
            db.execute(
                "UPDATE accounts SET settings=?",
                [serde_json::to_string(&account())?],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let approved = profile
        .database
        .write(move |db| approve(db, &key, id, s.settings))
        .await
        .unwrap();
    assert_eq!(approved.phase, "staging");
    let mapping: Vec<(String, String)> = profile
        .database
        .read(|db| {
            Ok(db
                .prepare("SELECT local_id,shared_id FROM profile_account_mappings")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
        .unwrap();
    assert_eq!(mapping.len(), 1);
    assert_eq!(mapping[0].0, "fixture");
    assert!(Uuid::parse_str(&mapping[0].1).is_ok());
}
struct LocalReceipt;
#[async_trait::async_trait]
impl Remote for LocalReceipt {
    fn scope(&self) -> Scope {
        scope()
    }
    async fn advance(
        &self,
        _catalog: &Discovery,
    ) -> Result<shep_profile_core::drive::catalog::State> {
        anyhow::bail!("Not a discovery provider")
    }
    async fn publish(&self, worker: &Worker, _catalog: &Discovery) -> Result<Option<Uuid>> {
        let Reply::Upload(upload) = worker.request(HistoryCommand::NextUpload).await? else {
            panic!()
        };
        let Some(upload) = upload else {
            return Ok(None);
        };
        // Object-scoped fixture; actual Google receipt ordering has wire tests.
        let id = upload.operation.to_string();
        worker
            .request(HistoryCommand::Reserve {
                operation: upload.operation,
                file_id: id.clone(),
            })
            .await?;
        worker
            .request(HistoryCommand::Confirm {
                operation: upload.operation,
                file_id: id,
                sha256: upload.sha256,
            })
            .await?;
        Ok(Some(upload.operation))
    }
}
#[tokio::test]
async fn native_publication_stages_bounded_records_resumes_exact_edits_and_keeps_mail_unchanged() {
    let (dir, profile) = profile().await;
    let db = &profile.database;
    db.write(|db| {
        for n in 0..75 {
            let mut account = account();
            account.id = format!("legacy-{n:03}");
            db.execute(
                "INSERT INTO accounts VALUES(?,?)",
                params![account.id, serde_json::to_string(&account)?],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let id = Uuid::new_v4();
    let review = db
        .write(move |db| prepare(db, scope(), id, specification()))
        .await
        .unwrap();
    assert_eq!(review.accounts, 75);
    assert_eq!(review.total, 78);
    let key = scope().storage_key().unwrap();
    let key1 = key.clone();
    db.write(move |db| approve(db, &key1, id, specification().settings))
        .await
        .unwrap();
    let catalog = Discovery::open(dir.path().join("catalog.sqlite"), scope())
        .await
        .unwrap();
    let page = run(
        db,
        scope(),
        &LocalReceipt,
        &catalog,
        Command::Accounts { id, after: 0 },
    )
    .await
    .unwrap();
    assert_eq!(page.as_array().unwrap().len(), 50);
    // Fail the mail-cache receipt after the independent history commit.
    db.write(|db|{db.execute_batch("CREATE TRIGGER fail_stage BEFORE UPDATE ON profile_publications WHEN json_extract(NEW.review,'$.staged')=1 BEGIN SELECT RAISE(ABORT,'fixture receipt failure'); END;")?;Ok(())}).await.unwrap();
    assert!(
        step(db, key.clone(), id, &LocalReceipt, &catalog)
            .await
            .is_err()
    );
    db.write(|db| {
        db.execute_batch("DROP TRIGGER fail_stage")?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        step(db, key.clone(), id, &LocalReceipt, &catalog)
            .await
            .unwrap()
            .staged,
        1
    );
    for expected in 2..=78 {
        let state = step(db, key.clone(), id, &LocalReceipt, &catalog)
            .await
            .unwrap();
        assert_eq!(state.staged, expected);
    }
    for expected in 1..=78 {
        let state = step(db, key.clone(), id, &LocalReceipt, &catalog)
            .await
            .unwrap();
        assert_eq!(state.uploaded, expected);
    }
    let complete = step(db, key.clone(), id, &LocalReceipt, &catalog)
        .await
        .unwrap();
    assert_eq!(complete.phase, "complete");
    assert_eq!(
        db.read(
            |db| Ok(db.query_row("SELECT count(*) FROM accounts", [], |r| r.get::<_, i64>(0))?)
        )
        .await
        .unwrap(),
        75
    );
    assert_eq!(
        db.read(|db| Ok(
            db.query_row("SELECT count(*) FROM credential_slots", [], |r| r
                .get::<_, i64>(0))?
        ))
        .await
        .unwrap(),
        0
    );
    catalog.close().await.unwrap();
}
