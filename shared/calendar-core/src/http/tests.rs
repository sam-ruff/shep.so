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

fn restorable(status: &str, etag: &str) -> Value {
    let mut value: Value = serde_json::from_str(&saved("remote/id", etag)).expect("fixture");
    value["status"] = json!(status);
    value["iCalUID"] = json!("owned-uid@example.test");
    value["organizer"] = json!({"self":true,"email":"owner@example.test"});
    value["attendees"] = json!([{"email":"guest@example.test","responseStatus":"accepted"}]);
    value["conferenceData"] = json!({"conferenceId":"retained"});
    value["attachments"] = json!([{"fileUrl":"https://example.test/file"}]);
    value
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn google_status_only_cancellation_and_restore_preserve_original_status() -> Result<()> {
    for status in ["confirmed", "tentative"] {
        let (provider, server) = fixture(vec![
            reply(200, &restorable(status, "\"v1\"").to_string()),
            reply(200, &restorable("cancelled", "\"v2\"").to_string()),
            reply(200, &restorable(status, "\"v3\"").to_string()),
        ])
        .await?;
        let plan = provider.prepare_delete("token", &event()?).await?;
        let plan = serde_json::from_str(&serde_json::to_string(&plan)?)?;
        let receipt = provider.cancel_event("token", &plan).await?;
        let request = receipt.restore_request();
        let request = serde_json::from_str(&serde_json::to_string(&request)?)?;
        let restored = provider.restore_event("token", &request).await?;
        assert_eq!(restored.etag.as_deref(), Some("\"v3\""));
        let requests = server.await??;
        assert!(requests[0].headers.starts_with("GET "));
        for (index, expected, version) in [(1, "cancelled", "v1"), (2, status, "v2")] {
            assert!(requests[index].headers.starts_with("PATCH "));
            assert!(requests[index].headers.contains("conferenceDataVersion=1"));
            assert!(requests[index].headers.contains("supportsAttachments=true"));
            assert!(
                requests[index]
                    .headers
                    .to_lowercase()
                    .contains(&format!("if-match: \"{version}\""))
            );
            assert_eq!(
                serde_json::from_slice::<Value>(&requests[index].body)?,
                json!({"status":expected})
            );
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn google_restore_refuses_unowned_recurring_stale_or_sparse_events() -> Result<()> {
    let mut cases = Vec::new();
    for (key, value) in [
        (
            "organizer",
            json!({"self":false,"email":"owner@example.test"}),
        ),
        ("recurrence", json!(["RRULE:FREQ=DAILY"])),
        ("recurringEventId", json!("series")),
        ("originalStartTime", json!({"date":"2026-09-20"})),
        ("etag", json!("\"changed\"")),
        ("id", json!("another")),
        ("status", json!("cancelled")),
        ("start", Value::Null),
        ("iCalUID", Value::Null),
    ] {
        let mut value_body = restorable("confirmed", "\"v1\"");
        value_body[key] = value;
        cases.push(value_body);
    }
    for value in cases {
        let (provider, server) = fixture(vec![reply(200, &value.to_string())]).await?;
        assert_eq!(
            provider
                .prepare_delete("token", &event()?)
                .await
                .expect_err("unsafe plan")
                .kind,
            FailureKind::Rejected
        );
        assert_eq!(server.await??.len(), 1);
    }
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn google_cancellation_unknown_is_not_recovered_as_an_acknowledgement() -> Result<()> {
    let (provider, server) = fixture(vec![
        reply(200, &restorable("confirmed", "\"v1\"").to_string()),
        reply(200, "{}"),
        reply(200, &restorable("cancelled", "\"v2\"").to_string()),
        reply(404, "{}"),
    ])
    .await?;
    let plan = provider.prepare_delete("token", &event()?).await?;
    assert_eq!(
        provider
            .cancel_event("token", &plan)
            .await
            .expect_err("unknown")
            .kind,
        FailureKind::Uncertain
    );
    assert_eq!(
        provider.inspect_deletion("token", &plan).await?,
        crate::restoration::Inspection::Cancelled {
            etag: "\"v2\"".into()
        }
    );
    assert_eq!(
        provider.inspect_deletion("token", &plan).await?,
        crate::restoration::Inspection::Missing
    );
    let requests = server.await??;
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.headers.starts_with("PATCH "))
            .count(),
        1
    );
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn google_restore_preserves_conflict_and_unknown_boundaries() -> Result<()> {
    for (code, body, expected) in [
        (412, "{}", FailureKind::Rejected),
        (401, "{}", FailureKind::Waiting),
        (500, "{}", FailureKind::Uncertain),
        (200, "{}", FailureKind::Uncertain),
    ] {
        let (provider, server) = fixture(vec![
            reply(200, &restorable("tentative", "\"v1\"").to_string()),
            reply(200, &restorable("cancelled", "\"v2\"").to_string()),
            reply(code, body),
        ])
        .await?;
        let plan = provider.prepare_delete("token", &event()?).await?;
        let receipt = provider.cancel_event("token", &plan).await?;
        assert_eq!(
            provider
                .restore_event("token", &receipt.restore_request())
                .await
                .expect_err("failure")
                .kind,
            expected
        );
        assert_eq!(server.await??.len(), 3);
    }
    Ok(())
}

#[test]
fn restoration_deserialisation_rejects_invalid_versions_and_unknown_fields() -> Result<()> {
    let plan = crate::restoration::DeletePlan::new(
        event()?,
        crate::restoration::LiveStatus::Tentative,
        "uid".into(),
        "owner@example.test".into(),
    )?;
    for etag in ["", "*", "W/\"v1\"", "\"v1\", \"v2\"", "\"\r\n\""] {
        let mut value = serde_json::to_value(&plan)?;
        value["before"]["etag"] = json!(etag);
        assert!(serde_json::from_value::<crate::restoration::DeletePlan>(value).is_err());
    }
    let mut value = serde_json::to_value(&plan)?;
    value["unknown"] = json!(true);
    assert!(serde_json::from_value::<crate::restoration::DeletePlan>(value).is_err());
    let receipt = crate::restoration::DeleteReceipt::acknowledged(plan, "\"v2\"".into())?;
    let mut value = serde_json::to_value(receipt.restore_request())?;
    value["receipt"]["cancelled_etag"] = json!("\"v1\"");
    assert!(serde_json::from_value::<crate::restoration::RestoreRequest>(value).is_err());
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn google_restoration_lost_wire_reply_never_retries_or_creates() -> Result<()> {
    for restoring in [false, true] {
        let mut replies = vec![reply(200, &restorable("confirmed", "\"v1\"").to_string())];
        if restoring {
            replies.push(reply(200, &restorable("cancelled", "\"v2\"").to_string()));
        }
        replies.push(String::new());
        let (provider, server) = fixture(replies).await?;
        let plan = provider.prepare_delete("token", &event()?).await?;
        let failure = if restoring {
            let receipt = provider.cancel_event("token", &plan).await?;
            let request = receipt.restore_request();
            let saved = serde_json::to_string(&request)?;
            let failure = provider
                .restore_event("token", &request)
                .await
                .expect_err("lost reply");
            assert_eq!(serde_json::to_string(&request)?, saved);
            failure
        } else {
            provider
                .cancel_event("token", &plan)
                .await
                .expect_err("lost reply")
        };
        assert_eq!(failure.kind, FailureKind::Uncertain);
        let requests = server.await??;
        assert_eq!(requests.len(), if restoring { 3 } else { 2 });
        assert!(
            requests
                .iter()
                .all(|request| !request.headers.starts_with("POST "))
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn google_cancelled_inspection_requires_owned_full_identity() -> Result<()> {
    for (key, value) in [
        ("id", json!("replacement")),
        ("iCalUID", json!("replacement-uid")),
        (
            "organizer",
            json!({"self":true,"email":"replacement@example.test"}),
        ),
        ("etag", Value::Null),
        ("start", Value::Null),
        ("recurringEventId", json!("series")),
    ] {
        let mut cancelled = restorable("cancelled", "\"v2\"");
        cancelled[key] = value;
        let (provider, server) = fixture(vec![
            reply(200, &restorable("confirmed", "\"v1\"").to_string()),
            reply(200, &cancelled.to_string()),
        ])
        .await?;
        let plan = provider.prepare_delete("token", &event()?).await?;
        assert_eq!(
            provider
                .inspect_deletion("token", &plan)
                .await
                .expect_err("unsafe observation")
                .kind,
            FailureKind::Rejected
        );
        assert!(
            server
                .await??
                .iter()
                .all(|request| request.headers.starts_with("GET "))
        );
    }
    Ok(())
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
