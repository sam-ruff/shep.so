use crate::{
    accounts,
    tests::{account, draft, profile, request, seed},
};
use serde_json::{Value, json};

async fn failure(p: &crate::api::MobileProfile, value: Value) -> String {
    let result: Value = serde_json::from_str(&p.request(value.to_string()).await.unwrap()).unwrap();
    result["error"].as_str().unwrap().into()
}
#[tokio::test]
async fn reviewed_removal_is_atomic_preserves_other_accounts_and_cannot_resurrect() {
    let (dir, p) = profile().await;
    seed(&p, 3).await;
    let mut other = account();
    other.id = "other".into();
    request(&p, json!({"op":"save_account","account":other})).await;
    request(
        &p,
        json!({"op":"save_draft","draft":draft(1,"Keep until reviewed")}),
    )
    .await;
    let review = request(&p, json!({"op":"account_removal_preview","id":"fixture"})).await;
    assert_eq!(review["messages"], 3);
    assert_eq!(review["drafts"], 1);
    let a: accounts::Removal = serde_json::from_value(review.clone()).unwrap();
    p.database.write(|db|{db.execute_batch("CREATE TRIGGER fail_removal BEFORE DELETE ON accounts BEGIN SELECT RAISE(ABORT,'Fixture storage failure'); END;")?;Ok(())}).await.unwrap();
    assert!(
        failure(&p, json!({"op":"remove_account","review":review}))
            .await
            .contains("Fixture storage failure")
    );
    assert_eq!(
        request(&p, json!({"op":"account_removal_preview","id":"fixture"})).await["fingerprint"],
        review["fingerprint"]
    );
    p.database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_removal")?;
            Ok(())
        })
        .await
        .unwrap();
    request(&p, json!({"op":"remove_account","review":review})).await;
    request(&p, json!({"op":"remove_account","review":review})).await;
    assert_eq!(
        request(&p, json!({"op":"accounts"})).await["accounts"][0]["id"],
        "other"
    );
    assert_eq!(
        request(&p, json!({"op":"credential_cleanup"})).await,
        json!(["fixture"])
    );
    assert!(
        failure(&p, json!({"op":"save_account","account":account()}))
            .await
            .contains("removed")
    );
    assert!(
        failure(
            &p,
            json!({"op":"save_draft","draft":draft(500,"late save")})
        )
        .await
        .contains("discarded")
    );
    let mut fresh = draft(1, "late new draft");
    fresh["id"] = json!("late");
    assert!(
        failure(&p, json!({"op":"save_draft","draft":fresh}))
            .await
            .contains("removed")
    );
    assert!(
        failure(&p, json!({"op":"credential_cleanup_done","id":"other"}))
            .await
            .contains("refused")
    );
    p.database
        .read(|db| {
            for table in ["mail", "drafts", "draft_files", "mail_aliases"] {
                let n: i64 =
                    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
                assert_eq!(n, 0);
            }
            let n: i64 = db.query_row(
                "SELECT COUNT(*) FROM mail_search WHERE mail_search MATCH 'needle'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(n, 0);
            Ok(())
        })
        .await
        .unwrap();
    drop(p);
    let p =
        crate::api::MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
            .await
            .unwrap();
    assert_eq!(
        request(&p, json!({"op":"credential_cleanup"})).await,
        json!(["fixture"])
    );
    request(&p, json!({"op":"credential_cleanup_done","id":"fixture"})).await;
    assert_eq!(
        request(&p, json!({"op":"credential_cleanup"})).await,
        json!([])
    );
    assert!(
        failure(&p, json!({"op":"check_account","id":a.id}))
            .await
            .contains("removed")
    );
}
#[tokio::test]
async fn stale_review_and_occupied_provider_leave_mail_untouched() {
    let (_dir, p) = profile().await;
    seed(&p, 2).await;
    let review = request(&p, json!({"op":"account_removal_preview","id":"fixture"})).await;
    request(
        &p,
        json!({"op":"save_draft","draft":draft(1,"new after review")}),
    )
    .await;
    assert!(
        failure(&p, json!({"op":"remove_account","review":review}))
            .await
            .contains("Local data changed")
    );
    let review = request(&p, json!({"op":"account_removal_preview","id":"fixture"})).await;
    let held = p.operations.account("fixture").await;
    assert!(
        failure(&p, json!({"op":"remove_account","review":review}))
            .await
            .contains("operation in progress")
    );
    drop(held);
    let _network = p.operations.hold_network_capacity().await;
    request(&p, json!({"op":"remove_account","review":review})).await;
}
#[tokio::test]
async fn uncertain_move_requires_explicit_review_and_draft_file_edits_invalidate_review() {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    request(&p, json!({"op":"save_draft","draft":draft(1,"files")})).await;
    let review = request(&p, json!({"op":"account_removal_preview","id":"fixture"})).await;
    p.database.write(|db|{db.execute("INSERT INTO draft_file_revisions VALUES('draft-one',1)",[])?;db.execute("INSERT INTO draft_files VALUES('file','draft-one','local.txt','text/plain',X'0001')",[])?;Ok(())}).await.unwrap();
    assert!(
        failure(&p, json!({"op":"remove_account","review":review}))
            .await
            .contains("Local data changed")
    );
    p.database
        .write(|db| {
            db.execute(
                "INSERT INTO pending_moves SELECT id,'Archive' FROM mail",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let review = request(&p, json!({"op":"account_removal_preview","id":"fixture"})).await;
    assert_eq!(review["files"], 1);
    assert_eq!(review["moves"], 1);
    assert!(
        failure(&p, json!({"op":"remove_account","review":review}))
            .await
            .contains("unfinished")
    );
    request(
        &p,
        json!({"op":"remove_account","review":review,"discard_unresolved":true}),
    )
    .await;
}
