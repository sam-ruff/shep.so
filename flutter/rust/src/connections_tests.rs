use crate::tests::{account, draft, profile, request, seed};
use serde_json::{Value, json};
async fn failure(p: &crate::api::MobileProfile, value: Value) -> String {
    let result: Value = serde_json::from_str(&p.request(value.to_string()).await.unwrap()).unwrap();
    result["error"]
        .as_str()
        .unwrap_or_else(|| panic!("Expected failure: {result}"))
        .into()
}
async fn prepare(p: &crate::api::MobileProfile) -> Value {
    let a = request(p, json!({"op":"accounts"})).await["accounts"][0].clone();
    request(p, json!({"op":"prepare_account","account":a,"expected":a})).await
}
#[tokio::test]
async fn credential_activation_rolls_back_retries_and_survives_restart() {
    let (dir, p) = profile().await;
    seed(&p, 1).await;
    let staged = prepare(&p).await;
    let slot = staged["slot"].as_str().unwrap();
    assert_eq!(
        request(&p, json!({"op":"credential_target","account":account()})).await["slot"],
        "fixture"
    );
    p.database.write(|db|{db.execute_batch("CREATE TRIGGER fail_activation BEFORE INSERT ON account_credentials BEGIN SELECT RAISE(ABORT,'Fixture activation failure'); END;")?;Ok(())}).await.unwrap();
    assert!(
        failure(&p, json!({"op":"activate_account","slot":slot}))
            .await
            .contains("Fixture activation failure")
    );
    assert_eq!(
        request(&p, json!({"op":"credential_target","account":account()})).await["slot"],
        "fixture"
    );
    assert_eq!(
        request(&p, json!({"op":"credential_cleanup"})).await,
        json!([slot])
    );
    p.database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_activation")?;
            Ok(())
        })
        .await
        .unwrap();
    request(&p, json!({"op":"activate_account","slot":slot})).await;
    request(&p, json!({"op":"activate_account","slot":slot})).await;
    assert_eq!(
        request(&p, json!({"op":"credential_cleanup"})).await,
        json!(["fixture"])
    );
    assert!(
        failure(&p, json!({"op":"credential_cleanup_done","id":slot}))
            .await
            .contains("refused")
    );
    request(&p, json!({"op":"credential_cleanup_done","id":"fixture"})).await;
    assert!(
        failure(&p, json!({"op":"save_account","account":account()}))
            .await
            .contains("binding")
    );
    drop(p);
    let p =
        crate::api::MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
            .await
            .unwrap();
    assert_eq!(
        request(&p, json!({"op":"credential_target","account":account()})).await["slot"],
        slot
    );
    let review = request(&p, json!({"op":"account_removal_preview","id":"fixture"})).await;
    request(&p, json!({"op":"remove_account","review":review})).await;
    let cleanup = request(&p, json!({"op":"credential_cleanup"})).await;
    assert!(cleanup.as_array().unwrap().contains(&json!(slot)));
    assert!(
        failure(&p, json!({"op":"activate_account","slot":slot}))
            .await
            .contains("removed")
    );
    for id in cleanup.as_array().unwrap() {
        request(&p, json!({"op":"credential_cleanup_done","id":id})).await;
    }
    assert_eq!(
        request(&p, json!({"op":"credential_cleanup"})).await,
        json!([])
    );
}
#[tokio::test]
async fn changed_settings_or_another_activation_cannot_commit_a_stale_pair() {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    let a = prepare(&p).await;
    let b = prepare(&p).await;
    let removal = request(&p, json!({"op":"account_removal_preview","id":"fixture"})).await;
    request(&p, json!({"op":"activate_account","slot":a["slot"]})).await;
    assert!(
        failure(&p, json!({"op":"remove_account","review":removal}))
            .await
            .contains("Local data changed")
    );
    assert!(
        failure(&p, json!({"op":"activate_account","slot":b["slot"]}))
            .await
            .contains("changed")
    );
    // A failed legacy cleanup must still allow a fresh explicit reconnect.
    let c = prepare(&p).await;
    request(
        &p,
        json!({"op":"save_sent_preferences","id":"fixture","policy":"LocalOnly","folder":"Changed"}),
    )
    .await;
    assert!(
        failure(&p, json!({"op":"activate_account","slot":c["slot"]}))
            .await
            .contains("changed")
    );
    let d = prepare(&p).await;
    assert_eq!(d["account"]["sent_copy"], "LocalOnly");
    request(&p, json!({"op":"activate_account","slot":d["slot"]})).await;
    let mut changed = account();
    changed.smtp_host = "another.example.test".into();
    assert!(
        failure(
            &p,
            json!({"op":"prepare_account","account":changed,"expected":account()})
        )
        .await
        .contains("keeps")
    );
    let hold = p.operations.account("fixture").await;
    assert!(
        failure(
            &p,
            json!({"op":"prepare_account","account":account(),"expected":account()})
        )
        .await
        .contains("progress")
    );
    drop(hold);
}
#[tokio::test]
async fn stale_bindings_refuse_sync_move_and_send_before_journaling() {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    p.database
        .write(|db| {
            db.execute(
                "UPDATE accounts SET settings=json_set(settings,'$.protocol','Imap')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    request(
        &p,
        json!({"op":"save_draft","draft":draft(1,"Binding test")}),
    )
    .await;
    let staged = prepare(&p).await;
    request(&p, json!({"op":"activate_account","slot":staged["slot"]})).await;
    for stale in [Value::Null, json!("fixture"), json!("obsolete")] {
        for payload in [
            json!({"op":"sync","account":"fixture","password":"fixture-secret","credential_slot":stale}),
            json!({"op":"mutate","id":"fixture:INBOX:0","folder":"Archive","password":"fixture-secret","credential_slot":stale}),
            json!({"op":"send","id":"draft-one","revision":1,"password":"fixture-secret","credential_slot":stale}),
        ] {
            assert!(failure(&p, payload).await.contains("credentials changed"));
        }
    }
    p.database
        .read(|db| {
            for table in ["pending_moves", "outgoing"] {
                let n: i64 =
                    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
                assert_eq!(n, 0);
            }
            Ok(())
        })
        .await
        .unwrap();
    // Hold the production account lock, queue a mutation with the old binding,
    // commit a reconnect while owning that lock, then release the queued work.
    let next = prepare(&p).await;
    let guard = p.operations.account("fixture").await;
    let second = crate::api::MobileProfile {
        database: p.database.clone(),
        operations: p.operations.clone(),
    };
    let old = staged["slot"].clone();
    let task = tokio::spawn(async move {
        failure(&second,json!({"op":"mutate","id":"fixture:INBOX:0","starred":true,"password":"fixture-secret","credential_slot":old})).await
    });
    p.operations.mutation_waiting.notified().await;
    let slot = next["slot"].as_str().unwrap().to_owned();
    p.database
        .write(move |db| crate::connections::activate(db, &slot))
        .await
        .unwrap();
    drop(guard);
    assert!(task.await.unwrap().contains("credentials changed"));
}

#[tokio::test]
async fn version_seven_upgrade_preserves_legacy_credentials_and_new_accounts_activate_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.sqlite3");
    let db = rusqlite::Connection::open(&path).unwrap();
    let old = include_str!("schema.sql")
        .split("CREATE TABLE IF NOT EXISTS credential_slots")
        .next()
        .unwrap();
    db.execute_batch(&format!("{old}PRAGMA user_version=7;COMMIT;"))
        .unwrap();
    let mut legacy = account();
    legacy.smtp_security = None;
    db.execute(
        "INSERT INTO accounts VALUES(?1,?2)",
        rusqlite::params![legacy.id, serde_json::to_string(&legacy).unwrap()],
    )
    .unwrap();
    drop(db);
    let p = crate::api::MobileProfile::open(path.to_str().unwrap().into())
        .await
        .unwrap();
    assert_eq!(
        request(&p, json!({"op":"credential_target","account":account()})).await["slot"],
        "fixture"
    );
    assert_eq!(
        request(&p, json!({"op":"credential_cleanup"})).await,
        json!([])
    );
    let mut fresh = account();
    fresh.id = "new-account".into();
    let pending = request(&p, json!({"op":"prepare_account","account":fresh})).await;
    assert_eq!(
        request(&p, json!({"op":"accounts"})).await["accounts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    request(&p, json!({"op":"activate_account","slot":pending["slot"]})).await;
    request(&p, json!({"op":"activate_account","slot":pending["slot"]})).await;
    assert_eq!(
        request(&p, json!({"op":"accounts"})).await["accounts"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        request(&p, json!({"op":"credential_cleanup"})).await,
        json!([])
    );
    let staged = prepare(&p).await;
    request(&p, json!({"op":"activate_account","slot":staged["slot"]})).await;
}

struct Connected(std::sync::atomic::AtomicUsize);
#[async_trait::async_trait]
impl shep_mail_core::providers::MailProvider for Connected {
    async fn sync(
        &self,
        account: &shep_mail_core::model::Account,
        password: &secrecy::SecretString,
        _known: &std::collections::HashSet<String>,
        _output: tokio::sync::mpsc::Sender<shep_mail_core::model::MailSyncItem>,
    ) -> anyhow::Result<Vec<String>> {
        use secrecy::ExposeSecret;
        assert_eq!(account.id, "fixture");
        assert_eq!(password.expose_secret(), "fixture-current-secret");
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(vec![])
    }
    async fn move_mail(
        &self,
        _a: &shep_mail_core::model::Account,
        _p: &secrecy::SecretString,
        _m: &shep_mail_core::model::Mail,
        _folder: &str,
    ) -> anyhow::Result<Option<String>> {
        panic!("Unexpected move")
    }
    async fn set_flags(
        &self,
        _a: &shep_mail_core::model::Account,
        _p: &secrecy::SecretString,
        _m: &shep_mail_core::model::Mail,
        _changes: shep_mail_core::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        panic!("Unexpected flags")
    }
}
#[tokio::test]
async fn current_binding_reaches_provider_and_stale_sent_recovery_does_not() {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    let staged = prepare(&p).await;
    request(&p, json!({"op":"activate_account","slot":staged["slot"]})).await;
    let provider = std::sync::Arc::new(Connected(std::sync::atomic::AtomicUsize::new(0)));
    *p.operations.provider.lock().unwrap() = Some(provider.clone());
    let synced=request(&p,json!({"op":"sync","account":"fixture","credential_slot":staged["slot"],"password":"fixture-current-secret"})).await;
    assert_eq!(synced["synced"], true);
    assert_eq!(provider.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    p.database.write(|db|{db.execute("INSERT INTO outgoing VALUES('sent-attempt','sent-draft','delivered','fixture','<fixture@example.test>',X'','{}')",[])?;Ok(())}).await.unwrap();
    assert!(failure(&p,json!({"op":"sent_outgoing","id":"sent-attempt","copy":true,"confirmed":true,"password":"fixture-obsolete-secret","credential_slot":"fixture"})).await.contains("credentials changed"));
    assert_eq!(provider.0.load(std::sync::atomic::Ordering::SeqCst), 1);
}
