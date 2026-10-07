use crate::{
    api::MobileProfile,
    operations,
    tests::{account, profile, request},
};
use base64::Engine;
use serde_json::{Value, json};
use shep_mail_core::model::*;

/// CRLF and bare LF line ends, 8-bit bytes, trailing spaces and no final line
/// break must all survive unchanged.
fn exact_raw() -> Vec<u8> {
    let mut raw = b"From: Alex <alex@example.test>\r\nTo: robin@example.test\r\n\
        Subject: Exact bytes\r\nContent-Type: text/plain; charset=iso-8859-1\r\n\
        Content-Transfer-Encoding: 8bit\r\n\r\nCaf"
        .to_vec();
    raw.extend_from_slice(&[0xE9, b' ', b' ', b'\n', b'L', b'F', 0xFF, b'\r', b'\n']);
    raw.extend_from_slice(b"no final break  ");
    raw
}

async fn store(profile: &MobileProfile, raw: Vec<u8>) -> String {
    request(profile, json!({"op":"save_account","account":account()})).await;
    profile
        .database
        .write(move |db| {
            let mail = parse_mail("fixture", "7", "INBOX", raw, true, false)?;
            let id = mail.summary.id.clone();
            operations::insert_mail(db, mail, false)?;
            Ok(id)
        })
        .await
        .unwrap()
}

async fn original(profile: &MobileProfile, id: &str) -> Result<Vec<u8>, String> {
    let reply: Value = serde_json::from_str(
        &profile
            .request(json!({"op":"original_message","id":id}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    if let Some(error) = reply["error"].as_str() {
        return Err(error.to_owned());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(reply["data"]["bytes"].as_str().unwrap())
        .unwrap();
    assert_eq!(reply["data"]["size"], bytes.len());
    Ok(bytes)
}

#[tokio::test]
async fn original_message_returns_the_exact_cached_bytes_through_aliases() {
    let (_dir, p) = profile().await;
    let id = store(&p, exact_raw()).await;
    assert_eq!(original(&p, &id).await, Ok(exact_raw()));
    let target = id.clone();
    p.database
        .write(move |db| {
            db.execute(
                "INSERT INTO mail_aliases(alias,id) VALUES('earlier-identity',?1)",
                [&target],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(original(&p, "earlier-identity").await, Ok(exact_raw()));
    let unchanged: Vec<u8> = p
        .database
        .read(move |db| Ok(db.query_row("SELECT raw FROM mail WHERE id=?1", [&id], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(unchanged, exact_raw(), "saving reads without rewriting");
}

#[tokio::test]
async fn missing_or_empty_originals_are_refused() {
    let (_dir, p) = profile().await;
    let missing = original(&p, "not-cached").await.unwrap_err();
    assert!(missing.contains("no longer cached"), "{missing}");
    let id = store(&p, exact_raw()).await;
    let target = id.clone();
    p.database
        .write(move |db| {
            db.execute("UPDATE mail SET raw=X'' WHERE id=?1", [&target])?;
            Ok(())
        })
        .await
        .unwrap();
    let empty = original(&p, &id).await.unwrap_err();
    assert!(empty.contains("no cached original"), "{empty}");
}
