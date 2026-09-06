use super::*;
use crate::providers::test_http::{Reply, Server};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Mutex as StdMutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Default)]
struct Credentials {
    saved: StdMutex<Option<SecretString>>,
    reads: AtomicUsize,
    writes: AtomicUsize,
    locked: AtomicBool,
    write_gate: StdMutex<Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>,
}
#[async_trait]
impl tokens::CredentialStore for Credentials {
    async fn read(&self) -> anyhow::Result<Option<SecretString>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(
            !self.locked.load(Ordering::SeqCst),
            "Fixture keychain locked"
        );
        Ok(self.saved.lock().unwrap().clone())
    }
    async fn write(&self, value: SecretString) -> anyhow::Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        let gate = self.write_gate.lock().unwrap().take();
        if let Some((entered, release)) = gate {
            entered.notify_one();
            release.notified().await;
        }
        anyhow::ensure!(
            !self.locked.load(Ordering::SeqCst),
            "Fixture keychain locked"
        );
        *self.saved.lock().unwrap() = Some(value);
        Ok(())
    }
}
impl Credentials {
    fn seed(&self, value: Value) {
        *self.saved.lock().unwrap() = Some(SecretString::from(value.to_string()));
    }
    fn value(&self) -> Value {
        serde_json::from_str(self.saved.lock().unwrap().as_ref().unwrap().expose_secret()).unwrap()
    }
}
fn prefs() -> Preferences {
    Preferences {
        google_client_id: "fixture-client".into(),
        google_client_secret: "fixture&secret+/".into(),
        ..Default::default()
    }
}
fn saved(expires: i64) -> Value {
    json!({"client_id":"fixture-client", "access_token":"old-access", "refresh_token":"old-refresh&+/", "expires_at":expires})
}
fn response(access: &str, refresh: Option<&str>) -> Reply {
    let mut value = json!({"access_token":access, "expires_in":3600, "token_type":"Bearer"});
    if let Some(refresh) = refresh {
        value["refresh_token"] = refresh.into();
    }
    Reply::new(200, value.to_string())
}
fn google(server: &Server, credentials: Arc<Credentials>) -> Google {
    Google {
        http: crate::providers::test_http::client(),
        state: Default::default(),
        credentials,
        token_endpoint: server.url.join("/token").unwrap(),
    }
}
fn form(server: &Server, index: usize) -> HashMap<String, String> {
    let requests = server.requests();
    let request = &requests[index];
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/token");
    assert_eq!(
        request.headers["content-type"],
        "application/x-www-form-urlencoded"
    );
    assert!(!request.headers.contains_key("authorization"));
    url::form_urlencoded::parse(&request.bytes)
        .into_owned()
        .collect()
}

#[tokio::test]
async fn cancelled_google_keychain_write_cannot_overtake_the_next_sign_in() {
    let queue = tokens::WriteQueue::default();
    let entered = Arc::new(tokio::sync::Notify::new());
    let (release, blocked) = std::sync::mpsc::channel();
    let writes = Arc::new(StdMutex::new(Vec::new()));
    let first_queue = queue.clone();
    let first_writes = writes.clone();
    let first_entered = entered.clone();
    let first = tokio::spawn(async move {
        first_queue
            .run(move || {
                first_entered.notify_one();
                blocked.recv_timeout(Duration::from_secs(10)).unwrap();
                first_writes.lock().unwrap().push("old grant");
                Ok(())
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(10), entered.notified())
        .await
        .unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let next_writes = writes.clone();
    let mut next = Box::pin(queue.run(move || {
        next_writes.lock().unwrap().push("new grant");
        Ok(())
    }));
    // The cancelled async task is gone, but its OS operation still owns the
    // write queue. This assertion needs no timing or sleep.
    assert!(futures::poll!(next.as_mut()).is_pending());
    release.send(()).unwrap();
    next.await.unwrap();
    assert_eq!(*writes.lock().unwrap(), ["old grant", "new grant"]);
}

#[tokio::test]
async fn concurrent_google_refresh_coalesces_and_rotation_survives_restart() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(0));
    let mut server = Server::start(vec![
        response("rotated-access", Some("rotated-refresh&+/")),
        response("next-access", None),
    ])
    .await;
    let google = google(&server, credentials.clone());
    let prefs = prefs();
    let values = futures::future::join_all((0..8).map(|_| google.token(&prefs))).await;
    assert!(
        values
            .iter()
            .all(|v| v.as_ref().unwrap().expose_secret() == "rotated-access")
    );
    assert_eq!(server.requests().len(), 1);
    assert_eq!(credentials.reads.load(Ordering::SeqCst), 1);
    assert_eq!(credentials.writes.load(Ordering::SeqCst), 1);
    assert_eq!(form(&server, 0)["refresh_token"], "old-refresh&+/");
    assert_eq!(form(&server, 0)["client_secret"], "fixture&secret+/");
    assert_eq!(form(&server, 0)["grant_type"], "refresh_token");
    let mut saved = credentials.value();
    assert_eq!(saved["refresh_token"], "rotated-refresh&+/");
    saved["expires_at"] = 0.into();
    credentials.seed(saved);
    drop(google);
    let restarted = self::google(&server, credentials.clone());
    assert_eq!(
        restarted.token(&prefs).await.unwrap().expose_secret(),
        "next-access"
    );
    assert_eq!(form(&server, 1)["refresh_token"], "rotated-refresh&+/");
    assert_eq!(credentials.value()["refresh_token"], "rotated-refresh&+/");
    server.finish().await;
}

#[tokio::test]
async fn google_refresh_keychain_failure_retries_saving_the_rotated_grant_before_use() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(0));
    let mut server = Server::start(vec![response("new-access", Some("new-refresh"))]).await;
    let google = google(&server, credentials.clone());
    assert!(google.connected(&prefs()).await.unwrap());
    credentials.locked.store(true, Ordering::SeqCst);
    for _ in 0..2 {
        let error = google.token(&prefs()).await.unwrap_err().to_string();
        assert!(error.contains("renewed") && error.contains("Keep Shep open"));
        assert_eq!(server.requests().len(), 1);
        assert_eq!(credentials.value()["refresh_token"], "old-refresh&+/");
    }
    assert!(!google.connected(&prefs()).await.unwrap());
    credentials.locked.store(false, Ordering::SeqCst);
    assert_eq!(
        google.token(&prefs()).await.unwrap().expose_secret(),
        "new-access"
    );
    assert_eq!(credentials.value()["refresh_token"], "new-refresh");
    assert!(google.connected(&prefs()).await.unwrap());
    assert_eq!(server.requests().len(), 1);
    server.finish().await;
}

#[tokio::test]
async fn pending_google_sign_in_cannot_mix_accounts_or_repeat_an_exchanged_code() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
    let mut server = Server::start(vec![response(
        "new-account-access",
        Some("new-account-refresh"),
    )])
    .await;
    let google = google(&server, credentials.clone());
    assert!(google.connected(&prefs()).await.unwrap());
    credentials.locked.store(true, Ordering::SeqCst);
    let error = google
        .exchange_code(
            &prefs(),
            "one-use-code&+",
            "http://127.0.0.1:9876/callback",
            "fixture-verifier",
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("authorization was received"));
    assert_eq!(credentials.value()["refresh_token"], "old-refresh&+/");
    assert!(
        google
            .token(&prefs())
            .await
            .unwrap_err()
            .to_string()
            .contains("Finish Google sign-in")
    );
    assert!(google.finish_pending_login(&prefs()).await.is_err());
    assert_eq!(server.requests().len(), 1);
    credentials.locked.store(false, Ordering::SeqCst);
    // Production login's pending-grant branch must return before browser launch.
    google.login(&prefs()).await.unwrap();
    assert_eq!(
        google.token(&prefs()).await.unwrap().expose_secret(),
        "new-account-access"
    );
    assert_eq!(credentials.value()["refresh_token"], "new-account-refresh");
    let form = form(&server, 0);
    assert_eq!(form["code"], "one-use-code&+");
    assert_eq!(form["code_verifier"], "fixture-verifier");
    assert_eq!(form["redirect_uri"], "http://127.0.0.1:9876/callback");
    assert!(!form.contains_key("refresh_token"));
    server.finish().await;
}

#[tokio::test]
async fn google_login_waits_for_inflight_refresh_persistence_and_then_wins() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    *credentials.write_gate.lock().unwrap() = Some((entered.clone(), release.clone()));
    let mut server = Server::start(vec![
        response("refreshed", Some("rotated")),
        response("new-account", Some("new-grant")),
    ])
    .await;
    let google = google(&server, credentials.clone());
    let worker = google.clone();
    let refresh = tokio::spawn(async move { worker.token(&prefs()).await });
    tokio::time::timeout(Duration::from_secs(10), entered.notified())
        .await
        .expect("Refresh did not reach its keychain checkpoint");
    let prefs = prefs();
    let mut login = Box::pin(google.exchange_code(
        &prefs,
        "code",
        "http://127.0.0.1:3456/callback",
        "verifier",
    ));
    assert!(futures::poll!(login.as_mut()).is_pending());
    assert_eq!(server.requests().len(), 1);
    release.notify_one();
    assert_eq!(refresh.await.unwrap().unwrap().expose_secret(), "refreshed");
    login.await.unwrap();
    assert_eq!(credentials.value()["refresh_token"], "new-grant");
    assert_eq!(
        google.token(&prefs).await.unwrap().expose_secret(),
        "new-account"
    );
    server.finish().await;
}

#[tokio::test]
async fn google_sign_in_without_offline_access_preserves_the_previous_account() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
    let mut server = Server::start(vec![response("new-account", None)]).await;
    let google = google(&server, credentials.clone());
    let error = google
        .exchange_code(
            &prefs(),
            "code",
            "http://127.0.0.1:1234/callback",
            "verifier",
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("offline access"));
    assert_eq!(credentials.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        google.token(&prefs()).await.unwrap().expose_secret(),
        "old-access"
    );
    server.finish().await;
}

#[tokio::test]
async fn google_revoked_grants_require_reconnect_while_client_errors_can_be_corrected() {
    for invalid_grant in [false, true] {
        let credentials = Arc::new(Credentials::default());
        credentials.seed(saved(0));
        let code = if invalid_grant {
            "invalid_grant"
        } else {
            "invalid_client"
        };
        let mut responses = vec![Reply::new(
            400,
            json!({"error":code,"error_description":"NEVER-EXPOSE-THIS-CREDENTIAL"}).to_string(),
        )];
        if !invalid_grant {
            responses.push(response("recovered", None));
        }
        let mut server = Server::start(responses).await;
        let google = google(&server, credentials.clone());
        let mut prefs = prefs();
        let error = format!("{:#}", google.token(&prefs).await.unwrap_err());
        assert!(!error.contains("NEVER-EXPOSE"));
        assert_eq!(credentials.writes.load(Ordering::SeqCst), 0);
        if invalid_grant {
            assert!(!google.connected(&prefs).await.unwrap());
            assert!(
                google
                    .token(&prefs)
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("Reconnect Google")
            );
            assert_eq!(server.requests().len(), 1);
        } else {
            assert!(error.contains("client ID and client secret"));
            prefs.google_client_secret = "corrected-secret".into();
            assert_eq!(
                google.token(&prefs).await.unwrap().expose_secret(),
                "recovered"
            );
            assert_eq!(form(&server, 1)["client_secret"], "corrected-secret");
        }
        server.finish().await;
    }
}

#[tokio::test]
async fn google_malformed_or_unsuccessful_responses_never_replace_saved_credentials() {
    let responses = vec![
        Reply::new(302, "{\"access_token\":\"NEVER-EXPOSE\",\"refresh_token\":\"secret\",\"expires_in\":3600,\"token_type\":\"Bearer\"}").header("Location", "https://unrelated.invalid/steal"),
        Reply::new(503, "NEVER-EXPOSE"), Reply::disconnect(),
        Reply::new(200, "{\"expires_in\":\"NEVER-EXPOSE\"}"),
        Reply::new(200, "{\"access_token\":\"a\",\"access_token\":\"b\"}"),
        Reply::new(200, "{\"access_token\":\"NEVER-EXPOSE\",\"expires_in\":-1,\"token_type\":\"Bearer\"}"),
        Reply::new(200, "{\"access_token\":\"NEVER-EXPOSE\",\"expires_in\":0,\"token_type\":\"Bearer\"}"),
        Reply::new(200, "{\"access_token\":\"NEVER-EXPOSE\",\"expires_in\":18446744073709551615,\"token_type\":\"Bearer\"}"),
        Reply::new(200, "{\"access_token\":\"NEVER-EXPOSE\",\"expires_in\":3600,\"token_type\":\"MAC\"}"),
        Reply::new(200, "{\"access_token\":\"\",\"expires_in\":3600,\"token_type\":\"Bearer\"}"),
        Reply::new(200, "{\"access_token\":\"a\\nNEVER-EXPOSE\",\"expires_in\":3600,\"token_type\":\"Bearer\"}"),
        Reply::new(200, "{\"access_token\":\"a\",\"refresh_token\":\"\",\"expires_in\":3600,\"token_type\":\"Bearer\"}"),
        Reply::new(200, "{\"error\":\"invalid_grant\"}"),
        Reply::new(200, "").header("Content-Length", "65537"),
        Reply::chunked(200, vec![b' '; 65537]),
    ];
    for response in responses {
        let credentials = Arc::new(Credentials::default());
        credentials.seed(saved(0));
        let mut server = Server::start(vec![response]).await;
        let google = google(&server, credentials.clone());
        let error = format!("{:#}", google.token(&prefs()).await.unwrap_err());
        assert!(!error.contains("NEVER-EXPOSE"));
        assert_eq!(credentials.writes.load(Ordering::SeqCst), 0);
        assert_eq!(credentials.value(), saved(0));
        server.finish().await;
    }
}

#[tokio::test]
async fn google_saved_credentials_are_client_bound_and_keychain_errors_are_not_missing_grants() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
    let server = Server::start(vec![]).await;
    let google = google(&server, credentials.clone());
    let mut prefs = prefs();
    prefs.google_client_id = "another-client".into();
    assert!(!google.connected(&prefs).await.unwrap());
    assert!(
        google
            .token(&prefs)
            .await
            .unwrap_err()
            .to_string()
            .contains("Reconnect Google")
    );
    prefs.google_client_id = "fixture-client".into();
    assert_eq!(
        google.token(&prefs).await.unwrap().expose_secret(),
        "old-access"
    );
    prefs.google_client_id.clear();
    assert!(!google.connected(&prefs).await.unwrap());
    assert!(google.token(&prefs).await.is_err());
    drop(google);
    let mut legacy = credentials.value();
    legacy.as_object_mut().unwrap().remove("client_id");
    credentials.seed(legacy);
    let google = self::google(&server, credentials.clone());
    assert!(!google.connected(&self::prefs()).await.unwrap());
    assert!(google.token(&self::prefs()).await.is_err());
    drop(google);
    credentials.locked.store(true, Ordering::SeqCst);
    let google = self::google(&server, credentials.clone());
    assert!(google.connected(&self::prefs()).await.is_err());
    credentials.locked.store(false, Ordering::SeqCst);
    credentials.seed(json!({"access_token":"NEVER-EXPOSE","expires_at":"NEVER-EXPOSE"}));
    let error = format!("{:#}", google.token(&self::prefs()).await.unwrap_err());
    assert!(!error.contains("NEVER-EXPOSE"));
    *credentials.saved.lock().unwrap() = None;
    assert!(!google.connected(&self::prefs()).await.unwrap());
    assert_eq!(credentials.writes.load(Ordering::SeqCst), 0);
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn google_callback_ignores_invalid_requests_and_accepts_fragmented_headers() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let receiver = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(10),
            callback::receive(listener, "expected-state"),
        )
        .await
        .expect("Callback did not finish")
    });
    let _idle_browser_connection = tokio::net::TcpStream::connect(addr).await.unwrap();
    for target in [
        "/callback?state=wrong&code=bad",
        "/favicon.ico",
        "/callback?state=expected-state&state=other&code=bad",
        "/callback?state=expected-state&code=one&code=two",
        "/callback?state=expected-state&code=bad&error=access_denied",
        "/callback?state=expected-state&code=",
        "/callback?state=expected-state&code=bad%0Avalue",
    ] {
        let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
        socket
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(!response.contains("expected-state"));
    }
    let target = "/callback?state=expected-state&code=valid";
    let mut invalid_requests = vec![
        format!("POST {target} HTTP/1.1\r\nHost: {addr}\r\n\r\n"),
        format!("GET {target} HTTP/1.1\r\nHost: unrelated.example\r\n\r\n"),
        format!("GET {target} HTTP/1.1\r\nHost: {addr}\r\nHost: {addr}\r\n\r\n"),
    ];
    let prefix = format!("GET {target} HTTP/1.1\r\nHost: {addr}\r\nX-Padding: ");
    invalid_requests.push(format!(
        "{prefix}{}\r\n\r\n",
        "x".repeat(8193 - prefix.len() - 4)
    ));
    for request in invalid_requests {
        let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
        socket.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).await.unwrap();
        assert!(response.starts_with("HTTP/1.1 400"));
    }
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(b"GET /callback?state=expected-state&code=valid%2Bcode HTTP/1.1\r\n")
        .await
        .unwrap();
    socket
        .write_all(format!("Host: {addr}\r\n").as_bytes())
        .await
        .unwrap();
    socket.write_all(b"\r\n").await.unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("Cache-Control: no-store"));
    assert!(!response.contains("valid+code"));
    assert_eq!(
        receiver.await.unwrap().unwrap().expose_secret(),
        "valid+code"
    );
}

#[tokio::test]
async fn google_callback_denial_with_matching_state_acknowledges_the_browser() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let receiver = tokio::spawn(async move {
        tokio::time::timeout(
            Duration::from_secs(10),
            callback::receive(listener, "state"),
        )
        .await
        .expect("Callback did not finish")
    });
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(
            format!(
                "GET /callback?state=state&error=access_denied HTTP/1.1\r\nHost: {addr}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).await.unwrap();
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("cancelled"));
    assert!(
        receiver
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("cancelled or denied")
    );
}
