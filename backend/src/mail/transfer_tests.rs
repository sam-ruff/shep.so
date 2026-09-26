use super::*;
use crate::tests::{body, login};
use axum::http::Request as HttpRequest;
use tower::ServiceExt;

const RAW: &[u8] = b"Message-ID: <plans@example.test>\r\nSubject: Plans\r\n\r\nBody";

fn setup(transfers: MockHostedTransfer) -> AppState {
    let (mut state, _) = crate::tests::state("owner@example.test");
    let endpoints = vec![policy::Endpoint {
        host: "mail.example.test".into(),
        port: 993,
        service: policy::Service::Imap,
        address: "127.0.0.1:1993".parse().expect("fixture address"),
    }];
    if let Some(config) = Arc::get_mut(&mut state.config) {
        config.mail_endpoints = endpoints.clone();
    }
    let mut mail = MailHub::new(endpoints);
    mail.transfers = Arc::new(transfers);
    state.mail = Arc::new(mail);
    state
}
fn connection(id: &str) -> serde_json::Value {
    serde_json::json!({"account":{"id":id,"name":id,"email":format!("{id}@example.test"),"protocol":"Imap","host":"mail.example.test","port":993,"username":id,"smtp_host":"mail.example.test","smtp_port":465},"password":"fixture-password"})
}
fn mail() -> Mail {
    shep_mail_core::model::parse_mail("work", "42.7", "INBOX", RAW.to_vec(), true, false)
        .expect("fixture mail")
        .summary
}
fn request() -> serde_json::Value {
    use base64::Engine;
    serde_json::json!({
        "source": connection("work"),
        "destination": connection("personal"),
        "mail": mail(),
        "folder": "Plans",
        "raw": base64::engine::general_purpose::STANDARD.encode(RAW),
    })
}
async fn post(state: &AppState, path: &str, value: serde_json::Value) -> (StatusCode, String) {
    let (cookie, csrf) = login(state).await;
    let response = crate::app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri(path)
                .header(header::COOKIE, cookie)
                .header(header::ORIGIN, &state.config.origin)
                .header("x-shep-csrf", csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string()))
                .expect("fixture request"),
        )
        .await
        .expect("fixture response");
    let status = response.status();
    (status, body(response).await)
}

#[tokio::test]
async fn upload_returns_the_destination_identity_for_the_exact_bytes() {
    let mut transfers = MockHostedTransfer::new();
    transfers
        .expect_transfer()
        .withf(|source, destination, mail, folder, raw| {
            source.account.id == "work"
                && destination.account.id == "personal"
                && mail.remote_id == "42.7"
                && folder == "Plans"
                && raw == RAW
        })
        .times(1)
        .returning(|_, _, _, _, _| Ok(Some("9.3".into())));
    transfers.expect_finish_transfer().never();
    let (status, text) = post(&setup(transfers), "/api/mail/transfer", request()).await;
    assert_eq!(status, StatusCode::OK);
    let data: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
    assert_eq!(data["committed"], true);
    assert_eq!(data["remote_id"], "9.3");
}

#[tokio::test]
async fn refused_and_uncertain_uploads_are_reported_distinctly_without_private_text() {
    for (failure, expected) in [
        (true, StatusCode::CONFLICT),
        (false, StatusCode::BAD_GATEWAY),
    ] {
        let mut transfers = MockHostedTransfer::new();
        transfers
            .expect_transfer()
            .times(1)
            .returning(move |_, _, _, _, _| {
                let error = anyhow::anyhow!("private server text");
                Err(if failure {
                    TransferFailure::NotApplied(error)
                } else {
                    TransferFailure::Uncertain(error)
                })
            });
        let (status, text) = post(&setup(transfers), "/api/mail/transfer", request()).await;
        assert_eq!(status, expected);
        assert!(!text.contains("private server text"));
        assert!(!text.contains("committed"));
        assert_eq!(text.contains("\"refused\":true"), failure);
    }
}

#[tokio::test]
async fn invalid_transfers_never_reach_either_server() {
    let mut cases = Vec::new();
    let mut same = request();
    same["destination"] = connection("work");
    cases.push(same);
    let mut pop = request();
    pop["destination"]["account"]["protocol"] = "Pop3".into();
    cases.push(pop);
    let mut foreign = request();
    foreign["mail"]["account_id"] = "personal".into();
    cases.push(foreign);
    let mut folder = request();
    folder["folder"] = "Plans\r\nA1 DELETE INBOX".into();
    cases.push(folder);
    let mut empty = request();
    empty["raw"] = "".into();
    cases.push(empty);
    let mut garbled = request();
    garbled["raw"] = "not base64!".into();
    cases.push(garbled);
    let mut host = request();
    host["destination"]["account"]["host"] = "elsewhere.example.test".into();
    cases.push(host);
    for case in cases {
        let mut transfers = MockHostedTransfer::new();
        transfers.expect_transfer().never();
        let (status, _) = post(&setup(transfers), "/api/mail/transfer", case).await;
        assert!(
            status == StatusCode::BAD_REQUEST || status == StatusCode::FORBIDDEN,
            "{status}"
        );
    }
}

#[tokio::test]
async fn source_cleanup_names_the_exact_original_and_reports_failures() {
    for succeeds in [true, false] {
        let mut transfers = MockHostedTransfer::new();
        transfers.expect_transfer().never();
        transfers
            .expect_finish_transfer()
            .withf(|source, original| {
                source.account.id == "work"
                    && original.id == mail().id
                    && original.remote_id == "42.7"
                    && original.folder == "INBOX"
            })
            .times(1)
            .returning(move |_, _| {
                if succeeds {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("private cleanup text"))
                }
            });
        let (status, text) = post(
            &setup(transfers),
            "/api/mail/transfer/finish",
            serde_json::json!({"source": connection("work"), "mail": mail()}),
        )
        .await;
        assert_eq!(
            status,
            if succeeds {
                StatusCode::OK
            } else {
                StatusCode::BAD_GATEWAY
            }
        );
        assert!(!text.contains("private cleanup text"));
    }
    let mut transfers = MockHostedTransfer::new();
    transfers.expect_finish_transfer().never();
    let (status, _) = post(
        &setup(transfers),
        "/api/mail/transfer/finish",
        serde_json::json!({"source": connection("personal"), "mail": mail()}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
