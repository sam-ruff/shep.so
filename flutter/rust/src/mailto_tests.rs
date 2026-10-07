use crate::{
    api::MobileProfile,
    tests::{account, profile, request},
};
use rusqlite::params;
use serde_json::{Value, json};
use shep_mail_core::model::*;

const DRAFT: &str = "6b1f6f0e-1c1d-4c3b-9a55-2f0d6f4a9e01";

async fn error(profile: &MobileProfile, payload: Value) -> String {
    let reply: Value =
        serde_json::from_str(&profile.request(payload.to_string()).await.unwrap()).unwrap();
    reply["error"]
        .as_str()
        .unwrap_or_else(|| panic!("expected an error: {reply}"))
        .to_owned()
}

async fn drafts(profile: &MobileProfile) -> Vec<Value> {
    request(profile, json!({"op":"drafts"}))
        .await
        .as_array()
        .cloned()
        .unwrap_or_default()
}

async fn outgoing(profile: &MobileProfile) -> i64 {
    profile
        .database
        .read(|db| Ok(db.query_row("SELECT COUNT(*) FROM outgoing", [], |r| r.get(0))?))
        .await
        .unwrap()
}

#[tokio::test]
async fn activation_link_saves_every_field_beside_existing_drafts_through_restart() {
    let (dir, p) = profile().await;
    request(&p, json!({"op":"save_account","account":account()})).await;
    let existing = Draft {
        id: "existing".into(),
        account_id: account().id,
        to: "kept@example.test".into(),
        body: "Unsent edit".into(),
        revision: 3,
        ..Default::default()
    };
    request(&p, json!({"op":"save_draft","draft":existing})).await;
    let link = "mailto:friend@example.test?cc=copy@example.test&bcc=hidden@example.test\
                &subject=Hello%20there&body=First%0D%0ASecond&attachment=/private/file";
    let created = request(
        &p,
        json!({"op":"mailto_draft","id":DRAFT,"account":account().id,"link":link,"message":false}),
    )
    .await;
    assert_eq!(created["id"], DRAFT);
    assert_eq!(created["account_id"], account().id);
    assert_eq!(created["to"], "friend@example.test");
    assert_eq!(created["cc"], "copy@example.test");
    assert_eq!(created["bcc"], "hidden@example.test");
    assert_eq!(created["subject"], "Hello there");
    assert_eq!(created["body"], "First\nSecond");
    assert_eq!(created["revision"], 0);
    assert_eq!(created["attachments"], json!([]));
    drop(p);
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let saved = drafts(&reopened).await;
    assert_eq!(saved.len(), 2);
    let kept = saved.iter().find(|d| d["id"] == "existing").unwrap();
    assert_eq!(kept["body"], "Unsent edit");
    assert_eq!(kept["revision"], 3);
    let opened = saved.iter().find(|d| d["id"] == DRAFT).unwrap();
    assert_eq!(opened["subject"], "Hello there");
    assert_eq!(outgoing(&reopened).await, 0, "a link never sends mail");
}

#[tokio::test]
async fn message_link_keeps_only_its_address() {
    let (_dir, p) = profile().await;
    request(&p, json!({"op":"save_account","account":account()})).await;
    let created = request(
        &p,
        json!({"op":"mailto_draft","id":DRAFT,"account":account().id,
            "link":"mailto:friend%40example.test?subject=untrusted&bcc=hidden@example.test&body=Hi&attachment=/private/file",
            "message":true}),
    )
    .await;
    assert_eq!(created["to"], "friend@example.test");
    for field in ["cc", "bcc", "subject", "body"] {
        assert_eq!(created[field], "", "{field}");
    }
}

#[tokio::test]
async fn exact_retry_returns_the_same_draft_and_other_uses_are_refused() {
    let (_dir, p) = profile().await;
    request(&p, json!({"op":"save_account","account":account()})).await;
    let open = |link: &str| json!({"op":"mailto_draft","id":DRAFT,"account":account().id,"link":link,"message":false});
    let first = request(&p, open("mailto:a@example.test?subject=Once")).await;
    let retry = request(&p, open("mailto:a@example.test?subject=Once")).await;
    assert_eq!(first, retry);
    assert_eq!(drafts(&p).await.len(), 1);
    assert!(
        error(&p, open("mailto:b@example.test"))
            .await
            .contains("already used")
    );
    let mut edited: Draft = serde_json::from_value(first).unwrap();
    edited.body = "Typed after opening".into();
    edited.revision = 1;
    request(&p, json!({"op":"save_draft","draft":edited})).await;
    assert!(
        error(&p, open("mailto:a@example.test?subject=Once"))
            .await
            .contains("already used"),
        "an edited draft is never replaced"
    );
    assert_eq!(drafts(&p).await[0]["body"], "Typed after opening");
    request(&p, json!({"op":"discard_draft","id":DRAFT,"revision":1})).await;
    assert!(
        error(&p, open("mailto:a@example.test?subject=Once"))
            .await
            .contains("already used"),
        "a discarded draft is not revived"
    );
    assert!(drafts(&p).await.is_empty());
    assert!(
        error(
            &p,
            json!({"op":"mailto_draft","id":"not-a-uuid","account":account().id,
                "link":"mailto:a@example.test","message":false}),
        )
        .await
        .contains("new draft identity")
    );
}

#[tokio::test]
async fn rejected_links_create_no_draft() {
    let (_dir, p) = profile().await;
    request(&p, json!({"op":"save_account","account":account()})).await;
    let long = format!(
        "mailto:{}@example.test",
        "a".repeat(shep_mail_content::mailto::MAX_LEN)
    );
    for (link, message, expected) in [
        (
            "mailto:a@example.test?subject=caf%E9",
            false,
            "invalid encoded text",
        ),
        ("mailto:a%zz@example.test", true, "invalid encoded text"),
        ("https://example.test", false, "not an email link"),
        ("javascript:alert(1)", true, "not an email link"),
        (long.as_str(), false, "too long"),
    ] {
        let text = error(
            &p,
            json!({"op":"mailto_draft","id":DRAFT,"account":account().id,"link":link,"message":message}),
        )
        .await;
        assert!(text.contains(expected), "{link}: {text}");
    }
    assert!(drafts(&p).await.is_empty());
}

#[tokio::test]
async fn draft_without_an_account_is_kept_and_unknown_or_removed_accounts_are_refused() {
    let (_dir, p) = profile().await;
    let created = request(
        &p,
        json!({"op":"mailto_draft","id":DRAFT,"account":"","link":"mailto:a@example.test","message":false}),
    )
    .await;
    assert_eq!(created["account_id"], "");
    assert_eq!(created["to"], "a@example.test");
    let other = "0d8c7c1e-8f26-4d71-9a7e-5f8f3c7b2a10";
    assert!(
        error(
            &p,
            json!({"op":"mailto_draft","id":other,"account":"missing","link":"mailto:a@example.test","message":false}),
        )
        .await
        .contains("no longer connected")
    );
    p.database
        .write(|db| {
            db.execute(
                "INSERT INTO removed_accounts(id,fingerprint) VALUES(?1,'fixture')",
                params!["gone"],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        error(
            &p,
            json!({"op":"mailto_draft","id":other,"account":"gone","link":"mailto:a@example.test","message":false}),
        )
        .await
        .contains("removed")
    );
    assert_eq!(drafts(&p).await.len(), 1);
}

#[tokio::test]
async fn shared_cases_agree_with_the_native_draft() {
    let (_dir, p) = profile().await;
    let cases: Value =
        serde_json::from_str(include_str!("../../../shared/mailto-cases.json")).unwrap();
    for case in cases["cases"].as_array().unwrap() {
        let id = uuid::Uuid::new_v4().to_string();
        let payload = json!({"op":"mailto_draft","id":id,"account":"","link":case["link"],
            "message":case["mode"] == "message"});
        if case.get("rejected").is_some() {
            error(&p, payload).await;
            continue;
        }
        let created = request(&p, payload).await;
        for field in ["to", "cc", "bcc", "subject", "body"] {
            assert_eq!(
                created[field],
                case["expected"].get(field).cloned().unwrap_or(json!("")),
                "{}: {field}",
                case["name"]
            );
        }
    }
}
