use super::*;
use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use secrecy::SecretString;
use shep_mail_core::{
    mail_actions::{Flags, MoveReceipt},
    model::{Account, Mail, MailSyncItem},
    providers::MailProvider,
};
use std::collections::HashSet;

mockall::mock! {
    Provider {}
    #[async_trait::async_trait]
    impl MailProvider for Provider {
        async fn sync(&self,account:&Account,password:&SecretString,known:&HashSet<String>,output:tokio::sync::mpsc::Sender<MailSyncItem>) -> Result<Vec<String>>;
        async fn move_mail(&self,account:&Account,password:&SecretString,mail:&Mail,folder:&str) -> Result<Option<String>>;
        async fn set_flags(&self,account:&Account,password:&SecretString,mail:&Mail,changes:Flags) -> Result<()>;
        async fn inspect_move(&self,account:&Account,password:&SecretString,receipt:&MoveReceipt) -> Result<Mail>;
        async fn inspect_flags(&self,account:&Account,password:&SecretString,mail:&Mail) -> Result<Flags>;
    }
}

#[tokio::test]
async fn acknowledgement_cache_failure_restart_repairs_before_another_provider_step() -> Result<()>
{
    let (directory, p) = profile().await;
    seed(&p, 2).await;
    make_imap(&p).await;
    let mut provider = MockProvider::new();
    provider
        .expect_move_mail()
        .times(2)
        .returning(|_, _, mail, folder| {
            assert_eq!(folder, "Archive");
            Ok(Some(format!("ack-{}", mail.remote_id)))
        });
    let provider = Arc::new(provider);
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    review(&p, "ack-restart", json!({"kind":"archive"})).await;
    approve(&p, "ack-restart").await;
    p.database.write(|db| {db.execute_batch("CREATE TRIGGER fail_cache BEFORE UPDATE OF folder ON mail BEGIN SELECT RAISE(FAIL,'Synthetic cache failure'); END;")?;Ok(())}).await?;
    let first = step(&p, Some("fixture-only-not-a-real-password")).await;
    assert_eq!(first["outcome"], "repair");
    let row = items(&p, "ack-restart", None).await["rows"][0].clone();
    assert_eq!(row["receipt"]["after"]["folder"], "Archive");
    assert!(
        row["receipt"]["after"]["remote_id"]
            .as_str()
            .context("ack UID")?
            .starts_with("ack-")
    );
    assert_eq!(
        p.database
            .read(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM mail WHERE folder='INBOX'",
                [],
                |row| row.get::<_, i64>(0)
            )?))
            .await?,
        2
    );
    assert_eq!(step(&p, None).await["idle"], true);
    assert_eq!(
        groups(&p, json!({"kind":"history"})).await["runnable"],
        false
    );
    drop(p);
    let p = crate::api::MobileProfile::open(
        directory
            .path()
            .join("mail.sqlite3")
            .to_string_lossy()
            .into(),
    )
    .await?;
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    assert_eq!(
        items(&p, "ack-restart", None).await["rows"][0]["state"],
        "repair"
    );
    assert_eq!(step(&p, None).await["idle"], true);
    p.database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_cache")?;
            Ok(())
        })
        .await?;
    groups(&p, json!({"kind":"retry","id":"ack-restart","position":0})).await;
    let capacity = p.operations.hold_network_capacity().await;
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), step(&p, None)).await?["outcome"],
        "done"
    );
    drop(capacity);
    assert_eq!(
        p.database
            .read(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM mail WHERE folder='Archive'",
                [],
                |row| row.get::<_, i64>(0)
            )?))
            .await?,
        1
    );
    assert_eq!(
        step(&p, Some("fixture-only-not-a-real-password")).await["outcome"],
        "done"
    );
    assert_eq!(page(&p, "Archive").await["total"], 2);
    Ok(())
}

#[tokio::test]
async fn lost_flag_acknowledgement_is_uncertain_and_never_replayed() {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let mut provider = MockProvider::new();
    provider
        .expect_set_flags()
        .times(1)
        .returning(|_, _, _, _| Err(anyhow::anyhow!("Synthetic lost STORE reply")));
    *p.operations.provider.lock().expect("provider") = Some(Arc::new(provider));
    review(&p, "lost-flags", json!({"kind":"read"})).await;
    approve(&p, "lost-flags").await;
    assert_eq!(
        step(&p, Some("fixture-only-not-a-real-password")).await["outcome"],
        "uncertain"
    );
    assert!(
        group_failure(&p, json!({"kind":"retry","id":"lost-flags","position":0}))
            .await
            .contains("unknown")
    );
    assert_eq!(
        step(&p, Some("fixture-only-not-a-real-password")).await["idle"],
        true
    );
}

#[tokio::test]
async fn newer_already_applied_group_choice_survives_original_undo_and_retirement() {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    review(&p, "first-read", json!({"kind":"read"})).await;
    approve(&p, "first-read").await;
    run_all(&p, None).await;
    review(&p, "newer-read", json!({"kind":"read"})).await;
    approve(&p, "newer-read").await;
    assert_eq!(step(&p, None).await["outcome"], "skipped");
    groups(&p, json!({"kind":"remove","id":"newer-read"})).await;
    groups(&p, json!({"kind":"undo","id":"first-read"})).await;
    run_all(&p, None).await;
    assert_eq!(page(&p, "Inbox").await["mail"][0]["unread"], false);
    assert_eq!(job(&p, "first-read").await["counts"]["undo_skipped"], 1);
}

#[tokio::test]
async fn vanished_selected_member_keeps_account_and_is_skipped_without_cancelling_review()
-> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 2).await;
    let captured = select_all(&p, "vanished-source").await;
    p.database
        .write(|db| {
            db.execute("DELETE FROM mail WHERE remote_id='0'", [])?;
            Ok(())
        })
        .await?;
    let frozen=groups(&p,json!({"kind":"prepare","id":"vanished-review","selection":"vanished-source","expected":captured["revision"],"action":{"kind":"read"},"scope":{"folder":"Inbox"}})).await;
    assert_eq!(frozen["total"], 2);
    assert_eq!(frozen["counts"]["skipped"], 1);
    assert_eq!(approve(&p, "vanished-review").await["state"], "running");
    run_all(&p, None).await;
    Ok(())
}

#[tokio::test]
async fn group_children_are_private_and_survive_individual_history_collection() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    review(&p, "private-child", json!({"kind":"archive"})).await;
    approve(&p, "private-child").await;
    step(&p, None).await;
    let child: String = p
        .database
        .read(|db| {
            Ok(db.query_row(
                "SELECT id FROM individual_mail_actions WHERE group_job='private-child'",
                [],
                |row| row.get(0),
            )?)
        })
        .await?;
    let mail = items(&p, "private-child", None).await["rows"][0]["mail"]
        .as_str()
        .context("mail")?
        .to_owned();
    for input in [
        json!({"op":"cancel_mail_action","id":child}),
        json!({"op":"undo_mail_action","id":child}),
        json!({"op":"inspect_mail_action","id":child}),
        json!({"op":"mutate","action_id":child,"id":mail,"folder":"Archive"}),
    ] {
        assert!(failure(&p, input).await.contains("group action"));
    }
    assert_eq!(
        request(&p, json!({"op":"mail_actions"})).await["actions"],
        json!([])
    );
    assert_eq!(
        request(&p, json!({"op":"mail_actions","runnable":true})).await["actions"],
        json!([])
    );
    for index in 0..105 {
        request(&p,json!({"op":"mutate","action_id":format!("unrelated-{index}"),"id":mail,"starred":index%2==0})).await;
    }
    assert_eq!(p.database.read(|db|Ok(db.query_row("SELECT COUNT(*) FROM individual_mail_action_receipts r JOIN individual_mail_actions a ON a.id=r.action WHERE a.group_job='private-child'",[],|row|row.get::<_,i64>(0))?)).await?,1);
    groups(&p, json!({"kind":"undo","id":"private-child"})).await;
    assert_eq!(step(&p, None).await["outcome"], "undone");
    Ok(())
}

#[tokio::test]
async fn captured_same_id_replacement_and_legacy_review_never_gain_source_proof() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    let captured = select_all(&p, "replacement-source").await;
    p.database
        .write(|db| {
            db.execute(
                "UPDATE mail SET raw=?1 WHERE remote_id='0'",
                [b"replacement MIME".to_vec()],
            )?;
            Ok(())
        })
        .await?;
    let frozen=groups(&p,json!({"kind":"prepare","id":"replacement-review","selection":"replacement-source","expected":captured["revision"],"action":{"kind":"read"},"scope":{"folder":"Inbox"}})).await;
    assert_eq!(frozen["counts"]["skipped"], 1);
    approve(&p, "replacement-review").await;
    assert_eq!(step(&p, None).await["idle"], true);
    review(&p, "legacy-review", json!({"kind":"read"})).await;
    p.database
        .write(|db| {
            db.execute(
                "UPDATE group_items SET lineage=NULL WHERE job='legacy-review'",
                [],
            )?;
            Ok(())
        })
        .await?;
    approve(&p, "legacy-review").await;
    let skipped = step(&p, None).await;
    assert_eq!(skipped["outcome"], "skipped");
    assert!(
        skipped["reason"]
            .as_str()
            .context("skip reason")?
            .contains("source proof")
    );
    assert_eq!(
        p.database
            .read(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM individual_mail_actions",
                [],
                |row| row.get::<_, i64>(0)
            )?))
            .await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn inverse_acknowledgement_restarts_as_cache_repair_without_another_move() -> Result<()> {
    let (directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let mut provider = MockProvider::new();
    let mut sequence = mockall::Sequence::new();
    provider
        .expect_move_mail()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_, _, mail, folder| {
            assert_eq!(mail.folder, "INBOX");
            assert_eq!(folder, "Archive");
            Ok(Some("forward-uid".into()))
        });
    provider
        .expect_move_mail()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_, _, mail, folder| {
            assert_eq!(mail.folder, "Archive");
            assert_eq!(mail.remote_id, "forward-uid");
            assert_eq!(folder, "INBOX");
            Ok(Some("inverse-uid".into()))
        });
    let provider = Arc::new(provider);
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    review(&p, "inverse-cache", json!({"kind":"archive"})).await;
    approve(&p, "inverse-cache").await;
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "done");
    groups(&p, json!({"kind":"undo","id":"inverse-cache"})).await;
    p.database.write(|db| {db.execute_batch("CREATE TRIGGER fail_cache BEFORE UPDATE OF folder ON mail BEGIN SELECT RAISE(FAIL,'Synthetic inverse cache failure'); END;")?;Ok(())}).await?;
    assert_eq!(
        step(&p, Some("fixture-only")).await["outcome"],
        "undo_repair"
    );
    let saved = items(&p, "inverse-cache", None).await["rows"][0].clone();
    assert_eq!(saved["receipt"]["after"]["remote_id"], "inverse-uid");
    drop(p);
    let p = crate::api::MobileProfile::open(
        directory
            .path()
            .join("mail.sqlite3")
            .to_string_lossy()
            .into(),
    )
    .await?;
    *p.operations.provider.lock().expect("provider") = Some(provider);
    assert_eq!(
        items(&p, "inverse-cache", None).await["rows"][0]["state"],
        "undo_repair"
    );
    assert_eq!(step(&p, None).await["idle"], true);
    p.database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_cache")?;
            Ok(())
        })
        .await?;
    groups(
        &p,
        json!({"kind":"retry","id":"inverse-cache","position":0}),
    )
    .await;
    assert_eq!(step(&p, None).await["outcome"], "undone");
    assert_eq!(
        p.database
            .read(
                |db| Ok(db.query_row("SELECT remote_id FROM mail", [], |row| row
                    .get::<_, String>(0))?)
            )
            .await?,
        "inverse-uid"
    );
    assert_eq!(step(&p, None).await["idle"], true);
    Ok(())
}

#[tokio::test]
async fn unknown_move_uid_retains_ack_and_uses_read_only_inspection_not_move_replay() -> Result<()>
{
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let source = p
        .database
        .read(|db| {
            let id: String = db.query_row("SELECT id FROM mail", [], |row| row.get(0))?;
            crate::operations::stored_mail(db, &id)
        })
        .await?;
    let mut resolved = source.clone();
    resolved.id = "fixture:Archive:resolved-uid".into();
    resolved.folder = "Archive".into();
    resolved.remote_id = "resolved-uid".into();
    let mut provider = MockProvider::new();
    provider
        .expect_move_mail()
        .times(1)
        .returning(|_, _, _, _| Ok(None));
    provider
        .expect_inspect_move()
        .times(1)
        .returning(move |_, _, receipt| {
            assert!(receipt.current.is_none());
            assert_eq!(receipt.folder, "Archive");
            assert!(receipt.fingerprint.is_some());
            Ok(resolved.clone())
        });
    *p.operations.provider.lock().expect("provider") = Some(Arc::new(provider));
    review(&p, "unknown-uid", json!({"kind":"archive"})).await;
    approve(&p, "unknown-uid").await;
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "repair");
    assert_eq!(
        items(&p, "unknown-uid", None).await["rows"][0]["receipt"]["after"],
        Value::Null
    );
    assert_eq!(step(&p, None).await["requires_credentials"], "fixture");
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "done");
    assert_eq!(
        items(&p, "unknown-uid", None).await["rows"][0]["receipt"]["after"]["remote_id"],
        "resolved-uid"
    );
    Ok(())
}

#[tokio::test]
async fn held_flag_ack_after_undo_preserves_newer_approval_and_cache_repair() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let provider = Scripted::new(vec![Ok(None)]);
    let gate = Arc::new(tokio::sync::Notify::new());
    *provider.gate.lock().expect("gate") = Some(gate.clone());
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    review(&p, "old-flags", json!({"kind":"read"})).await;
    approve(&p, "old-flags").await;
    let held = {
        let p = crate::api::MobileProfile {
            database: p.database.clone(),
            operations: p.operations.clone(),
        };
        tokio::spawn(async move { step(&p, Some("fixture-only")).await })
    };
    provider.started.notified().await;
    groups(&p, json!({"kind":"undo","id":"old-flags"})).await;
    review(&p, "new-flags", json!({"kind":"read"})).await;
    approve(&p, "new-flags").await;
    p.database.write(|db|{db.execute_batch("CREATE TRIGGER fail_flags BEFORE UPDATE OF unread ON mail BEGIN SELECT RAISE(FAIL,'Synthetic flag cache failure'); END;")?;Ok(())}).await?;
    *provider.gate.lock().expect("gate") = None;
    gate.notify_one();
    assert_eq!(held.await?["outcome"], "repair");
    let row = items(&p, "old-flags", None).await["rows"][0].clone();
    assert_eq!(row["receipt"]["after"]["unread"], false);
    assert_eq!(row["state"], "repair");
    assert_eq!(step(&p, Some("fixture-only")).await["idle"], true);
    p.database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_flags")?;
            Ok(())
        })
        .await?;
    groups(&p, json!({"kind":"retry","id":"old-flags","position":0})).await;
    assert_eq!(step(&p, None).await["outcome"], "done");
    assert_eq!(step(&p, None).await["outcome"], "undo_skipped");
    run_all(&p, Some("fixture-only")).await;
    assert_eq!(provider.flags.load(Ordering::SeqCst), 1);
    assert_eq!(page(&p, "Inbox").await["mail"][0]["unread"], false);
    Ok(())
}

#[tokio::test]
async fn flag_cache_rollback_keeps_exact_ack_and_newer_individual_choice() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let mut provider = MockProvider::new();
    let mut sequence = mockall::Sequence::new();
    provider
        .expect_set_flags()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_, _, _, changes| {
            assert_eq!(changes.unread, Some(false));
            Ok(())
        });
    provider
        .expect_set_flags()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_, _, _, changes| {
            assert_eq!(changes.unread, Some(true));
            Ok(())
        });
    *p.operations.provider.lock().expect("provider") = Some(Arc::new(provider));
    review(&p, "flag-cache", json!({"kind":"read"})).await;
    approve(&p, "flag-cache").await;
    p.database.write(|db|{db.execute_batch("CREATE TRIGGER fail_flags BEFORE UPDATE OF unread ON mail BEGIN SELECT RAISE(FAIL,'Synthetic flag cache failure'); END;")?;Ok(())}).await?;
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "repair");
    let acknowledgement=p.database.read(|db|Ok(db.query_row("SELECT r.result FROM individual_mail_action_receipts r JOIN individual_mail_actions a ON a.id=r.action WHERE a.group_job='flag-cache'",[],|row|row.get::<_,String>(0))?)).await?;
    let mail = p
        .database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_flags")?;
            let id: String = db.query_row("SELECT id FROM mail", [], |row| row.get(0))?;
            Ok(id)
        })
        .await?;
    assert_eq!(request(&p,json!({"op":"mutate","action_id":"new-completed-unread","id":mail,"unread":true,"password":"fixture-only"})).await["status"],"succeeded");
    assert_eq!(
        request(
            &p,
            json!({"op":"mutate","action_id":"new-cancelled-read","id":mail,"unread":false})
        )
        .await["status"],
        "waiting"
    );
    request(
        &p,
        json!({"op":"cancel_mail_action","id":"new-cancelled-read"}),
    )
    .await;
    assert_eq!(step(&p, None).await["outcome"], "done");
    assert_eq!(page(&p, "Inbox").await["mail"][0]["unread"], true);
    assert_eq!(p.database.read(|db|Ok(db.query_row("SELECT r.result FROM individual_mail_action_receipts r JOIN individual_mail_actions a ON a.id=r.action WHERE a.group_job='flag-cache'",[],|row|row.get::<_,String>(0))?)).await?,acknowledgement);
    groups(&p, json!({"kind":"undo","id":"flag-cache"})).await;
    assert_eq!(step(&p, None).await["outcome"], "undo_skipped");
    Ok(())
}

#[tokio::test]
async fn owned_provider_failures_never_publish_or_store_private_error_text() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let mut provider = MockProvider::new();
    provider
        .expect_set_flags()
        .times(1)
        .returning(|_, _, _, _| {
            Err(
                shep_mail_core::mail_actions::MoveRefused("private-password-sentinel".into())
                    .into(),
            )
        });
    *p.operations.provider.lock().expect("provider") = Some(Arc::new(provider));
    review(&p, "private-error", json!({"kind":"read"})).await;
    approve(&p, "private-error").await;
    let outcome = step(&p, Some("fixture-only")).await;
    assert_eq!(outcome["outcome"], "failed");
    assert!(!outcome.to_string().contains("private-password-sentinel"));
    assert!(
        !items(&p, "private-error", None)
            .await
            .to_string()
            .contains("private-password-sentinel")
    );
    assert!(
        !groups(&p, json!({"kind":"history"}))
            .await
            .to_string()
            .contains("private-password-sentinel")
    );
    let raw = p
        .database
        .read(|db| {
            Ok(db.query_row(
                "SELECT error FROM individual_mail_actions WHERE group_job='private-error'",
                [],
                |row| row.get::<_, String>(0),
            )?)
        })
        .await?;
    assert!(!raw.contains("private-password-sentinel"));
    Ok(())
}

#[tokio::test]
async fn retirement_drains_at_most_fifty_child_attempts_before_removing_parent() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    review(&p, "many-attempts", json!({"kind":"read"})).await;
    approve(&p, "many-attempts").await;
    step(&p, None).await;
    p.database.write(|db|{
        for index in 0..125 {
            db.execute("INSERT INTO individual_mail_actions(id,mail,account,fields,physical,intent_revision,status,created,group_job,group_position,group_inverse) SELECT ?1,mail,account,fields,physical,intent_revision,'rejected',created,group_job,group_position,group_inverse FROM individual_mail_actions WHERE group_job='many-attempts' LIMIT 1",[format!("old-attempt-{index}")])?;
        }
        Ok(())
    }).await?;
    assert!(
        p.database
            .write(|db| crate::operations::group::retire(db, "many-attempts"))
            .await?
    );
    assert_eq!(
        p.database
            .read(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM individual_mail_actions WHERE group_job='many-attempts'",
                [],
                |row| row.get::<_, i64>(0)
            )?))
            .await?,
        76
    );
    assert_eq!(
        p.database
            .read(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM group_jobs WHERE id='many-attempts'",
                [],
                |row| row.get::<_, i64>(0)
            )?))
            .await?,
        1
    );
    assert!(
        p.database
            .write(|db| crate::operations::group::retire(db, "many-attempts"))
            .await?
    );
    assert!(
        p.database
            .write(|db| crate::operations::group::retire(db, "many-attempts"))
            .await?
    );
    assert!(
        !p.database
            .write(|db| crate::operations::group::retire(db, "many-attempts"))
            .await?
    );
    assert_eq!(
        groups(&p, json!({"kind":"remove","id":"many-attempts"})).await["removed"],
        true
    );
    Ok(())
}

#[tokio::test]
async fn inspection_rechecks_review_connection_after_held_account_ownership() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let mut provider = MockProvider::new();
    provider
        .expect_move_mail()
        .times(1)
        .returning(|_, _, _, _| Ok(None));
    provider.expect_inspect_move().times(0);
    *p.operations.provider.lock().expect("provider") = Some(Arc::new(provider));
    review(&p, "held-inspect", json!({"kind":"archive"})).await;
    approve(&p, "held-inspect").await;
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "repair");
    let attempt: String = p
        .database
        .read(|db| {
            Ok(db.query_row(
                "SELECT attempt FROM group_items WHERE job='held-inspect'",
                [],
                |row| row.get(0),
            )?)
        })
        .await?;
    let owner = p.operations.account("fixture").await;
    let mut held = {
        let p = crate::api::MobileProfile {
            database: p.database.clone(),
            operations: p.operations.clone(),
        };
        tokio::spawn(async move {
            crate::operations::group::inspect(&p, attempt, SecretString::from("fixture-only"), None)
                .await
        })
    };
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), &mut held)
            .await
            .is_err()
    );
    p.database.write(|db|{
        db.execute("UPDATE accounts SET settings=json_set(settings,'$.host','reconnected.example.test') WHERE id='fixture'",[])?;
        Ok(())
    }).await?;
    drop(owner);
    let error = tokio::time::timeout(std::time::Duration::from_secs(1), held)
        .await??
        .expect_err("changed review connection");
    assert!(error.to_string().contains("connection changed"));
    assert_eq!(
        items(&p, "held-inspect", None).await["rows"][0]["state"],
        "repair"
    );
    Ok(())
}

#[tokio::test]
async fn local_child_losing_ownership_after_admission_has_no_success_receipt() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    review(&p, "local-held", json!({"kind":"read"})).await;
    approve(&p, "local-held").await;
    let mail=p.database.write(|db|{
        let (mail,lineage,approved):(String,String,i64)=db.query_row("SELECT i.mail,i.lineage,j.approved FROM group_items i JOIN group_jobs j ON j.id=i.job WHERE i.job='local-held'",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        crate::operations::group::admit(db,crate::operations::group::Admission {job:"local-held",position:0,inverse:false,attempt:"held-local-attempt",mail:&mail,lineage:&lineage,approved,fields:&json!({"unread":false}),credential_slot:None})?;
        db.execute("UPDATE group_items SET state='sending',attempt='held-local-attempt' WHERE job='local-held'",[])?;
        Ok(mail)
    }).await?;
    request(
        &p,
        json!({"op":"mutate","action_id":"new-local-choice","id":mail,"unread":true}),
    )
    .await;
    assert_eq!(
        crate::operations::group::dispatch(&p, "held-local-attempt".into(), None).await?["status"],
        "cancelled"
    );
    assert_eq!(p.database.read(|db|Ok(db.query_row("SELECT COUNT(*) FROM individual_mail_action_receipts WHERE action='held-local-attempt'",[],|row|row.get::<_,i64>(0))?)).await?,0);
    assert_eq!(page(&p, "Inbox").await["mail"][0]["unread"], true);
    Ok(())
}

#[tokio::test]
async fn acknowledged_alias_merge_keeps_proven_source_over_stale_captured_target() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 2).await;
    let page = page(&p, "Inbox").await;
    let target = page["mail"][0]["id"].as_str().context("target")?.to_owned();
    let source = page["mail"][1]["id"].as_str().context("source")?.to_owned();
    let captured = select_all(&p, "alias-selection").await;
    let old_source = source.clone();
    let current_target = target.clone();
    p.database
        .write(move |db| {
            let tx = db.transaction()?;
            let source_raw: Vec<u8> =
                tx.query_row("SELECT raw FROM mail WHERE id=?1", [&old_source], |row| {
                    row.get(0)
                })?;
            tx.execute(
                "UPDATE mail SET raw=?2 WHERE id=?1",
                rusqlite::params![current_target, source_raw],
            )?;
            crate::operations::adopt_action_alias(&tx, &old_source, &current_target)?;
            tx.execute(
                "INSERT INTO mail_aliases VALUES(?1,?2)",
                rusqlite::params![old_source, current_target],
            )?;
            tx.execute("DELETE FROM mail WHERE id=?1", [old_source])?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    let observed=request(&p,json!({"op":"selection","command":{"kind":"observe","id":"alias-selection"},"observed":[source,target]})).await;
    assert_eq!(observed["selected"], 1);
    assert_eq!(observed["available"], 1);
    assert_eq!(observed["positions"][&target], 0);
    let staged=groups(&p,json!({"kind":"prepare","id":"alias-group","selection":"alias-selection","expected":captured["revision"],"action":{"kind":"read"},"scope":{"folder":"Inbox"}})).await;
    assert_eq!(staged["total"], 1);
    assert_eq!(staged["counts"]["skipped"], Value::Null);
    approve(&p, "alias-group").await;
    assert_eq!(step(&p, None).await["outcome"], "skipped");
    assert_eq!(
        items(&p, "alias-group", None).await["rows"][0]["reason"],
        "Already up to date"
    );
    Ok(())
}

#[tokio::test]
async fn unresolved_ack_repair_rejects_replaced_cache_and_modified_fingerprint() -> Result<()> {
    for replace_lineage in [true, false] {
        let (_directory, p) = profile().await;
        seed(&p, 1).await;
        make_imap(&p).await;
        let mut provider = MockProvider::new();
        provider
            .expect_move_mail()
            .times(1)
            .returning(|_, _, _, _| Ok(None));
        provider.expect_inspect_move().times(0);
        *p.operations.provider.lock().expect("provider") = Some(Arc::new(provider));
        review(&p, "replaced-unknown", json!({"kind":"archive"})).await;
        approve(&p, "replaced-unknown").await;
        step(&p, Some("fixture-only")).await;
        p.database.write(move |db|{
            if replace_lineage {
                db.execute("UPDATE mail SET raw=?1",[b"different cache incarnation".to_vec()])?;
            } else {
                db.execute("UPDATE move_receipts SET receipt=json_set(receipt,'$.fingerprint.bytes',999999)",[])?;
                // Retain the original captured lineage but supply a resolved receipt,
                // as if inspection had returned before this cache receipt changed.
                let (action,mut raw):(String,String)=db.query_row("SELECT a.id,r.result FROM individual_mail_actions a JOIN individual_mail_action_receipts r ON r.action=a.id WHERE a.group_job='replaced-unknown'",[],|row|Ok((row.get(0)?,row.get(1)?)))?;
                let mut saved:Value=serde_json::from_str(&raw)?;
                let mail_id:String=db.query_row("SELECT id FROM mail",[],|row|row.get(0))?;
                let mut mail=crate::operations::stored_mail(db,&mail_id)?;
                mail.remote_id="inspected-uid".into();
                saved["receipt"]["current"]=serde_json::to_value(mail)?;
                raw=serde_json::to_string(&saved)?;
                db.execute("UPDATE individual_mail_action_receipts SET result=?2 WHERE action=?1",rusqlite::params![action,raw])?;
            }
            Ok(())
        }).await?;
        assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "repair");
        assert_eq!(
            p.database
                .read(
                    |db| Ok(db.query_row("SELECT remote_id FROM mail", [], |row| row
                        .get::<_, String>(0))?)
                )
                .await?,
            "0"
        );
    }
    Ok(())
}

#[tokio::test]
async fn opposite_newer_group_flag_reaches_provider_after_held_old_acknowledgement() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let provider = Scripted::new(vec![Ok(None), Ok(None)]);
    let gate = Arc::new(tokio::sync::Notify::new());
    *provider.gate.lock().expect("gate") = Some(gate.clone());
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    review(&p, "old-read", json!({"kind":"read"})).await;
    approve(&p, "old-read").await;
    let held = {
        let p = crate::api::MobileProfile {
            database: p.database.clone(),
            operations: p.operations.clone(),
        };
        tokio::spawn(async move { step(&p, Some("fixture-only")).await })
    };
    provider.started.notified().await;
    review(&p, "new-unread", json!({"kind":"unread"})).await;
    approve(&p, "new-unread").await;
    *provider.gate.lock().expect("gate") = None;
    gate.notify_one();
    assert_eq!(held.await?["outcome"], "done");
    assert_eq!(page(&p, "Inbox").await["mail"][0]["unread"], true);
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "done");
    let values = provider.flag_values.lock().expect("flag values");
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].unread, Some(false));
    assert_eq!(values[1].unread, Some(true));
    Ok(())
}

#[tokio::test]
async fn pause_fences_unstarted_child_at_account_wait_and_resume_retains_approval() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let mut provider = MockProvider::new();
    provider
        .expect_move_mail()
        .times(1)
        .returning(|_, _, _, _| Ok(Some("resumed-uid".into())));
    *p.operations.provider.lock().expect("provider") = Some(Arc::new(provider));
    review(&p, "paused-wait", json!({"kind":"archive"})).await;
    approve(&p, "paused-wait").await;
    let approval = p
        .database
        .read(|db| {
            Ok(db.query_row(
                "SELECT approved FROM group_jobs WHERE id='paused-wait'",
                [],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .await?;
    let owner = p.operations.account("fixture").await;
    let held = {
        let p = crate::api::MobileProfile {
            database: p.database.clone(),
            operations: p.operations.clone(),
        };
        tokio::spawn(async move { step(&p, Some("fixture-only")).await })
    };
    let attempt=tokio::time::timeout(std::time::Duration::from_secs(1),async {
        loop {
            let child=p.database.read(|db|Ok(db.query_row("SELECT id FROM individual_mail_actions WHERE group_job='paused-wait' AND status='queued'",[],|row|row.get::<_,String>(0)).optional()?)).await?;
            if let Some(child)=child {return Ok::<_,anyhow::Error>(child);}
            tokio::task::yield_now().await;
        }
    }).await??;
    assert_eq!(
        groups(&p, json!({"kind":"pause","id":"paused-wait"})).await["state"],
        "paused"
    );
    assert_eq!(
        items(&p, "paused-wait", None).await["rows"][0]["state"],
        "pending"
    );
    drop(owner);
    assert_eq!(held.await?["outcome"], "paused");
    assert_eq!(step(&p, Some("fixture-only")).await["idle"], true);
    groups(&p, json!({"kind":"resume","id":"paused-wait"})).await;
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "done");
    assert_eq!(
        p.database
            .read(move |db| Ok(db.query_row(
                "SELECT status FROM individual_mail_actions WHERE id=?1",
                [attempt],
                |row| row.get::<_, String>(0)
            )?))
            .await?,
        "cancelled"
    );
    assert_eq!(
        p.database
            .read(|db| Ok(db.query_row(
                "SELECT approved FROM group_jobs WHERE id='paused-wait'",
                [],
                |row| row.get::<_, i64>(0)
            )?))
            .await?,
        approval
    );
    Ok(())
}

#[tokio::test]
async fn schema26_upgrade_preserves_receipt_bytes_and_does_not_infer_field_completion() -> Result<()>
{
    let (directory, p) = profile().await;
    seed(&p, 1).await;
    review(&p, "legacy-done", json!({"kind":"read"})).await;
    approve(&p, "legacy-done").await;
    step(&p, None).await;
    let saved=p.database.write(|db|{
        db.execute("DELETE FROM individual_mail_action_receipts WHERE action IN (SELECT id FROM individual_mail_actions WHERE group_job IS NOT NULL)",[])?;
        db.execute("DELETE FROM individual_mail_actions WHERE group_job IS NOT NULL",[])?;
        db.execute("UPDATE group_items SET receipt=json_remove(receipt,'$.dispatch.lineage','$.after.lineage') WHERE job='legacy-done'",[])?;
        let raw:String=db.query_row("SELECT receipt FROM group_items WHERE job='legacy-done'",[],|row|row.get(0))?;
        let revision:i64=db.query_row("SELECT revision FROM mail_intents WHERE field='unread'",[],|row|row.get(0))?;
        db.execute_batch("DROP INDEX group_item_origin; ALTER TABLE group_items DROP COLUMN lineage; ALTER TABLE group_items DROP COLUMN connection; ALTER TABLE mail_intents DROP COLUMN applied_revision; PRAGMA user_version=26;")?;
        Ok((raw,revision))
    }).await?;
    drop(p);
    let p = crate::api::MobileProfile::open(
        directory
            .path()
            .join("mail.sqlite3")
            .to_string_lossy()
            .into(),
    )
    .await?;
    let actual=p.database.read(|db|{
        let raw:String=db.query_row("SELECT receipt FROM group_items WHERE job='legacy-done'",[],|row|row.get(0))?;
        let (revision,applied):(i64,i64)=db.query_row("SELECT revision,applied_revision FROM mail_intents WHERE field='unread'",[],|row|Ok((row.get(0)?,row.get(1)?)))?;
        let no_proof:bool=db.query_row("SELECT lineage IS NULL AND connection IS NULL FROM group_items WHERE job='legacy-done'",[],|row|row.get(0))?;
        assert!(no_proof);
        assert_eq!(applied,0);
        assert_eq!(db.query_row("PRAGMA user_version",[],|row|row.get::<_,i64>(0))?,27);
        Ok((raw,revision))
    }).await?;
    assert_eq!(actual, saved);
    groups(&p, json!({"kind":"undo","id":"legacy-done"})).await;
    assert_eq!(step(&p, None).await["outcome"], "undo_skipped");
    assert_eq!(
        items(&p, "legacy-done", None).await["rows"][0]["receipt"],
        serde_json::from_str::<Value>(&saved.0)?
    );
    Ok(())
}

#[tokio::test]
async fn mixed_individual_ack_cancel_and_newer_group_preserve_wire_flag_baseline() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let provider = Scripted::new(vec![Ok(None), Ok(None)]);
    let gate = Arc::new(tokio::sync::Notify::new());
    *provider.gate.lock().expect("gate") = Some(gate.clone());
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    let mail = page(&p, "Inbox").await["mail"][0]["id"]
        .as_str()
        .context("mail")?
        .to_owned();
    let old_mail = mail.clone();
    let held = {
        let p = crate::api::MobileProfile {
            database: p.database.clone(),
            operations: p.operations.clone(),
        };
        tokio::spawn(async move {
            request(&p,json!({"op":"mutate","action_id":"old-individual-read","id":old_mail,"unread":false,"password":"fixture-only"})).await
        })
    };
    provider.started.notified().await;
    assert_eq!(
        request(
            &p,
            json!({"op":"mutate","action_id":"queued-individual-unread","id":mail,"unread":true})
        )
        .await["status"],
        "waiting"
    );
    review(&p, "newer-group-unread", json!({"kind":"unread"})).await;
    approve(&p, "newer-group-unread").await;
    *provider.gate.lock().expect("gate") = None;
    gate.notify_one();
    assert_eq!(held.await?["status"], "succeeded");
    request(
        &p,
        json!({"op":"cancel_mail_action","id":"queued-individual-unread"}),
    )
    .await;
    // The cache retains wire truth while the newer group stays projected.
    assert!(
        !p.database
            .read(move |db| Ok(crate::operations::stored_mail(db, &mail)?.unread))
            .await?
    );
    assert_eq!(page(&p, "Inbox").await["mail"][0]["unread"], true);
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "done");
    let values = provider.flag_values.lock().expect("flag values");
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].unread, Some(false));
    assert_eq!(values[1].unread, Some(true));
    Ok(())
}

#[tokio::test]
async fn legacy_newer_field_choice_survives_upgrade_pending_cancel_and_old_ack_repair() -> Result<()>
{
    let (directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let mut provider = MockProvider::new();
    let mut sequence = mockall::Sequence::new();
    provider
        .expect_set_flags()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_, _, _, flags| {
            assert_eq!(flags.unread, Some(false));
            Ok(())
        });
    provider
        .expect_set_flags()
        .times(2)
        .in_sequence(&mut sequence)
        .returning(|_, _, _, flags| {
            assert_eq!(flags.unread, Some(true));
            Ok(())
        });
    let provider = Arc::new(provider);
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    p.database.write(|db|{db.execute_batch("CREATE TRIGGER fail_legacy_cache BEFORE UPDATE OF unread ON mail BEGIN SELECT RAISE(FAIL,'Synthetic legacy cache gap'); END")?;Ok(())}).await?;
    assert_eq!(request(&p,json!({"op":"mutate","action_id":"legacy-old-ack","id":"fixture:INBOX:0","unread":false,"password":"fixture-only"})).await["status"],"repair");
    p.database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_legacy_cache")?;
            Ok(())
        })
        .await?;
    assert_eq!(request(&p,json!({"op":"mutate","action_id":"legacy-new-completion","id":"fixture:INBOX:0","unread":true,"password":"fixture-only"})).await["status"],"succeeded");
    let exact=p.database.write(|db|{
        let receipt:String=db.query_row("SELECT result FROM individual_mail_action_receipts WHERE action='legacy-old-ack'",[],|row|row.get(0))?;
        let legacy_column:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('mail_intents') WHERE name='legacy_revision')",[],|row|row.get(0))?;
        if legacy_column {db.execute_batch("ALTER TABLE mail_intents DROP COLUMN legacy_revision")?;}
        db.execute_batch("ALTER TABLE mail_intents DROP COLUMN applied_revision; PRAGMA user_version=26")?;
        Ok(receipt)
    }).await?;
    drop(p);
    let p = crate::api::MobileProfile::open(
        directory
            .path()
            .join("mail.sqlite3")
            .to_string_lossy()
            .into(),
    )
    .await?;
    *p.operations.provider.lock().expect("provider") = Some(provider);
    assert_eq!(request(&p,json!({"op":"mutate","action_id":"post-upgrade-cancel","id":"fixture:INBOX:0","unread":false})).await["status"],"waiting");
    request(
        &p,
        json!({"op":"cancel_mail_action","id":"post-upgrade-cancel"}),
    )
    .await;
    assert_eq!(
        request(
            &p,
            json!({"op":"inspect_mail_action","id":"legacy-old-ack"})
        )
        .await["status"],
        "succeeded"
    );
    assert!(
        p.database
            .read(|db| Ok(crate::operations::stored_mail(db, "fixture:INBOX:0")?.unread))
            .await?,
        "Unknown legacy completion must not grant permission to overwrite a newer choice"
    );
    assert_eq!(
        p.database
            .read(|db| Ok(db.query_row(
                "SELECT result FROM individual_mail_action_receipts WHERE action='legacy-old-ack'",
                [],
                |row| row.get::<_, String>(0)
            )?))
            .await?,
        exact
    );
    review(&p, "explicit-legacy-unread", json!({"kind":"unread"})).await;
    approve(&p, "explicit-legacy-unread").await;
    assert_eq!(
        step(&p, Some("fixture-only")).await["outcome"],
        "done",
        "An explicit new choice establishes completion instead of inferring a legacy no-op"
    );
    Ok(())
}

#[tokio::test]
async fn move_cache_repair_alias_keeps_completed_destination_flags_and_rejects_replacement()
-> Result<()> {
    for replaced in [false, true] {
        let (_directory, p) = profile().await;
        seed(&p, 1).await;
        make_imap(&p).await;
        let mut provider = MockProvider::new();
        provider
            .expect_move_mail()
            .times(1)
            .returning(|_, _, _, _| Ok(Some("destination-uid".into())));
        provider
            .expect_set_flags()
            .times(1)
            .returning(|_, _, mail, flags| {
                assert_eq!(mail.remote_id, "destination-uid");
                assert_eq!(mail.folder, "Archive");
                assert_eq!(flags.unread, Some(false));
                Ok(())
            });
        *p.operations.provider.lock().expect("provider") = Some(Arc::new(provider));
        review(&p, "alias-cache", json!({"kind":"archive"})).await;
        approve(&p, "alias-cache").await;
        p.database.write(|db|{db.execute_batch("CREATE TRIGGER fail_alias_move_cache BEFORE UPDATE OF folder ON mail BEGIN SELECT RAISE(FAIL,'Synthetic cache gap'); END")?;Ok(())}).await?;
        assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "repair");
        p.database.write(|db|{
            db.execute("INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) SELECT 'synced-destination',account_id,'destination-uid','Archive',sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw FROM mail",[])?;
            Ok(())
        }).await?;
        assert_eq!(request(&p,json!({"op":"mutate","action_id":"destination-read","id":"synced-destination","unread":false,"password":"fixture-only"})).await["status"],"succeeded");
        p.database
            .write(move |db| {
                db.execute_batch("DROP TRIGGER fail_alias_move_cache")?;
                if replaced {
                    db.execute(
                        "UPDATE mail SET raw=?1 WHERE id='synced-destination'",
                        [b"unrelated replacement".to_vec()],
                    )?;
                }
                Ok(())
            })
            .await?;
        assert_eq!(
            step(&p, None).await["outcome"],
            if replaced { "repair" } else { "done" }
        );
        let source = p
            .database
            .read(|db| crate::operations::stored_mail(db, "fixture:INBOX:0"))
            .await?;
        if replaced {
            assert_eq!(source.folder, "INBOX");
            assert_eq!(
                p.database
                    .read(|db| Ok(db.query_row(
                        "SELECT COUNT(*) FROM mail_aliases WHERE alias='synced-destination'",
                        [],
                        |row| row.get::<_, i64>(0)
                    )?))
                    .await?,
                0
            );
        } else {
            assert_eq!(source.folder, "Archive");
            assert_eq!(source.remote_id, "destination-uid");
            assert!(
                !source.unread,
                "Completed destination choice must agree with merged ownership"
            );
            assert!(
                p.database
                    .read(|db| Ok(db.query_row(
                        "SELECT applied_revision=revision FROM mail_intents WHERE mail='fixture:INBOX:0' AND field='unread'",
                        [],
                        |row| row.get::<_, bool>(0)
                    )?))
                    .await?,
                "The merged destination choice must be recorded as completed"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn alias_folding_keeps_legacy_fence_without_inventing_completion() -> Result<()> {
    let (_directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let mut provider = MockProvider::new();
    provider
        .expect_set_flags()
        .times(1)
        .returning(|_, _, _, flags| {
            assert_eq!(flags.unread, Some(false));
            assert_eq!(flags.starred, None);
            Ok(())
        });
    *p.operations.provider.lock().expect("provider") = Some(Arc::new(provider));
    // The folded copy carries only migrated revisions; the target has a real
    // completed star choice. Both predate the current clock.
    p.database
        .write(|db| {
            let tx = db.transaction()?;
            tx.execute("UPDATE group_clock SET revision=MAX(revision,10) WHERE id=1", [])?;
            tx.execute("INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) SELECT 'legacy-copy',account_id,'legacy-uid','Archive',sender,recipient,subject,preview,timestamp,0,0,attachment_count,body,raw FROM mail WHERE id='fixture:INBOX:0'",[])?;
            tx.execute("INSERT INTO mail_intents(mail,field,revision,legacy_revision) VALUES('legacy-copy','unread',0,7),('legacy-copy','starred',0,7)",[])?;
            tx.execute("INSERT INTO mail_intents(mail,field,revision,applied_revision) VALUES('fixture:INBOX:0','starred',9,9)",[])?;
            crate::operations::adopt_action_alias(&tx, "legacy-copy", "fixture:INBOX:0")?;
            tx.execute("DELETE FROM mail WHERE id='legacy-copy'", [])?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    let merged = p
        .database
        .read(|db| {
            let mail = crate::operations::stored_mail(db, "fixture:INBOX:0")?;
            let fences = db
                .prepare("SELECT field,applied_revision,legacy_revision FROM mail_intents WHERE mail='fixture:INBOX:0' ORDER BY field")?
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok((mail.unread, mail.starred, fences))
        })
        .await?;
    assert_eq!(
        merged,
        (
            false,
            true,
            vec![("starred".into(), 9, 7), ("unread".into(), 0, 7)]
        ),
        "Folding moves the newer legacy value and fence but records no completion"
    );
    review(&p, "legacy-alias-read", json!({"kind":"read"})).await;
    approve(&p, "legacy-alias-read").await;
    assert_eq!(
        step(&p, Some("fixture-only")).await["outcome"],
        "done",
        "A matching cached value under an unknown legacy fence still needs one write"
    );
    review(&p, "legacy-alias-flag", json!({"kind":"flag"})).await;
    approve(&p, "legacy-alias-flag").await;
    let skipped = step(&p, Some("fixture-only")).await;
    assert_eq!(skipped["outcome"], "skipped");
    assert_eq!(skipped["reason"], "Already up to date");
    assert!(
        p.database
            .read(|db| Ok(db.query_row(
                "SELECT applied_revision>legacy_revision FROM mail_intents WHERE mail='fixture:INBOX:0' AND field='unread'",
                [],
                |row| row.get::<_, bool>(0)
            )?))
            .await?,
        "The confirmed write establishes a known baseline"
    );
    Ok(())
}
