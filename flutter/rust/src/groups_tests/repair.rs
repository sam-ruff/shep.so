//! Review regressions for acknowledged group steps whose local result is not
//! saved yet, and for replies that never reached the provider.
use super::*;
use anyhow::Result;
use std::collections::HashSet;

async fn fail_folder_cache(p: &crate::api::MobileProfile) -> Result<()> {
    p.database
        .write(|db| {
            db.execute_batch("CREATE TRIGGER review_fail_cache BEFORE UPDATE OF folder ON mail BEGIN SELECT RAISE(FAIL,'Synthetic cache failure'); END;")?;
            Ok(())
        })
        .await
}
async fn drop_folder_failure(p: &crate::api::MobileProfile) -> Result<()> {
    p.database
        .write(|db| {
            db.execute_batch("DROP TRIGGER review_fail_cache")?;
            Ok(())
        })
        .await
}
async fn scalar<T: rusqlite::types::FromSql + Send + 'static>(
    p: &crate::api::MobileProfile,
    sql: &'static str,
) -> Result<T> {
    p.database
        .read(move |db| Ok(db.query_row(sql, [], |row| row.get(0))?))
        .await
}

/// Admits an attempt for the only item of `job` without dispatching it, as a
/// step does just before its provider call.
async fn admit_only(
    p: &crate::api::MobileProfile,
    job: &'static str,
    attempt: &'static str,
) -> Result<()> {
    p.database
        .write(move |db| {
            let (mail, lineage, approved): (String, String, i64) = db.query_row(
                "SELECT i.mail,i.lineage,j.approved FROM group_items i JOIN group_jobs j ON j.id=i.job WHERE i.job=?1",
                [job],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            crate::operations::group::admit(
                db,
                crate::operations::group::Admission {
                    job,
                    position: 0,
                    inverse: false,
                    attempt,
                    mail: &mail,
                    lineage: &lineage,
                    approved,
                    fields: &json!({"unread":false}),
                    credential_slot: None,
                },
            )?;
            db.execute(
                "UPDATE group_items SET state='sending',attempt=?2 WHERE job=?1",
                rusqlite::params![job, attempt],
            )?;
            Ok(())
        })
        .await
}

#[tokio::test]
async fn repair_item_keeps_acknowledged_projection() -> Result<()> {
    let (_dir, p) = profile().await;
    seed(&p, 2).await;
    make_imap(&p).await;
    let provider = Scripted::new(vec![]);
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    review(&p, "projection", json!({"kind":"archive"})).await;
    approve(&p, "projection").await;
    assert_eq!(page(&p, "Inbox").await["total"], 0);
    fail_folder_cache(&p).await?;
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "repair");
    assert_eq!(provider.moves.load(Ordering::SeqCst), 1);
    let inbox = page(&p, "Inbox").await;
    assert_eq!(
        inbox["total"], 0,
        "the acknowledged message reappeared in Inbox: {inbox}"
    );
    assert_eq!(page(&p, "Archive").await["total"], 2);
    Ok(())
}

#[tokio::test]
async fn unrepairable_receipt_pauses_only_its_group_and_can_be_accepted() -> Result<()> {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let provider = Scripted::new(vec![]);
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    review(&p, "stuck-a", json!({"kind":"archive"})).await;
    approve(&p, "stuck-a").await;
    fail_folder_cache(&p).await?;
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "repair");
    drop_folder_failure(&p).await?;
    // A refresh removes the moved source before its cache repair can run.
    p.database
        .write(|db| {
            let tx = db.transaction()?;
            crate::operations::reconcile_folder(&tx, "fixture", "INBOX", &HashSet::new())?;
            let raw = b"From: Alex <alex@example.test>\r\nTo: robin@example.test\r\nSubject: Later\r\nContent-Type: text/plain\r\n\r\nLater body".to_vec();
            let mail =
                shep_mail_core::model::parse_mail("fixture", "9", "INBOX", raw, true, false)?;
            crate::operations::insert_mail(&tx, mail, false)?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    groups(&p, json!({"kind":"retry","id":"stuck-a","position":0})).await;
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "repair");
    assert_eq!(job(&p, "stuck-a").await["state"], "paused");
    review(&p, "later-b", json!({"kind":"read"})).await;
    approve(&p, "later-b").await;
    run_all(&p, Some("fixture-only")).await;
    assert_eq!(
        provider.flags.load(Ordering::SeqCst),
        1,
        "an unrelated later group never reached the provider: {}",
        job(&p, "later-b").await
    );
    let accepted = groups(&p, json!({"kind":"accept","id":"stuck-a","position":0})).await;
    assert_eq!(accepted["counts"]["accepted"], 1);
    let row = items(&p, "stuck-a", None).await["rows"][0].clone();
    assert_eq!(row["state"], "accepted");
    assert_eq!(row["receipt"]["applied"]["folder"], "Archive");
    assert_eq!(
        scalar::<i64>(&p, "SELECT COUNT(*) FROM individual_mail_action_receipts r JOIN individual_mail_actions a ON a.id=r.action WHERE a.group_job='stuck-a'").await?,
        1,
        "accepting keeps the saved acknowledgement"
    );
    groups(&p, json!({"kind":"resume","id":"stuck-a"})).await;
    assert_eq!(job(&p, "stuck-a").await["state"], "finished");
    assert_eq!(step(&p, Some("fixture-only")).await["idle"], true);
    assert_eq!(provider.moves.load(Ordering::SeqCst), 1);
    groups(&p, json!({"kind":"remove","id":"stuck-a"})).await;
    assert!(job(&p, "stuck-a").await.is_null());
    Ok(())
}

#[tokio::test]
async fn same_value_skip_needs_a_settled_cache_behind_individual_repair() -> Result<()> {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let provider = Scripted::new(vec![]);
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    p.database
        .write(|db| {
            db.execute_batch("CREATE TRIGGER review_fail_unread BEFORE UPDATE OF unread ON mail BEGIN SELECT RAISE(ABORT,'fixture cache failure'); END;")?;
            Ok(())
        })
        .await?;
    let older = request(
        &p,
        json!({"op":"mutate","action_id":"older-read","id":"fixture:INBOX:0","unread":false,"password":"fixture-only"}),
    )
    .await;
    assert_eq!(older["status"], "repair");
    p.database
        .write(|db| {
            db.execute_batch("DROP TRIGGER review_fail_unread")?;
            Ok(())
        })
        .await?;
    assert_eq!(page(&p, "Inbox").await["mail"][0]["unread"], false);
    review(&p, "newer-unread", json!({"kind":"unread"})).await;
    approve(&p, "newer-unread").await;
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "done");
    request(&p, json!({"op":"inspect_mail_action","id":"older-read"})).await;
    let sent = provider.flag_values.lock().expect("flags").clone();
    let cached = p
        .database
        .read(|db| Ok(crate::operations::stored_mail(db, "fixture:INBOX:0")?.unread))
        .await?;
    assert_eq!(
        sent.last().and_then(|f| f.unread),
        Some(true),
        "the newer group Unread never reached the provider, which still has Read; cache unread={cached}"
    );
    assert!(
        cached,
        "the older acknowledgement must not replace the newer choice"
    );
    Ok(())
}

#[tokio::test]
async fn credential_request_is_not_recorded_as_group_success() -> Result<()> {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let provider = Scripted::new(vec![]);
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    review(&p, "no-credential", json!({"kind":"read"})).await;
    approve(&p, "no-credential").await;
    admit_only(&p, "no-credential", "no-credential-attempt").await?;
    let dispatched =
        crate::operations::group::dispatch(&p, "no-credential-attempt".into(), None).await?;
    assert_eq!(dispatched["status"], "waiting");
    let next = step(&p, None).await;
    assert_eq!(provider.flags.load(Ordering::SeqCst), 0);
    assert_eq!(next["outcome"], "failed", "no provider call was made");
    let row = items(&p, "no-credential", None).await["rows"][0].clone();
    assert!(row["receipt"].is_null());
    assert!(
        row["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("needs its password"))
    );
    assert_eq!(
        scalar::<String>(
            &p,
            "SELECT status FROM individual_mail_actions WHERE id='no-credential-attempt'"
        )
        .await?,
        "cancelled",
        "the unsent attempt must not stay resumable"
    );
    let removal = request(&p, json!({"op":"account_removal_preview","id":"fixture"})).await;
    assert_eq!(
        removal["actions"], 0,
        "group attempts are counted as group work"
    );
    assert_eq!(removal["groups"], 1);
    Ok(())
}

#[tokio::test]
async fn restart_requeues_an_admitted_attempt_that_was_never_sent() -> Result<()> {
    let (directory, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    review(&p, "unsent", json!({"kind":"read"})).await;
    approve(&p, "unsent").await;
    admit_only(&p, "unsent", "unsent-attempt").await?;
    drop(p);
    let p = crate::api::MobileProfile::open(
        directory
            .path()
            .join("mail.sqlite3")
            .to_string_lossy()
            .into(),
    )
    .await?;
    let provider = Scripted::new(vec![]);
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    assert_eq!(
        items(&p, "unsent", None).await["rows"][0]["state"],
        "pending"
    );
    assert_eq!(job(&p, "unsent").await["state"], "running");
    assert_eq!(step(&p, Some("fixture-only")).await["outcome"], "done");
    assert_eq!(provider.flags.load(Ordering::SeqCst), 1);
    Ok(())
}

#[tokio::test]
async fn unsaved_individual_move_defers_a_group_step_without_provider_work() -> Result<()> {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    make_imap(&p).await;
    let provider = Scripted::new(vec![Ok(Some("archived-uid".into()))]);
    *p.operations.provider.lock().expect("provider") = Some(provider.clone());
    review(&p, "behind-move", json!({"kind":"flag"})).await;
    approve(&p, "behind-move").await;
    fail_folder_cache(&p).await?;
    let moved = request(
        &p,
        json!({"op":"mutate","action_id":"individual-archive","id":"fixture:INBOX:0","folder":"Archive","password":"fixture-only"}),
    )
    .await;
    assert_eq!(moved["status"], "repair");
    drop_folder_failure(&p).await?;
    let deferred = step(&p, Some("fixture-only")).await;
    assert_eq!(deferred["outcome"], "failed");
    assert!(
        deferred["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("earlier move"))
    );
    assert_eq!(provider.flags.load(Ordering::SeqCst), 0);
    assert_eq!(provider.moves.load(Ordering::SeqCst), 1);
    Ok(())
}
