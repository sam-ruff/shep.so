use crate::{
    api::MobileProfile,
    drafts, operations,
    tests::{profile, request, seed},
};
use rusqlite::params;
use serde_json::{Value, json};
use shep_mail_core::{
    compose,
    model::{Draft, parse_mail},
};

fn raw() -> Vec<u8> {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../../shared/forward-fixtures.json")).unwrap();
    cases[0]["raw"].as_str().unwrap().as_bytes().to_vec()
}
async fn original(p: &MobileProfile) -> String {
    seed(p, 0).await;
    p.database
        .write(|db| {
            let mail = parse_mail("fixture", "42.7", "INBOX", raw(), true, false)?;
            let id = mail.summary.id.clone();
            operations::insert_mail(db, mail, false)?;
            Ok(id)
        })
        .await
        .unwrap()
}
fn forward(source: &str, id: &str) -> Value {
    json!({"op":"forward","id":source,"draft_id":id})
}

#[tokio::test]
async fn cached_forward_is_independent_of_providers_and_retains_metadata_through_save_and_reopen() {
    let (dir, p) = profile().await;
    let source = original(&p).await;
    let id = uuid::Uuid::new_v4().to_string();
    let network = p.operations.hold_network_capacity().await;
    let render = p.operations.hold_render_capacity().await;
    let mut draft = request(&p, forward(&source, &id)).await;
    assert_eq!(draft["subject"], "Fwd: Café project");
    for field in ["to", "cc", "bcc"] {
        assert_eq!(draft[field], "");
    }
    assert_eq!(draft["in_reply_to"], Value::Null);
    assert_eq!(draft["references"], json!([]));
    let files = draft["attachments"].clone();
    assert_eq!(files.as_array().unwrap().len(), 3);
    assert!(
        files[2]["content_id"]
            .as_str()
            .unwrap()
            .starts_with("shep-")
    );
    let quote = draft["forward"].clone();
    draft["revision"] = json!(3);
    draft["to"] = json!("reviewer@example.test");
    draft["body"] = json!(format!(
        "Review <this>.\n{}",
        draft["body"].as_str().unwrap()
    ));
    let body = draft["body"].clone();
    // A stale/client-only text snapshot cannot erase the original quote/files.
    draft["forward"] = Value::Null;
    draft["attachments"] = json!([]);
    request(&p, json!({"op":"save_draft","draft":draft})).await;
    let saved = request(&p, json!({"op":"drafts"})).await;
    assert_eq!(saved[0]["forward"], quote);
    assert_eq!(saved[0]["attachments"], files);
    assert_eq!(saved[0]["body"], body);
    drop(network);
    drop(render);
    drop(p);
    let p = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let retried = request(&p, forward(&source, &id)).await;
    assert_eq!(retried, saved[0]);
    let target = id.clone();
    p.database
        .read(move |db| {
            let mut draft: Draft = serde_json::from_value(drafts::value(db, &target)?)?;
            let files = drafts::files(db, &mut draft)?;
            assert_eq!(files[0].bytes, [0, 255, 1, 13, 10]);
            assert_eq!(files[0].attachment.media_type, "application/x-first");
            assert_eq!(files[1].attachment.media_type, "application/x-second");
            assert_eq!(files[2].bytes, b"inline fixture");
            let wire = compose::build(&crate::tests::account(), &draft, files)?.formatted();
            let body = shep_mail_core::reader::decode(&wire)?;
            assert!(!body.html.is_empty());
            assert_eq!(body.resources.values().next().unwrap(), b"inline fixture");
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn forward_insert_rolls_back_all_files_and_retry_preserves_current_draft_after_source_disappears()
 {
    let (_dir, p) = profile().await;
    let source = original(&p).await;
    let id = uuid::Uuid::new_v4().to_string();
    p.database.write(|db| {db.execute_batch("CREATE TRIGGER fail_forward BEFORE INSERT ON draft_inline BEGIN SELECT RAISE(ABORT,'Synthetic disk full'); END;")?; Ok(())}).await.unwrap();
    let failed: Value =
        serde_json::from_str(&p.request(forward(&source, &id).to_string()).await.unwrap()).unwrap();
    assert!(failed.get("error").is_some());
    p.database
        .write(|db| {
            for table in ["drafts", "draft_files", "draft_inline", "draft_forwards"] {
                assert_eq!(
                    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))?,
                    0
                );
            }
            db.execute_batch("DROP TRIGGER fail_forward")?;
            Ok(())
        })
        .await
        .unwrap();
    let draft = request(&p, forward(&source, &id)).await;
    request(
        &p,
        json!({"op":"remove_draft_file","id":id,"file":draft["attachments"][0]["id"]}),
    )
    .await;
    p.database
        .write(|db| {
            db.execute("DELETE FROM mail", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let retried = request(&p, forward(&source, &id)).await;
    assert_eq!(retried["attachments"].as_array().unwrap().len(), 2);
    assert_eq!(retried["file_revision"], 2);
    request(&p, json!({"op":"discard_draft","id":id,"revision":1})).await;
    let reply: Value =
        serde_json::from_str(&p.request(forward(&source, &id).to_string()).await.unwrap()).unwrap();
    assert!(reply.get("error").is_some());
    assert_eq!(request(&p, json!({"op":"drafts"})).await, json!([]));
}

#[tokio::test]
async fn account_removal_after_preparation_refuses_commit_and_corrupt_resources_leave_no_draft() {
    let (_dir, p) = profile().await;
    let source = original(&p).await;
    let (draft, files) =
        compose::prepare_forward(uuid::Uuid::new_v4().to_string(), "fixture".into(), &raw())
            .unwrap();
    p.database
        .write(move |db| {
            db.execute("DELETE FROM mail", [])?;
            db.execute("DELETE FROM accounts", [])?;
            assert!(drafts::create_forward(db, &source, draft, files).is_err());
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM drafts", [], |r| r.get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
    let source = original(&p).await;
    p.database
        .write(|db| {
            let broken = String::from_utf8(raw())
                .unwrap()
                .replace("aW5saW5lIGZpeHR1cmU=", "PRIVATE-invalid***")
                .into_bytes();
            db.execute("UPDATE mail SET raw=?1", [broken])?;
            Ok(())
        })
        .await
        .unwrap();
    let error: Value = serde_json::from_str(
        &p.request(forward(&source, &uuid::Uuid::new_v4().to_string()).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("Could not decode")
    );
    assert!(!error["error"].as_str().unwrap().contains("PRIVATE"));
    assert_eq!(request(&p, json!({"op":"drafts"})).await, json!([]));
}

#[tokio::test]
async fn rejected_forward_recovery_clones_inline_identities_and_immutable_quote() {
    let (_dir, p) = profile().await;
    let source = original(&p).await;
    let id = uuid::Uuid::new_v4().to_string();
    let draft = request(&p, forward(&source, &id)).await;
    let target = id.clone();
    p.database.write(move|db| {
        let mut draft:Draft=serde_json::from_value(drafts::value(db,&target)?)?;
        draft.to="recipient@example.test".into();
        drafts::save_text(db,draft.clone())?;
        let parts=drafts::files(db,&mut draft)?;
        let raw=compose::build(&crate::tests::account(),&draft,parts)?.formatted();
        db.execute("INSERT INTO outgoing(id,draft_id,state,account_id,message_id,raw,draft) VALUES('rejected',?1,'rejected','fixture','<forward@example.test>',?2,?3)",params![target,raw,serde_json::to_string(&draft)?])?;
        Ok(())
    }).await.unwrap();
    let recovered = request(
        &p,
        json!({"op":"recover_outgoing","id":"rejected","action":"return","confirmed":false}),
    )
    .await;
    let list = request(&p, json!({"op":"drafts"})).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], recovered["draft_id"]);
    assert_eq!(list[0]["forward"], draft["forward"]);
    for (old, new) in draft["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .zip(list[0]["attachments"].as_array().unwrap())
    {
        assert_ne!(old["id"], new["id"]);
        assert_eq!(old["content_id"], new["content_id"]);
    }
    let id = list[0]["id"].as_str().unwrap().to_owned();
    p.database
        .read(move |db| {
            let mut draft: Draft = serde_json::from_value(drafts::value(db, &id)?)?;
            let files = drafts::files(db, &mut draft)?;
            assert_eq!(files[2].bytes, b"inline fixture");
            assert!(files[2].attachment.content_id.is_some());
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn version_eight_drafts_keep_plain_attachments_when_inline_metadata_is_added() {
    let (dir, p) = profile().await;
    seed(&p, 0).await;
    request(
        &p,
        json!({"op":"save_draft","draft":crate::tests::draft(1,"Legacy text")}),
    )
    .await;
    p.database.write(|db| {
        db.execute("INSERT INTO draft_files VALUES('legacy-file','draft-one','old.txt','text/plain',?1)",[b"Legacy bytes".as_slice()])?;
        db.execute_batch("DROP TABLE draft_inline; DROP TABLE draft_forwards; PRAGMA user_version=8;")?;Ok(())
    }).await.unwrap();
    drop(p);
    let p = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let drafts = request(&p, json!({"op":"drafts"})).await;
    assert_eq!(drafts[0]["body"], "Legacy text");
    assert_eq!(drafts[0]["attachments"][0]["content_id"], Value::Null);
    p.database
        .read(|db| {
            assert_eq!(
                db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
                11
            );
            assert_eq!(
                db.query_row(
                    "SELECT bytes FROM draft_files WHERE id='legacy-file'",
                    [],
                    |r| r.get::<_, Vec<u8>>(0)
                )?,
                b"Legacy bytes"
            );
            Ok(())
        })
        .await
        .unwrap();
}
