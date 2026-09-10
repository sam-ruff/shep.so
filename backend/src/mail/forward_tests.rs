use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use mailparse::MailHeaderMap;

#[tokio::test]
async fn forward_prepare_preserves_inline_identity_and_rejects_header_injection_before_send() {
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../../shared/forward-fixtures.json")).unwrap();
    let source = cases[0]["raw"].as_str().unwrap();
    let (mut draft, files) = shep_mail_core::compose::prepare_forward(
        "new-forward".into(),
        "fixture-account".into(),
        source.as_bytes(),
    )
    .unwrap();
    draft.to = "reviewer@example.test".into();
    draft.body = format!("Please review <this>.\n{}", draft.body);
    let attachments: Vec<_> = files
        .iter()
        .map(|f| {
            serde_json::json!({
                "id": f.attachment.id, "name": f.attachment.name,
                "media_type": f.attachment.media_type,
                "content_id": f.attachment.content_id, "data": STANDARD.encode(&f.bytes),
            })
        })
        .collect();
    let id = reserve(&state, &cookie, &csrf).await;
    let mut request = serde_json::json!({"id": id, "connection": connection(), "draft": draft, "files": attachments});
    let inline = files
        .iter()
        .position(|f| f.attachment.content_id.is_some())
        .unwrap();
    request["files"][inline]["content_id"] = "evil\r\nBcc: injected@example.test".into();
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/outgoing/prepare",
        request.clone(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(!body(response).await.contains("injected@example.test"));
    request["files"][inline]["content_id"] = attachments[inline]["content_id"].clone();
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/outgoing/prepare",
        request,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let prepared: serde_json::Value = serde_json::from_str(&body(response).await).unwrap();
    let raw = STANDARD
        .decode(prepared["wire"]["raw"].as_str().unwrap())
        .unwrap();
    let parsed = mailparse::parse_mail(&raw).unwrap();
    for header in ["Bcc", "In-Reply-To", "References"] {
        assert!(parsed.headers.get_first_value(header).is_none());
    }
    assert_eq!(
        parsed.headers.get_first_value("Message-ID").unwrap(),
        format!("<{id}@shep.so>")
    );
    let mut pending = vec![&parsed];
    let mut leaves = Vec::new();
    while let Some(part) = pending.pop() {
        if part.subparts.is_empty() {
            leaves.push(part);
        } else {
            pending.extend(&part.subparts);
        }
    }
    let html = leaves
        .iter()
        .find(|p| p.ctype.mimetype == "text/html")
        .unwrap()
        .get_body()
        .unwrap();
    assert!(html.contains("<table>"));
    assert!(html.contains("Please review &lt;this&gt;."));
    for file in &files {
        assert!(leaves.iter().any(|part| {
            part.ctype.mimetype == file.attachment.media_type
                && part.get_body_raw().unwrap() == file.bytes
                && part.headers.get_first_value("Content-ID")
                    == file
                        .attachment
                        .content_id
                        .as_ref()
                        .map(|id| format!("<{id}>"))
        }));
    }
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}
