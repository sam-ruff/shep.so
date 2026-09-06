use super::*;

#[test]
fn complete_plain_text_headers_and_template_like_values_remain_literal() {
    let body = format!(
        "{}\nEND-OF-LONG-MESSAGE",
        "A long line of text\n".repeat(3000)
    );
    let raw = format!(
        "From: Alex <alex@example.test>\r\nTo: Jo <jo@example.test>\r\nSubject: Literal {{{{DOCUMENT}}}} </script>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}"
    );
    let rendered = document::prepare(raw.as_bytes(), Options::default(), "fixed-nonce").unwrap();
    assert!(rendered.contains("END-OF-LONG-MESSAGE"));
    assert!(rendered.contains("Literal {{DOCUMENT}} \\u003c/script>"));
    assert!(rendered.contains("sandbox=\"allow-same-origin allow-modals\""));
    assert!(rendered.contains("script-src 'none'"));
    assert!(!rendered.contains("allow-scripts"));
}

#[test]
fn html_is_inert_and_only_supplied_rasters_are_embedded() {
    let raw = b"From: Alex <alex@example.test>\r\nSubject: Styled mail\r\nContent-Type: text/html\r\n\r\n<html><head><style>.card{color:red;background:url(https://blocked.example.test/css)}</style><base href='https://blocked.example.test'><meta http-equiv='refresh' content='0;url=https://blocked.example.test'></head><body bgcolor='#fafafa'><h1 class='card'>Hello</h1><script>EVIL_SCRIPT</script><svg><script>EVIL_SVG</script></svg><img src='https://images.example.test/allowed' onerror='EVIL_EVENT' srcset='https://blocked.example.test/set 2x'><img src='https://blocked.example.test/image'><iframe src='https://blocked.example.test/frame'></iframe><a href='javascript:EVIL_LINK'>No</a><a href='https://example.test/docs'>Docs</a></body></html>";
    let mut options = Options::default();
    options.images.insert(
        "https://images.example.test/allowed".into(),
        Arc::from(include_bytes!("../../assets/logo-light.webp").as_slice()),
    );
    let rendered = document::prepare(raw, options, "nonce").unwrap();
    assert!(rendered.contains("data:image/webp;base64,"));
    assert!(rendered.contains("bgcolor=&quot;#fafafa&quot;"));
    assert!(rendered.contains(".card{color:red"));
    for forbidden in [
        "EVIL_SCRIPT",
        "EVIL_EVENT",
        "EVIL_SVG",
        "EVIL_LINK",
        "srcset=",
        "http-equiv=&quot;refresh",
        "blocked.example.test/image",
        "blocked.example.test/frame",
    ] {
        assert!(!rendered.contains(forbidden), "{forbidden}");
    }
    let plain = document::prepare(
        raw,
        Options {
            plain: true,
            ..Default::default()
        },
        "nonce",
    )
    .unwrap();
    assert!(!plain.contains(".card{color:red"));
    assert!(plain.contains("Hello"));
}

async fn request(
    url: &str,
    method: &str,
    host_override: Option<&str>,
    path_override: Option<&str>,
) -> String {
    let url = url::Url::parse(url).unwrap();
    let host = format!("127.0.0.1:{}", url.port().unwrap());
    let mut socket = TcpStream::connect(&host).await.unwrap();
    socket
        .write_all(
            format!(
                "{method} {} HTTP/1.1\r\nHost: {}\r\n\r\n",
                path_override.unwrap_or(url.path()),
                host_override.unwrap_or(&host)
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).await.unwrap();
    response
}

#[tokio::test]
async fn server_requires_host_token_method_and_consumes_once_without_disk_files() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = listener.local_addr().unwrap().to_string();
    let url = format!("http://{host}/print/token");
    let server = tokio::spawn(serve(
        listener,
        host,
        "/print/token".into(),
        "PRIVATE-MESSAGE".into(),
        "nonce".into(),
    ));
    for (method, host, path) in [
        ("GET", Some("attacker.example.test"), None),
        ("GET", None, Some("/")),
        ("POST", None, None),
    ] {
        let denied = request(&url, method, host, path).await;
        assert!(denied.starts_with("HTTP/1.1 404"));
        assert!(!denied.contains("PRIVATE-MESSAGE"));
    }
    let head = request(&url, "HEAD", None, None).await;
    assert!(head.starts_with("HTTP/1.1 200"));
    assert!(!head.contains("PRIVATE-MESSAGE"));
    let get = request(&url, "GET", None, None).await;
    assert!(get.ends_with("PRIVATE-MESSAGE"));
    assert!(get.contains("Cache-Control: no-store"));
    assert!(get.contains("Referrer-Policy: no-referrer"));
    tokio::time::timeout(Duration::from_secs(1), server)
        .await
        .unwrap()
        .unwrap();
    assert!(
        TcpStream::connect(
            url::Url::parse(&url)
                .unwrap()
                .socket_addrs(|| None)
                .unwrap()[0]
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn dropping_unopened_preview_releases_the_server() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let host = listener.local_addr().unwrap().to_string();
    let task = tokio::spawn(serve(
        listener,
        host.clone(),
        "/print/token".into(),
        "message".into(),
        "nonce".into(),
    ));
    let preview = Preview {
        url: format!("http://{host}/print/token"),
        task: task.abort_handle(),
    };
    drop(preview);
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(TcpStream::connect(host).await.is_err());
}

#[tokio::test]
async fn preparation_is_bounded_and_failed_or_expired_previews_release_capacity() {
    let store = crate::store::Store::memory().unwrap();
    let mail = crate::model::parse_mail(
        "work",
        "1",
        "INBOX",
        b"Subject: Test\r\n\r\nComplete body".to_vec(),
        true,
        false,
    )
    .unwrap();
    let id = mail.summary.id.clone();
    store.upsert(vec![mail]).await.unwrap();
    let service = Service::default();
    assert!(
        service
            .prepare(&store, "missing", Options::default())
            .await
            .is_err()
    );
    let first = service
        .prepare(&store, &id, Options::default())
        .await
        .unwrap();
    let second = service
        .prepare(&store, &id, Options::default())
        .await
        .unwrap();
    assert!(
        service
            .prepare(&store, &id, Options::default())
            .await
            .is_err()
    );
    drop(first);
    tokio::task::yield_now().await;
    let third = service
        .prepare(&store, &id, Options::default())
        .await
        .unwrap();
    assert!(
        request(&third.url, "GET", None, None)
            .await
            .contains("Complete body")
    );
    tokio::task::yield_now().await;
    let fourth = service
        .prepare(&store, &id, Options::default())
        .await
        .unwrap();
    drop((second, third, fourth));
}
