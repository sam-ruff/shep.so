use super::*;
use crate::FailureKind;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Request {
    headers: String,
}

fn reply(status: u16, etag: Option<&str>, body: &str) -> String {
    let etag = etag
        .map(|value| format!("ETag: {value}\r\n"))
        .unwrap_or_default();
    format!(
        "HTTP/1.1 {status} Fixture\r\n{etag}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

async fn fixture(
    replies: Vec<String>,
) -> anyhow::Result<(
    CalDavProvider,
    tokio::task::JoinHandle<anyhow::Result<Vec<Request>>>,
)> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let url = format!("http://{}/home/", listener.local_addr()?);
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in replies {
            let (mut stream, _) = listener.accept().await?;
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut chunk = [0; 1024];
                let count = stream.read(&mut chunk).await?;
                anyhow::ensure!(count > 0, "fixture request ended early");
                bytes.extend_from_slice(&chunk[..count]);
                anyhow::ensure!(bytes.len() <= 65_536, "fixture request exceeded its bound");
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8(bytes[..header_end].to_vec())?;
            let length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>())
                })
                .transpose()?
                .unwrap_or(0);
            anyhow::ensure!(length <= 65_536, "fixture body exceeded its bound");
            while bytes.len() < header_end + length {
                let mut chunk = [0; 1024];
                let count = stream.read(&mut chunk).await?;
                anyhow::ensure!(count > 0, "fixture body ended early");
                bytes.extend_from_slice(&chunk[..count]);
            }
            requests.push(Request { headers });
            stream.write_all(response.as_bytes()).await?;
            stream.shutdown().await?;
        }
        Ok(requests)
    });
    Ok((
        CalDavProvider::new(CalDavConnection {
            id: "home".into(),
            url,
            username: "sam".into(),
        })?,
        server,
    ))
}

fn event() -> anyhow::Result<Event> {
    Ok(Event {
        id: "walk".into(),
        source_id: "home".into(),
        title: "Walk".into(),
        start: "2026-09-20T10:00:00Z".parse()?,
        end: "2026-09-20T11:00:00Z".parse()?,
        location: String::new(),
        description: String::new(),
        all_day: false,
        etag: None,
        remote_url: None,
    })
}

fn resource() -> String {
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:walk\r\nDTSTART:20260920T100000Z\r\nDTEND:20260920T110000Z\r\nSUMMARY:Walk\r\nDESCRIPTION:\r\nLOCATION:\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n".into()
}

#[tokio::test]
async fn recurring_and_weak_version_writes_are_rejected_before_network() -> anyhow::Result<()> {
    let provider = CalDavProvider::new(CalDavConnection {
        id: "home".into(),
        url: "http://127.0.0.1:1/home/".into(),
        username: "fixture".into(),
    })?;
    let mut recurring = event()?;
    recurring.etag = Some("\"v1\"".into());
    assert_eq!(
        provider
            .delete("secret", &recurring)
            .await
            .expect_err("recurrence")
            .kind,
        FailureKind::Rejected
    );
    assert_eq!(
        provider
            .save("secret", "request", &recurring)
            .await
            .expect_err("recurrence")
            .kind,
        FailureKind::Rejected
    );
    recurring.remote_url = Some("walk.ics".into());
    for version in ["W/\"v1\"", "*", "\"v1\", \"v2\""] {
        recurring.etag = Some(version.into());
        assert_eq!(
            provider
                .delete("secret", &recurring)
                .await
                .expect_err("version")
                .kind,
            FailureKind::Rejected
        );
        assert_eq!(
            provider
                .save("secret", "request", &recurring)
                .await
                .expect_err("version")
                .kind,
            FailureKind::Rejected
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn caldav_create_uses_reserved_identity_and_requires_version() -> anyhow::Result<()> {
    let (provider, server) =
        fixture(vec![reply(201, Some("\"v1\""), ""), reply(201, None, "")]).await?;
    let saved = provider.save("secret", "request", &event()?).await?;
    assert_eq!(saved.id, "walk");
    assert_eq!(saved.etag.as_deref(), Some("\"v1\""));
    let failure = provider
        .save("secret", "request-two", &event()?)
        .await
        .expect_err("missing version is uncertain");
    assert_eq!(failure.kind, FailureKind::Uncertain);
    let requests = server.await??;
    for request in requests {
        assert!(request.headers.starts_with("PUT /home/walk.ics "));
        assert!(request.headers.contains("if-none-match: *"));
        assert!(request.headers.contains("authorization: Basic "));
    }
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn caldav_discovery_cannot_borrow_another_collection_privilege() -> anyhow::Result<()> {
    let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
        <d:response><d:href>/home/</d:href><d:propstat><d:prop><d:resourcetype><c:calendar/></d:resourcetype><d:displayname>Home</d:displayname></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
        <d:response><d:href>/other/</d:href><d:propstat><d:prop><d:current-user-privilege-set><d:privilege><d:write/></d:privilege></d:current-user-privilege-set></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
        </d:multistatus>"#;
    let (provider, server) = fixture(vec![reply(207, None, xml)]).await?;
    let sources = provider.sources("secret").await?;
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].name, "Home");
    assert!(sources[0].read_only);
    let requests = server.await??;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].headers.starts_with("PROPFIND /home/ "));
    assert!(requests[0].headers.contains("depth: 0"));
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn caldav_distinguishes_waiting_rejection_and_unknown_write() -> anyhow::Result<()> {
    let (provider, server) = fixture(vec![
        reply(401, None, ""),
        reply(412, None, ""),
        reply(503, None, ""),
    ])
    .await?;
    for expected in [
        FailureKind::Waiting,
        FailureKind::Rejected,
        FailureKind::Uncertain,
    ] {
        let failure = provider
            .save("secret", "request", &event()?)
            .await
            .expect_err("status must fail");
        assert_eq!(failure.kind, expected);
    }
    assert_eq!(server.await??.len(), 3);
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn caldav_edit_and_delete_use_exact_versions() -> anyhow::Result<()> {
    let (provider, server) = fixture(vec![
        reply(200, Some("\"v1\""), &resource()),
        reply(204, Some("\"v2\""), ""),
        reply(204, None, ""),
    ])
    .await?;
    let mut changed = event()?;
    changed.etag = Some("\"v1\"".into());
    changed.remote_url = Some("walk.ics".into());
    changed.title = "Long walk".into();
    let saved = provider.save("secret", "edit", &changed).await?;
    assert_eq!(saved.etag.as_deref(), Some("\"v2\""));
    provider.delete("secret", &saved).await?;
    let requests = server.await??;
    assert!(requests[0].headers.starts_with("GET /home/walk.ics "));
    assert!(requests[1].headers.starts_with("PUT /home/walk.ics "));
    assert!(requests[1].headers.contains("if-match: \"v1\""));
    assert!(requests[2].headers.starts_with("DELETE /home/walk.ics "));
    assert!(requests[2].headers.contains("if-match: \"v2\""));
    Ok(())
}

#[tokio::test]
#[ignore = "owned loopback HTTP contract"]
async fn caldav_redirect_and_lost_mutation_replies_are_uncertain() -> anyhow::Result<()> {
    let (provider, server) = fixture(vec![reply(302, None, ""), String::new()]).await?;
    for request_id in ["redirect", "lost-reply"] {
        let failure = provider
            .save("secret", request_id, &event()?)
            .await
            .expect_err("write cannot be confirmed");
        assert_eq!(failure.kind, FailureKind::Uncertain);
    }
    assert_eq!(server.await??.len(), 2);
    Ok(())
}
