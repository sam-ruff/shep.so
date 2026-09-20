use super::*;
use crate::FailureKind;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Request {
    headers: String,
    body: Vec<u8>,
}

fn reply(status: u16, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn fixture(
    replies: Vec<String>,
) -> Result<(
    GoogleCalendarProvider,
    tokio::task::JoinHandle<Result<Vec<Request>>>,
)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}/calendar/v3", listener.local_addr()?);
    let server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(10), async {
            let mut requests = Vec::new();
            for response in replies {
                let (mut stream, _) = listener.accept().await?;
                let mut bytes = Vec::new();
                let (end, length) = loop {
                    let mut chunk = [0; 1024];
                    let count = stream.read(&mut chunk).await?;
                    anyhow::ensure!(count > 0, "Fixture request ended early");
                    bytes.extend_from_slice(&chunk[..count]);
                    anyhow::ensure!(bytes.len() <= 65536, "Fixture request exceeded its bound");
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = std::str::from_utf8(&bytes[..end])?;
                        let length = headers
                            .lines()
                            .find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>())
                            })
                            .transpose()?
                            .unwrap_or(0);
                        anyhow::ensure!(length <= 65536, "Fixture body exceeded its bound");
                        break (end + 4, length);
                    }
                };
                while bytes.len() < end + length {
                    let mut chunk = [0; 1024];
                    let count = stream.read(&mut chunk).await?;
                    anyhow::ensure!(count > 0, "Fixture body ended early");
                    bytes.extend_from_slice(&chunk[..count]);
                }
                requests.push(Request {
                    headers: String::from_utf8(bytes[..end].to_vec())?,
                    body: bytes[end..end + length].to_vec(),
                });
                stream.write_all(response.as_bytes()).await?;
                stream.shutdown().await?;
            }
            Ok(requests)
        })
        .await?
    });
    Ok((
        GoogleCalendarProvider {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(3))
                .build()?,
            base,
        },
        server,
    ))
}

fn event() -> Result<Event> {
    Ok(Event {
        id: "remote/id".into(),
        source_id: "team@example.test".into(),
        title: "Review".into(),
        start: "2026-09-20T10:00:00Z".parse()?,
        end: "2026-09-20T11:00:00Z".parse()?,
        location: String::new(),
        description: String::new(),
        all_day: false,
        etag: Some("\"v1\"".into()),
        remote_url: Some("remote/id".into()),
    })
}

fn saved(id: &str, etag: &str) -> String {
    json!({"id": id, "etag": etag, "summary":"Review",
        "start":{"dateTime":"2026-09-20T10:00:00Z"},
        "end":{"dateTime":"2026-09-20T11:00:00Z"}})
    .to_string()
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn google_http_recovery_requires_exact_identity_and_editable_receipt() -> Result<()> {
    let mut missing: Value = serde_json::from_str(&saved("remote/id", "v2"))?;
    missing
        .as_object_mut()
        .expect("event object")
        .remove("etag");
    let (provider, server) = fixture(vec![
        reply(200, &saved("different-id", "v2")),
        reply(200, &missing.to_string()),
        reply(200, &missing.to_string()),
        reply(200, &saved("remote/id", "")),
        reply(200, &saved("remote/id", "v2")),
    ])
    .await?;
    let event = event()?;
    for _ in 0..2 {
        assert_eq!(
            provider
                .read("fixture-token", &event)
                .await
                .expect_err("unconfirmed recovery")
                .kind,
            FailureKind::Waiting
        );
    }
    for _ in 0..2 {
        assert_eq!(
            provider
                .save("fixture-token", "edit", &event)
                .await
                .expect_err("incomplete receipt")
                .kind,
            FailureKind::Uncertain
        );
    }
    assert_eq!(
        provider
            .read("fixture-token", &event)
            .await?
            .expect("confirmed event")
            .id,
        event.id
    );
    assert_eq!(server.await??.len(), 5);
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn google_http_unknown_calendar_roles_are_read_only() -> Result<()> {
    let (provider, server) = fixture(vec![reply(
        200,
        &json!({"items":[
            {"id":"owner","accessRole":"owner"},
            {"id":"writer","accessRole":"writer"},
            {"id":"reader","accessRole":"reader"},
            {"id":"unknown","accessRole":"future-role"},
            {"id":"missing"}
        ]})
        .to_string(),
    )])
    .await?;
    let sources = provider.sources("fixture-token").await?;
    assert_eq!(
        sources
            .iter()
            .map(|source| source.read_only)
            .collect::<Vec<_>>(),
        [false, false, true, true, true]
    );
    assert_eq!(server.await??.len(), 1);
    Ok(())
}

#[test]
fn google_http_failures_preserve_retry_and_uncertainty_policy() {
    for status in [
        reqwest::StatusCode::UNAUTHORIZED,
        reqwest::StatusCode::FORBIDDEN,
        reqwest::StatusCode::TOO_MANY_REQUESTS,
    ] {
        assert_eq!(
            response_failure(status, String::new()).kind,
            FailureKind::Waiting
        );
    }
    assert_eq!(
        response_failure(reqwest::StatusCode::PRECONDITION_FAILED, String::new()).kind,
        FailureKind::Rejected
    );
    for status in [
        reqwest::StatusCode::REQUEST_TIMEOUT,
        reqwest::StatusCode::BAD_GATEWAY,
    ] {
        assert_eq!(
            response_failure(status, String::new()).kind,
            FailureKind::Uncertain
        );
    }
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn acknowledged_oversized_or_different_event_is_never_authorised_for_retry() -> Result<()> {
    let mut oversized: Value = serde_json::from_str(&saved("remote/id", "etag"))?;
    oversized["summary"] = json!("x".repeat(1025));
    let (provider, server) = fixture(vec![
        reply(200, &oversized.to_string()),
        reply(200, &saved("another-event", "etag")),
    ])
    .await?;
    for _ in 0..2 {
        assert_eq!(
            provider
                .save("fixture-token", "edit", &event()?)
                .await
                .expect_err("unconfirmed success metadata")
                .kind,
            FailureKind::Uncertain
        );
    }
    assert_eq!(server.await??.len(), 2);
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn google_http_preserves_reserved_create_identity_and_exact_etags() -> Result<()> {
    let (provider, server) = fixture(vec![
        reply(201, &saved("shep0123", "\"v1\"")),
        reply(201, &saved("shep0123", "\"v1\"")),
        reply(200, &saved("remote/id", "\"v2\"")),
        reply(204, ""),
    ])
    .await?;
    let existing = event()?;
    let creating = Event {
        etag: None,
        remote_url: None,
        ..existing.clone()
    };
    for _ in 0..2 {
        assert_eq!(
            provider.save("fixture-token", "01-23", &creating).await?.id,
            "shep0123"
        );
    }
    let updated = provider.save("fixture-token", "edit", &existing).await?;
    provider.delete("fixture-token", &updated).await?;
    let requests = server.await??;
    for request in &requests {
        assert!(
            request
                .headers
                .to_ascii_lowercase()
                .contains("authorization: bearer fixture-token\r\n")
        );
    }
    for request in &requests[..2] {
        assert!(
            request
                .headers
                .starts_with("POST /calendar/v3/calendars/team%40example.test/events HTTP/1.1\r\n")
        );
        let body: Value = serde_json::from_slice(&request.body)?;
        assert_eq!(body["id"], "shep0123");
        assert_eq!(
            body["extendedProperties"]["private"]["shepCreateId"],
            "01-23"
        );
    }
    assert!(requests[2].headers.starts_with(
        "PATCH /calendar/v3/calendars/team%40example.test/events/remote%2Fid HTTP/1.1\r\n"
    ));
    assert!(
        requests[2]
            .headers
            .to_ascii_lowercase()
            .contains("if-match: \"v1\"\r\n")
    );
    assert!(requests[3].headers.starts_with(
        "DELETE /calendar/v3/calendars/team%40example.test/events/remote%2Fid HTTP/1.1\r\n"
    ));
    assert!(
        requests[3]
            .headers
            .to_ascii_lowercase()
            .contains("if-match: \"v2\"\r\n")
    );
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn google_http_distinguishes_waiting_conflict_and_unknown_writes() -> Result<()> {
    let cases = [
        (401, FailureKind::Waiting),
        (429, FailureKind::Waiting),
        (412, FailureKind::Rejected),
        (408, FailureKind::Uncertain),
        (503, FailureKind::Uncertain),
    ];
    let replies = cases
        .iter()
        .flat_map(|(status, _)| [reply(*status, "{}"), reply(*status, "{}")])
        .collect();
    let (provider, server) = fixture(replies).await?;
    let event = event()?;
    for (_, expected) in cases {
        assert_eq!(
            provider
                .save("fixture-token", "edit", &event)
                .await
                .expect_err("refused save")
                .kind,
            expected
        );
        assert_eq!(
            provider
                .delete("fixture-token", &event)
                .await
                .expect_err("refused delete")
                .kind,
            expected
        );
    }
    assert_eq!(server.await??.len(), 10);
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn google_http_lost_success_body_and_delete_reply_remain_uncertain() -> Result<()> {
    let (provider, server) = fixture(vec![
        "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"id\":".into(),
        String::new(),
    ])
    .await?;
    let event = event()?;
    assert_eq!(
        provider
            .save("fixture-token", "edit", &event)
            .await
            .expect_err("truncated save")
            .kind,
        FailureKind::Uncertain
    );
    assert_eq!(
        provider
            .delete("fixture-token", &event)
            .await
            .expect_err("lost delete response")
            .kind,
        FailureKind::Uncertain
    );
    assert_eq!(server.await??.len(), 2);
    Ok(())
}
