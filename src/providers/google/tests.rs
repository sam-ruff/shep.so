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
    async fn delete(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.locked.load(Ordering::SeqCst),
            "Fixture keychain locked"
        );
        *self.saved.lock().unwrap() = None;
        Ok(())
    }
}
impl Credentials {
    fn seed(&self, value: Value) {
        *self.saved.lock().unwrap() = Some(SecretString::from(value.to_string()));
    }
    fn value(&self) -> Value {
        let value: Value =
            serde_json::from_str(self.saved.lock().unwrap().as_ref().unwrap().expose_secret())
                .unwrap();
        if let Some(grants) = value["grants"].as_array() {
            assert_eq!(
                grants.len(),
                1,
                "Use the raw vault for multi-grant assertions"
            );
            grants[0].clone()
        } else {
            value
        }
    }
}
const CLIENT: &str = "fixture-client";
const CLIENT_SECRET: &str = "fixture&secret+/";

/// Grants saved before grants recorded their client relied on these stored
/// values; most tests keep this build's client identical to them.
fn prefs() -> Preferences {
    Preferences {
        google_client_id: CLIENT.into(),
        google_client_secret: CLIENT_SECRET.into(),
        google_services: Some(crate::model::GoogleServices {
            drive: true,
            calendar: crate::model::GoogleCalendarRequest::ReadWrite,
        }),
        ..Default::default()
    }
}

fn consent_preferences(drive: bool, calendar: crate::model::GoogleCalendarRequest) -> Preferences {
    Preferences {
        google_services: Some(crate::model::GoogleServices { drive, calendar }),
        ..prefs()
    }
}

#[test]
fn google_authorization_url_requests_only_selected_services_with_pkce_and_account_choice() {
    use crate::model::GoogleCalendarRequest::*;
    for (calendar, events) in [
        (Off, None),
        (ReadOnly, Some("calendar.events.readonly")),
        (ReadWrite, Some("calendar.events")),
    ] {
        for drive in [false, true] {
            let prefs = consent_preferences(drive, calendar);
            if !drive && calendar == Off {
                assert!(
                    consent::authorization_url(
                        CLIENT,
                        &prefs,
                        "http://127.0.0.1:9876/callback",
                        "state",
                        "challenge",
                        true
                    )
                    .is_err()
                );
                continue;
            }
            for retry in [false, true] {
                let url = consent::authorization_url(
                    "built-in.apps.googleusercontent.com",
                    &prefs,
                    "http://127.0.0.1:9876/callback",
                    "fixture-state&+",
                    "fixture-challenge",
                    retry,
                )
                .unwrap();
                assert_eq!(url.host_str(), Some("accounts.google.com"));
                assert_eq!(url.scheme(), "https");
                let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
                // This build's client, never the stored self-configured one.
                assert_eq!(query["client_id"], "built-in.apps.googleusercontent.com");
                let actual: std::collections::BTreeSet<_> =
                    query["scope"].split_ascii_whitespace().collect();
                let mut expected = std::collections::BTreeSet::new();
                if drive {
                    expected.insert("https://www.googleapis.com/auth/drive.appdata".to_owned());
                }
                if let Some(events) = events {
                    expected.insert(format!("https://www.googleapis.com/auth/{events}"));
                    expected.insert(
                        "https://www.googleapis.com/auth/calendar.calendarlist.readonly".into(),
                    );
                }
                assert_eq!(actual, expected.iter().map(String::as_str).collect());
                assert_eq!(query["state"], "fixture-state&+");
                assert_eq!(query["code_challenge_method"], "S256");
                assert_eq!(query["code_challenge"], "fixture-challenge");
                assert_eq!(query["access_type"], "offline");
                assert_eq!(
                    query["prompt"],
                    if retry {
                        "consent"
                    } else {
                        "consent select_account"
                    }
                );
                assert!(!query.contains_key("client_secret"));
                assert!(!query.contains_key("include_granted_scopes"));
            }
        }
    }
}

#[tokio::test]
async fn google_omitted_token_scope_inherits_exact_request_and_never_all_services() {
    use crate::model::GoogleCalendarRequest::*;
    for (drive, calendar) in [(true, Off), (false, ReadOnly), (false, ReadWrite)] {
        let prefs = consent_preferences(drive, calendar);
        let credentials = Arc::new(Credentials::default());
        let mut server = Server::start(vec![response("scoped", Some("scoped-refresh"))]).await;
        let google = google(&server, credentials.clone());
        let grant = google
            .exchange_code(
                &prefs,
                "fixture-code",
                "http://127.0.0.1:9876/callback",
                "verifier",
            )
            .await
            .unwrap();
        assert_eq!(grant.access.drive, drive);
        assert_eq!(grant.access.calendar_read, calendar != Off);
        assert_eq!(grant.access.calendar_write, calendar == ReadWrite);
        let saved = credentials.value();
        assert_eq!(saved["scope"], consent::requested_scopes(&prefs).unwrap());
        assert_eq!(saved["requested_scopes"], saved["scope"]);
        assert_eq!(server.requests().len(), 1);
        server.finish().await;
    }
}

#[tokio::test]
async fn google_broader_returned_scopes_do_not_enable_unselected_services_after_refresh() {
    use crate::model::GoogleCalendarRequest::*;
    let mut prefs = consent_preferences(false, ReadOnly);
    let credentials = Arc::new(Credentials::default());
    let mut server = Server::start(vec![
        scoped_response(SCOPES),
        Reply::new(200, r#"{"items":[]}"#),
        scoped_response(SCOPES),
    ])
    .await;
    let google = google(&server, credentials.clone());
    let grant = google
        .exchange_code(
            &prefs,
            "fixture-code",
            "http://127.0.0.1:9876/callback",
            "verifier",
        )
        .await
        .unwrap();
    assert!(grant.access.calendar_read);
    assert!(!grant.access.calendar_write && !grant.access.drive);
    let (grant, identity, _) = google.prepare_grant(&prefs, grant).await.unwrap();
    assert!(identity.is_none());
    assert_eq!(server.requests().len(), 2);
    prefs.google_grant = grant;
    google.finish_activation(&prefs).await.unwrap();
    let mut saved = credentials.value();
    saved["expires_at"] = 0.into();
    credentials.seed(saved);
    drop(google);
    prefs.google_services = consent_preferences(true, ReadWrite).google_services;
    let restarted = self::google(&server, credentials.clone());
    restarted
        .token_for(&prefs, Service::CalendarRead)
        .await
        .unwrap();
    assert!(
        restarted
            .token_for(&prefs, Service::CalendarWrite)
            .await
            .is_err()
    );
    assert!(restarted.token_for(&prefs, Service::Drive).await.is_err());
    assert_eq!(server.requests().len(), 3);
    assert_eq!(
        credentials.value()["requested_scopes"],
        consent::requested_scopes(&consent_preferences(false, ReadOnly)).unwrap()
    );
    server.finish().await;
}

#[tokio::test]
async fn google_pending_login_retry_is_bound_to_requested_services_in_memory_and_after_restart() {
    use crate::model::GoogleCalendarRequest::*;
    let original = consent_preferences(true, Off);
    let changed = consent_preferences(false, ReadOnly);
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
    let mut server = Server::start(vec![response("candidate", Some("candidate-refresh"))]).await;
    let google = google(&server, credentials.clone());
    google.connected(&original).await.unwrap();
    credentials.locked.store(true, Ordering::SeqCst);
    assert!(
        google
            .exchange_code(
                &original,
                "fixture-code",
                "http://127.0.0.1:9876/callback",
                "verifier"
            )
            .await
            .is_err()
    );
    credentials.locked.store(false, Ordering::SeqCst);
    assert!(
        google
            .finish_pending_login(&changed)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        google.token(&original).await.unwrap().expose_secret(),
        "old-access"
    );
    let candidate = google
        .finish_pending_login(&original)
        .await
        .unwrap()
        .unwrap();
    drop(google);
    let restarted = self::google(&server, credentials);
    assert!(
        restarted
            .finish_pending_login(&changed)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        restarted
            .finish_pending_login(&original)
            .await
            .unwrap()
            .unwrap(),
        candidate
    );
    assert_eq!(
        restarted.token(&changed).await.unwrap().expose_secret(),
        "old-access"
    );
    assert_eq!(server.requests().len(), 1);
    server.finish().await;
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
    signing_in(
        server,
        credentials,
        client::OAuthClient::new(CLIENT, CLIENT_SECRET),
        Arc::new(NoBrowser),
        SIGN_IN_WAIT,
    )
}
fn signing_in(
    server: &Server,
    credentials: Arc<Credentials>,
    client: Option<client::OAuthClient>,
    browser: Arc<dyn Browser>,
    wait: Duration,
) -> Google {
    Google::start(
        crate::providers::test_http::client(),
        credentials,
        server.url.join("/token").unwrap(),
        server.url.join("/").unwrap(),
        SignIn {
            client,
            browser,
            wait,
        },
    )
}

/// Any browser launch in these tests is a failure.
struct NoBrowser;
impl Browser for NoBrowser {
    fn open(&self, _: &str) -> anyhow::Result<()> {
        anyhow::bail!("Unexpected browser launch")
    }
}

/// Hands the consent URL to the test, which then plays the user's browser.
struct FakeBrowser(tokio::sync::mpsc::UnboundedSender<url::Url>);
impl Browser for FakeBrowser {
    fn open(&self, url: &str) -> anyhow::Result<()> {
        self.0
            .send(url::Url::parse(url)?)
            .map_err(|_| anyhow::anyhow!("Fixture browser closed"))
    }
}
fn fake_browser() -> (
    Arc<dyn Browser>,
    tokio::sync::mpsc::UnboundedReceiver<url::Url>,
) {
    let (opened, consent) = tokio::sync::mpsc::unbounded_channel();
    (Arc::new(FakeBrowser(opened)), consent)
}
fn query(url: &url::Url) -> HashMap<String, String> {
    url.query_pairs().into_owned().collect()
}
/// Sends the browser's redirect back to the loopback listener named in `consent`.
async fn redirect(consent: &url::Url, callback_query: &str) -> String {
    let target = url::Url::parse(&query(consent)["redirect_uri"]).unwrap();
    let authority = format!("127.0.0.1:{}", target.port().unwrap());
    let mut socket = tokio::net::TcpStream::connect(&authority).await.unwrap();
    socket
        .write_all(
            format!("GET /callback?{callback_query} HTTP/1.1\r\nHost: {authority}\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).await.unwrap();
    response
}
fn sign_in_error(error: &anyhow::Error) -> Option<SignInError> {
    error.downcast_ref::<SignInError>().copied()
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
    struct Writes {
        entered: Option<tokio::sync::oneshot::Sender<()>>,
        held: Option<tokio::sync::oneshot::Receiver<()>>,
        writes: std::sync::mpsc::SyncSender<String>,
    }
    impl crate::credentials::Backend for Writes {
        fn read(&mut self, _: &str) -> anyhow::Result<Option<SecretString>> {
            Ok(None)
        }
        fn write(&mut self, key: &str, secret: SecretString) -> anyhow::Result<()> {
            assert_eq!(key, "google-oauth");
            if let Some(entered) = self.entered.take() {
                entered.send(()).unwrap();
                self.held.take().unwrap().blocking_recv().unwrap();
            }
            self.writes.send(secret.expose_secret().to_owned()).unwrap();
            Ok(())
        }
        fn delete(&mut self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let (release, held) = tokio::sync::oneshot::channel();
    let (writes, results) = std::sync::mpsc::sync_channel(2);
    let credentials = Arc::new(tokens::OsCredentialStore {
        credentials: crate::credentials::Credentials::with_backend(
            crate::credentials::Scope::Legacy,
            Writes {
                entered: Some(entered),
                held: Some(held),
                writes,
            },
        ),
    });
    let first = tokio::spawn({
        let credentials = credentials.clone();
        async move { credentials.write("old grant".into()).await }
    });
    waiting.await.unwrap();
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    let mut next = Box::pin(credentials.write("new grant".into()));
    assert!(futures::poll!(next.as_mut()).is_pending());
    release.send(()).unwrap();
    next.await.unwrap();
    assert_eq!(results.try_recv().unwrap(), "old grant");
    assert_eq!(results.try_recv().unwrap(), "new grant");
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
    assert_eq!(
        google.token(&prefs()).await.unwrap().expose_secret(),
        "old-access"
    );
    assert!(google.finish_pending_login(&prefs()).await.is_err());
    assert_eq!(server.requests().len(), 1);
    credentials.locked.store(false, Ordering::SeqCst);
    // Production login's pending-grant branch must return before browser launch.
    let grant = google.login(&prefs()).await.unwrap();
    assert_eq!(
        google.token(&prefs()).await.unwrap().expose_secret(),
        "old-access"
    );
    let authorized = Preferences {
        google_grant: grant,
        ..prefs()
    };
    google.finish_activation(&authorized).await.unwrap();
    assert_eq!(
        google.token(&authorized).await.unwrap().expose_secret(),
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
    let grant = login.await.unwrap();
    assert_eq!(
        google.token(&prefs).await.unwrap().expose_secret(),
        "refreshed"
    );
    let prefs = Preferences {
        google_grant: grant,
        ..prefs
    };
    google.finish_activation(&prefs).await.unwrap();
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
        let prefs = prefs();
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
            assert!(error.contains("sign-in client"));
            // A build with the rotated secret refreshes the same grant.
            let corrected = signing_in(
                &server,
                credentials.clone(),
                client::OAuthClient::new(CLIENT, "corrected-secret"),
                Arc::new(NoBrowser),
                SIGN_IN_WAIT,
            );
            assert_eq!(
                corrected.token(&prefs).await.unwrap().expose_secret(),
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
    let error = receiver.await.unwrap().unwrap_err();
    assert_eq!(sign_in_error(&error), Some(SignInError::Denied));
    assert!(error.to_string().contains("denied in the browser"));
}

#[tokio::test]
async fn disconnect_clears_cached_and_saved_grants_and_fresh_signin_can_reconnect() {
    let server = Server::start(vec![response("new-account", Some("new-refresh"))]).await;
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
    let google = google(&server, credentials.clone());
    assert!(google.connected(&prefs()).await.unwrap());
    google.clear_credentials().await.unwrap();
    assert!(!google.connected(&prefs()).await.unwrap());
    assert!(google.token(&prefs()).await.is_err());
    assert!(credentials.saved.lock().unwrap().is_none());
    google.clear_credentials().await.unwrap();
    let grant = google
        .exchange_code(
            &prefs(),
            "new-code",
            "http://127.0.0.1:7/callback",
            "verifier",
        )
        .await
        .unwrap();
    let prefs = Preferences {
        google_grant: grant,
        ..prefs()
    };
    google.finish_activation(&prefs).await.unwrap();
    assert_eq!(
        google.token(&prefs).await.unwrap().expose_secret(),
        "new-account"
    );
    assert_eq!(credentials.value()["refresh_token"], "new-refresh");
}

#[tokio::test]
async fn failed_credential_deletion_blocks_cached_grants_and_is_retryable() {
    let server = Server::start(vec![]).await;
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
    let google = google(&server, credentials.clone());
    assert!(google.token(&prefs()).await.is_ok());
    credentials.locked.store(true, Ordering::SeqCst);
    assert!(
        google
            .clear_credentials()
            .await
            .unwrap_err()
            .to_string()
            .contains("Retry Google cleanup")
    );
    assert!(google.token(&prefs()).await.is_err());
    assert!(!google.connected(&prefs()).await.unwrap());
    let mut disconnected = prefs();
    disconnected.google_lifecycle.disconnected = true;
    let reopened = self::google(&server, credentials.clone());
    assert!(!reopened.connected(&disconnected).await.unwrap());
    assert!(reopened.token(&disconnected).await.is_err());
    credentials.locked.store(false, Ordering::SeqCst);
    google.clear_credentials().await.unwrap();
    assert!(credentials.saved.lock().unwrap().is_none());
    assert!(server.requests().is_empty());
}

fn scoped_response(scope: &str) -> Reply {
    Reply::new(200, json!({"access_token":"candidate-access", "refresh_token":"candidate-refresh", "expires_in":3600, "token_type":"Bearer", "scope":scope}).to_string())
}

#[tokio::test]
async fn partial_google_grants_validate_only_granted_services_and_enforce_readonly_access() {
    for (scope, drive, write) in [
        ("https://www.googleapis.com/auth/drive.appdata", true, false),
        (
            "https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/calendar.calendarlist.readonly",
            false,
            true,
        ),
        (
            "https://www.googleapis.com/auth/calendar.readonly",
            false,
            false,
        ),
    ] {
        let reply = if drive {
            r#"{"user":{"permissionId":"candidate"}}"#
        } else {
            r#"{"items":[{"id":"work","accessRole":"owner","summary":"Work"}]}"#
        };
        let mut server = Server::start(vec![scoped_response(scope), Reply::new(200, reply)]).await;
        let credentials = Arc::new(Credentials::default());
        credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
        let google = google(&server, credentials.clone());
        let prefs = prefs();
        let grant = google
            .exchange_code(&prefs, "once", "http://127.0.0.1/callback", "verifier")
            .await
            .unwrap();
        assert_eq!(
            google.token(&prefs).await.unwrap().expose_secret(),
            "old-access"
        );
        let (grant, identity, sources) = google.prepare_grant(&prefs, grant).await.unwrap();
        assert_eq!(grant.access.drive, drive);
        assert_eq!(grant.access.calendar_write, write);
        assert_eq!(identity, drive.then(|| "drive:candidate".into()));
        if !drive {
            assert_eq!(sources[0].access.create, write);
        }
        let authorized = Preferences {
            google_grant: grant,
            ..prefs
        };
        let denied = if drive {
            Service::CalendarRead
        } else {
            Service::Drive
        };
        assert!(
            google
                .token_for(&authorized, denied)
                .await
                .unwrap_err()
                .to_string()
                .contains("did not grant")
        );
        if !write {
            assert!(
                google
                    .token_for(&authorized, Service::CalendarWrite)
                    .await
                    .is_err()
            );
        }
        let requests = server.requests();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].target.starts_with(if drive {
            "/drive/v3/about"
        } else {
            "/calendar/v3/users/me/calendarList"
        }));
        assert_eq!(
            requests[1].headers["authorization"],
            "Bearer candidate-access"
        );
        server.finish().await;
    }
}

#[tokio::test]
async fn staged_google_activation_survives_validation_failure_database_rollback_and_restart() {
    let mut server = Server::start(vec![
        scoped_response("https://www.googleapis.com/auth/calendar.readonly"),
        Reply::new(503, "NEVER-EXPOSE-service-error"),
        Reply::new(200, r#"{"items":[{"id":"new","accessRole":"owner"}]}"#),
    ])
    .await;
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
    let google = google(&server, credentials.clone());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite");
    let store = crate::store::Store::open(&path).unwrap();
    let prefs = prefs();
    store.put("preferences", prefs.clone()).await.unwrap();
    let grant = google
        .exchange_code(&prefs, "once", "http://127.0.0.1/callback", "verifier")
        .await
        .unwrap();
    let error = google
        .prepare_grant(&prefs, grant.clone())
        .await
        .unwrap_err();
    assert!(!format!("{error:#}").contains("NEVER-EXPOSE"));
    assert_eq!(
        google.token(&prefs).await.unwrap().expose_secret(),
        "old-access"
    );
    drop(google);
    let google = self::google(&server, credentials.clone());
    // Retry uses the saved candidate, without browser or repeating the one-use code.
    assert_eq!(google.login(&prefs).await.unwrap(), grant);
    let (grant, identity, sources) = google.prepare_grant(&prefs, grant).await.unwrap();
    store.run(|c| { c.execute_batch("CREATE TRIGGER fail_grant BEFORE INSERT ON kv WHEN NEW.key='google_archived' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;")?; Ok(()) }).await.unwrap();
    assert!(
        store
            .activate_google(
                prefs.clone(),
                grant.clone(),
                identity.clone(),
                sources.clone()
            )
            .await
            .is_err()
    );
    assert_eq!(
        store.get::<Preferences>("preferences").await.unwrap(),
        prefs
    );
    assert!(store.workspace().await.unwrap().calendars.is_empty());
    assert_eq!(
        google.token(&prefs).await.unwrap().expose_secret(),
        "old-access"
    );
    store
        .run(|c| {
            c.execute_batch("DROP TRIGGER fail_grant")?;
            Ok(())
        })
        .await
        .unwrap();
    store
        .activate_google(prefs.clone(), grant.clone(), identity, sources)
        .await
        .unwrap();
    // Simulate exit immediately after DB commit and before credential pruning.
    drop(store);
    drop(google);
    let store = crate::store::Store::open(&path).unwrap();
    let committed: Preferences = store.get("preferences").await.unwrap();
    assert_eq!(committed.google_grant, grant);
    let google = self::google(&server, credentials.clone());
    assert_eq!(
        google.token(&committed).await.unwrap().expose_secret(),
        "candidate-access"
    );
    assert!(
        google
            .finish_pending_login(&committed)
            .await
            .unwrap()
            .is_none()
    );
    credentials.locked.store(true, Ordering::SeqCst);
    assert!(google.finish_activation(&committed).await.is_err());
    assert_eq!(
        google.token(&committed).await.unwrap().expose_secret(),
        "candidate-access"
    );
    credentials.locked.store(false, Ordering::SeqCst);
    google.finish_activation(&committed).await.unwrap();
    let restarted = self::google(&server, credentials.clone());
    assert_eq!(
        restarted.token(&committed).await.unwrap().expose_secret(),
        "candidate-access"
    );
    assert!(restarted.token(&prefs).await.is_err());
    assert_eq!(credentials.value()["refresh_token"], "candidate-refresh");
    server.finish().await;
}

#[tokio::test]
async fn unhelpful_partial_grants_keep_the_working_google_connection() {
    for scope in [
        "",
        "https://www.googleapis.com/auth/calendar.calendarlist.readonly",
        "openid email",
    ] {
        let mut server = Server::start(vec![scoped_response(scope)]).await;
        let credentials = Arc::new(Credentials::default());
        credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
        let google = google(&server, credentials.clone());
        assert!(
            google
                .exchange_code(&prefs(), "once", "http://127.0.0.1/callback", "verifier")
                .await
                .is_err()
        );
        assert_eq!(credentials.writes.load(Ordering::SeqCst), 0);
        assert_eq!(
            google.token(&prefs()).await.unwrap().expose_secret(),
            "old-access"
        );
        server.finish().await;
    }
}

#[tokio::test]
async fn editing_new_oauth_client_details_keeps_the_committed_client_and_refresh_secret() {
    let mut server = Server::start(vec![
        response("candidate", Some("candidate-refresh")),
        response("renewed", None),
    ])
    .await;
    let credentials = Arc::new(Credentials::default());
    let google = google(&server, credentials.clone());
    let grant = google
        .exchange_code(&prefs(), "once", "http://127.0.0.1/callback", "verifier")
        .await
        .unwrap();
    let mut committed = Preferences {
        google_grant: grant,
        ..prefs()
    };
    google.finish_activation(&committed).await.unwrap();
    let mut value = credentials.value();
    value["expires_at"] = 0.into();
    credentials.seed(value);
    drop(google);
    let google = self::google(&server, credentials.clone());
    committed.google_client_id = "edited-new-client".into();
    committed.google_client_secret = "edited-new-secret".into();
    assert!(google.connected(&committed).await.unwrap());
    assert_eq!(
        google.token(&committed).await.unwrap().expose_secret(),
        "renewed"
    );
    assert_eq!(form(&server, 1)["client_id"], "fixture-client");
    assert_eq!(form(&server, 1)["client_secret"], "fixture&secret+/");
    assert_eq!(credentials.value()["client_id"], "fixture-client");
    server.finish().await;
}

#[test]
fn google_scope_aliases_require_both_list_and_event_access() {
    for scope in [
        "calendar",
        "calendar.events calendar.calendarlist",
        "calendar.events calendar.calendarlist.readonly",
    ] {
        let scope = scope
            .split(' ')
            .map(|s| format!("https://www.googleapis.com/auth/{s}"))
            .collect::<Vec<_>>()
            .join(" ");
        let access = scopes::access(Some(&scope));
        assert!(access.calendar_read && access.calendar_write && !access.drive);
    }
    assert!(!scopes::access(Some("https://www.googleapis.com/auth/calendar.events")).calendar_read);
    assert!(scopes::access(None).calendar_allowed()); // legacy permission metadata
    assert!(!scopes::access(Some("unrelated")).calendar_allowed());
}

#[tokio::test]
async fn resumed_google_candidate_rechecks_scopes_after_refresh_before_service_validation() {
    let mut server = Server::start(vec![
        response("initial-candidate", Some("candidate-refresh")),
        Reply::new(200, json!({"access_token":"renewed-candidate", "expires_in":3600, "token_type":"Bearer", "scope":"https://www.googleapis.com/auth/calendar.readonly"}).to_string()),
        Reply::new(200, r#"{"items":[]}"#),
    ]).await;
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
    let google = google(&server, credentials.clone());
    let grant = google
        .exchange_code(&prefs(), "once", "http://127.0.0.1/callback", "verifier")
        .await
        .unwrap();
    assert!(grant.access.drive);
    let mut vault: Value = serde_json::from_str(
        credentials
            .saved
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .expose_secret(),
    )
    .unwrap();
    vault["grants"][1]["expires_at"] = 0.into();
    credentials.seed(vault);
    drop(google);
    let google = self::google(&server, credentials.clone());
    let (grant, identity, sources) = google.prepare_grant(&prefs(), grant).await.unwrap();
    assert!(grant.access.calendar_read && !grant.access.calendar_write && !grant.access.drive);
    assert!(identity.is_none() && sources.is_empty());
    assert_eq!(server.requests().len(), 3);
    assert!(server.requests()[2].target.starts_with("/calendar/"));
    assert_eq!(
        google.token(&prefs()).await.unwrap().expose_secret(),
        "old-access"
    );
    server.finish().await;
}

#[tokio::test]
async fn calendar_roles_cannot_restore_writes_after_refresh_reduces_scope() {
    let mut server = Server::start(vec![
        Reply::new(200, json!({"access_token":"renewed", "expires_in":3600, "token_type":"Bearer", "scope":"https://www.googleapis.com/auth/calendar.readonly"}).to_string()),
        Reply::new(200, r#"{"items":[{"id":"work","accessRole":"owner"}]}"#),
    ]).await;
    let credentials = Arc::new(Credentials::default());
    let mut value = saved(0);
    value["scope"] = SCOPES.into();
    credentials.seed(value);
    let google = google(&server, credentials);
    let sources = google.calendars(&prefs()).await.unwrap();
    assert!(sources[0].access.read_only());
    assert!(
        google
            .token_for(&prefs(), Service::CalendarWrite)
            .await
            .is_err()
    );
    assert_eq!(server.requests().len(), 2);
    server.finish().await;
}

#[tokio::test]
async fn cancelled_refresh_observer_still_persists_the_rotated_token_in_order() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    *credentials.write_gate.lock().unwrap() = Some((entered.clone(), release.clone()));
    let mut server = Server::start(vec![response("rotated", Some("rotated-refresh"))]).await;
    let google = google(&server, credentials.clone());
    let worker = google.clone();
    let refresh = tokio::spawn(async move { worker.token(&prefs()).await });
    tokio::time::timeout(Duration::from_secs(10), entered.notified())
        .await
        .expect("Refresh did not reach its keychain checkpoint");
    // The observer leaves while the owner is mid-save; the admitted job drains.
    refresh.abort();
    assert!(refresh.await.unwrap_err().is_cancelled());
    let prefs = prefs();
    let mut connected = Box::pin(google.connected(&prefs));
    assert!(futures::poll!(connected.as_mut()).is_pending());
    assert!(
        credentials
            .saved
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| { !s.expose_secret().contains("rotated-refresh") })
    );
    release.notify_one();
    assert!(connected.await.unwrap());
    assert_eq!(credentials.value()["refresh_token"], "rotated-refresh");
    assert_eq!(server.requests().len(), 1);
    assert_eq!(
        google.token(&prefs).await.unwrap().expose_secret(),
        "rotated"
    );
    assert_eq!(server.requests().len(), 1);
    server.finish().await;
}

#[tokio::test]
async fn dropping_the_last_google_handle_drains_the_running_save_before_exit() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    *credentials.write_gate.lock().unwrap() = Some((entered.clone(), release.clone()));
    let mut server = Server::start(vec![response("rotated", Some("rotated-refresh"))]).await;
    let google = google(&server, credentials.clone());
    let worker = google.clone();
    let refresh = tokio::spawn(async move { worker.token(&prefs()).await });
    tokio::time::timeout(Duration::from_secs(10), entered.notified())
        .await
        .expect("Refresh did not reach its keychain checkpoint");
    refresh.abort();
    drop(google);
    release.notify_one();
    tokio::time::timeout(Duration::from_secs(10), async {
        while credentials.writes.load(Ordering::SeqCst) == 0
            || credentials
                .saved
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|s| !s.expose_secret().contains("rotated-refresh"))
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the owner persisted the rotated token before exiting");
    let restarted = self::google(&server, credentials.clone());
    let mut value = credentials.value();
    value["expires_at"] = (chrono::Utc::now().timestamp() + 3600).into();
    credentials.seed(value);
    assert_eq!(
        restarted.token(&prefs()).await.unwrap().expose_secret(),
        "rotated"
    );
    assert_eq!(server.requests().len(), 1);
    server.finish().await;
}

#[tokio::test]
async fn a_locked_keychain_fails_every_queued_caller_without_stopping_the_owner() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
    credentials.locked.store(true, Ordering::SeqCst);
    let server = Server::start(vec![]).await;
    let google = google(&server, credentials.clone());
    let prefs = prefs();
    let results = futures::future::join_all((0..4).map(|_| google.token(&prefs))).await;
    assert!(results.iter().all(|r| r.is_err()));
    assert!(google.connected(&prefs).await.is_err());
    credentials.locked.store(false, Ordering::SeqCst);
    assert_eq!(
        google.token(&prefs).await.unwrap().expose_secret(),
        "old-access"
    );
    assert!(server.requests().is_empty());
}

#[test]
fn pkce_uses_rfc_7636_s256_and_a_fresh_verifier_for_every_sign_in() {
    // RFC 7636 appendix B.
    assert_eq!(
        consent::challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
    let first = consent::Pkce::new();
    let second = consent::Pkce::new();
    assert_ne!(*first.verifier, *second.verifier);
    for pkce in [first, second] {
        // 32 random bytes; RFC 7636 allows 43 to 128 unreserved characters.
        assert_eq!(pkce.verifier.len(), 43);
        assert!(
            pkce.verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        );
        assert_eq!(pkce.challenge, consent::challenge(&pkce.verifier));
    }
}

/// A fresh install: no self-configured client was ever stored.
fn new_user() -> Preferences {
    Preferences {
        google_client_id: String::new(),
        google_client_secret: String::new(),
        ..consent_preferences(true, crate::model::GoogleCalendarRequest::ReadWrite)
    }
}

#[tokio::test]
async fn sign_in_uses_the_built_in_client_a_loopback_redirect_pkce_and_the_exact_state() {
    let credentials = Arc::new(Credentials::default());
    let mut server = Server::start(vec![response("fresh", Some("fresh-refresh"))]).await;
    let (browser, mut opened) = fake_browser();
    let google = signing_in(
        &server,
        credentials.clone(),
        client::OAuthClient::new("built-in.apps.googleusercontent.com", "built-in-secret"),
        browser,
        SIGN_IN_WAIT,
    );
    let login = tokio::spawn({
        let google = google.clone();
        async move {
            google
                .login_with_retry(&new_user(), false, &CancellationToken::new())
                .await
        }
    });
    let consent = opened.recv().await.unwrap();
    let params = query(&consent);
    assert_eq!(consent.host_str(), Some("accounts.google.com"));
    assert_eq!(params["client_id"], "built-in.apps.googleusercontent.com");
    assert!(!params.contains_key("client_secret"));
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["prompt"], "consent select_account");
    let callback = url::Url::parse(&params["redirect_uri"]).unwrap();
    assert_eq!(callback.host_str(), Some("127.0.0.1"));
    assert_eq!(callback.path(), "/callback");
    assert!(callback.port().is_some_and(|port| port != 0));
    let state = &params["state"];
    assert_eq!(state.len(), 43);
    let upper = state.to_ascii_uppercase();
    for wrong in [
        format!("{state}x"),
        state[..42].to_owned(),
        upper,
        String::new(),
    ] {
        let response = redirect(&consent, &format!("state={wrong}&code=stolen")).await;
        assert!(response.starts_with("HTTP/1.1 400"), "{wrong}");
    }
    assert!(!login.is_finished());
    assert!(server.requests().is_empty());
    let response = redirect(&consent, &format!("state={state}&code=fixture-code")).await;
    assert!(response.starts_with("HTTP/1.1 200"));
    let grant = login.await.unwrap().unwrap();
    assert_eq!(grant.client_id, "built-in.apps.googleusercontent.com");
    let form = form(&server, 0);
    assert_eq!(form["client_id"], "built-in.apps.googleusercontent.com");
    assert_eq!(form["client_secret"], "built-in-secret");
    assert_eq!(form["code"], "fixture-code");
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["redirect_uri"], params["redirect_uri"]);
    assert_eq!(
        consent::challenge(&form["code_verifier"]),
        params["code_challenge"]
    );
    let vault: Value = serde_json::from_str(
        credentials
            .saved
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .expose_secret(),
    )
    .unwrap();
    assert_eq!(
        vault["grants"][0]["client_id"],
        "built-in.apps.googleusercontent.com"
    );
    server.finish().await;
}

#[tokio::test]
async fn denied_refused_cancelled_and_timed_out_sign_ins_leave_the_connection_unchanged() {
    for outcome in [
        SignInError::Denied,
        SignInError::Refused,
        SignInError::Cancelled,
        SignInError::TimedOut,
    ] {
        let credentials = Arc::new(Credentials::default());
        credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
        let server = Server::start(vec![]).await;
        let (browser, mut opened) = fake_browser();
        let wait = if outcome == SignInError::TimedOut {
            Duration::from_millis(300)
        } else {
            SIGN_IN_WAIT
        };
        let google = signing_in(
            &server,
            credentials.clone(),
            client::OAuthClient::new(CLIENT, CLIENT_SECRET),
            browser,
            wait,
        );
        let cancel = CancellationToken::new();
        let login = tokio::spawn({
            let google = google.clone();
            let cancel = cancel.clone();
            async move { google.login_with_retry(&prefs(), true, &cancel).await }
        });
        let consent = opened.recv().await.unwrap();
        let state = query(&consent)["state"].clone();
        match outcome {
            SignInError::Denied => {
                let response =
                    redirect(&consent, &format!("state={state}&error=access_denied")).await;
                assert!(response.starts_with("HTTP/1.1 200") && response.contains("cancelled"));
            }
            SignInError::Refused => {
                let response =
                    redirect(&consent, &format!("state={state}&error=invalid_scope")).await;
                assert!(response.contains("did not complete"));
            }
            SignInError::Cancelled => cancel.cancel(),
            _ => {}
        }
        let error = tokio::time::timeout(Duration::from_secs(10), login)
            .await
            .expect("sign-in stopped")
            .unwrap()
            .unwrap_err();
        assert_eq!(sign_in_error(&error), Some(outcome), "{error}");
        assert!(error.to_string().contains("Nothing changed"));
        // The loopback listener is closed once sign-in stops.
        let callback = url::Url::parse(&query(&consent)["redirect_uri"]).unwrap();
        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", callback.port().unwrap()))
                .await
                .is_err()
        );
        assert!(server.requests().is_empty());
        assert_eq!(credentials.writes.load(Ordering::SeqCst), 0);
        assert_eq!(
            google.token(&prefs()).await.unwrap().expose_secret(),
            "old-access"
        );
    }
}

#[tokio::test]
async fn a_cancel_before_the_browser_opens_never_binds_a_listener_or_opens_the_browser() {
    let server = Server::start(vec![]).await;
    let google = google(&server, Arc::new(Credentials::default()));
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = google
        .login_with_retry(&prefs(), true, &cancel)
        .await
        .unwrap_err();
    assert_eq!(sign_in_error(&error), Some(SignInError::Cancelled));
}

#[tokio::test]
async fn an_expired_authorization_code_asks_for_a_new_sign_in_and_keeps_the_connection() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
    let mut server = Server::start(vec![Reply::new(
        400,
        json!({"error":"invalid_grant","error_description":"NEVER-EXPOSE"}).to_string(),
    )])
    .await;
    let google = google(&server, credentials.clone());
    let error = google
        .exchange_code(
            &prefs(),
            "expired-code",
            "http://127.0.0.1:1/callback",
            "verifier",
        )
        .await
        .unwrap_err();
    assert_eq!(sign_in_error(&error), Some(SignInError::CodeExpired));
    assert!(!format!("{error:#}").contains("NEVER-EXPOSE"));
    assert_eq!(credentials.writes.load(Ordering::SeqCst), 0);
    assert!(google.connected(&prefs()).await.unwrap());
    assert_eq!(
        google.token(&prefs()).await.unwrap().expose_secret(),
        "old-access"
    );
    server.finish().await;
}

#[tokio::test]
async fn an_unconfigured_build_cannot_sign_in_but_keeps_refreshing_a_saved_grant() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(0));
    let mut server = Server::start(vec![response("renewed", None)]).await;
    let google = signing_in(
        &server,
        credentials.clone(),
        None,
        Arc::new(NoBrowser),
        SIGN_IN_WAIT,
    );
    let error = google.login(&prefs()).await.unwrap_err();
    assert_eq!(sign_in_error(&error), Some(SignInError::NotConfigured));
    assert_eq!(
        error.to_string(),
        "Google sign-in is not configured in this build."
    );
    let error = google
        .exchange_code(&prefs(), "code", "http://127.0.0.1:1/callback", "verifier")
        .await
        .unwrap_err();
    assert_eq!(sign_in_error(&error), Some(SignInError::NotConfigured));
    assert!(
        google
            .finish_pending_login(&prefs())
            .await
            .unwrap()
            .is_none()
    );
    assert!(server.requests().is_empty());
    // The stored self-configured client still renews its own grant.
    assert!(google.connected(&prefs()).await.unwrap());
    assert_eq!(
        google.token(&prefs()).await.unwrap().expose_secret(),
        "renewed"
    );
    assert_eq!(form(&server, 0)["client_id"], CLIENT);
    assert_eq!(form(&server, 0)["client_secret"], CLIENT_SECRET);
    server.finish().await;
}

#[tokio::test]
async fn a_self_configured_grant_keeps_refreshing_until_a_built_in_sign_in_replaces_it() {
    use crate::model::{GoogleAccess, GoogleGrant};
    let credentials = Arc::new(Credentials::default());
    // An upgraded install: the grant predates saved client secrets.
    credentials.seed(
        json!({"grants":[{"grant_id":"legacy-grant","client_id":"own-client",
        "access_token":"old-access","refresh_token":"own-refresh","expires_at":0,
        "scope":SCOPES,"requested_scopes":SCOPES}],"candidate_id":null}),
    );
    let mut legacy = Preferences {
        google_client_id: "own-client".into(),
        google_client_secret: "own-secret".into(),
        google_grant: GoogleGrant {
            id: "legacy-grant".into(),
            client_id: "own-client".into(),
            access: GoogleAccess {
                known: true,
                drive: true,
                calendar_read: true,
                calendar_write: true,
            },
        },
        ..new_user()
    };
    let mut server = Server::start(vec![
        response("own-renewed", None),
        response("built-in-access", Some("built-in-refresh")),
        response("built-in-renewed", None),
    ])
    .await;
    let (browser, mut opened) = fake_browser();
    let built_in = client::OAuthClient::new("built-in", "built-in-secret");
    let google = signing_in(
        &server,
        credentials.clone(),
        built_in.clone(),
        browser.clone(),
        SIGN_IN_WAIT,
    );
    assert!(google.connected(&legacy).await.unwrap());
    assert_eq!(
        google
            .token_for(&legacy, Service::Drive)
            .await
            .unwrap()
            .expose_secret(),
        "own-renewed"
    );
    assert_eq!(form(&server, 0)["client_id"], "own-client");
    assert_eq!(form(&server, 0)["client_secret"], "own-secret");
    assert_eq!(form(&server, 0)["refresh_token"], "own-refresh");
    // The refreshed vault entry now carries the secret that renewed it.
    assert_eq!(credentials.value()["client_secret"], "own-secret");
    let login = tokio::spawn({
        let google = google.clone();
        let legacy = legacy.clone();
        async move {
            google
                .login_with_retry(&legacy, false, &CancellationToken::new())
                .await
        }
    });
    let consent = opened.recv().await.unwrap();
    assert_eq!(query(&consent)["client_id"], "built-in");
    let state = query(&consent)["state"].clone();
    redirect(&consent, &format!("state={state}&code=built-in-code")).await;
    let grant = login.await.unwrap().unwrap();
    assert_eq!(grant.client_id, "built-in");
    assert_eq!(form(&server, 1)["client_id"], "built-in");
    // The self-configured grant stays usable until activation commits.
    assert_eq!(
        google.token(&legacy).await.unwrap().expose_secret(),
        "own-renewed"
    );
    legacy.google_grant = grant;
    let activated = legacy;
    google.finish_activation(&activated).await.unwrap();
    assert_eq!(credentials.value()["client_id"], "built-in");
    let mut value = credentials.value();
    value["expires_at"] = 0.into();
    credentials.seed(value);
    drop(google);
    let restarted = signing_in(
        &server,
        credentials.clone(),
        built_in,
        browser,
        SIGN_IN_WAIT,
    );
    assert!(restarted.connected(&activated).await.unwrap());
    assert_eq!(
        restarted.token(&activated).await.unwrap().expose_secret(),
        "built-in-renewed"
    );
    assert_eq!(form(&server, 2)["client_id"], "built-in");
    assert_eq!(form(&server, 2)["client_secret"], "built-in-secret");
    server.finish().await;
}

#[tokio::test]
async fn a_staged_candidate_from_another_client_is_not_resumed_after_the_client_changes() {
    let credentials = Arc::new(Credentials::default());
    credentials.seed(saved(chrono::Utc::now().timestamp() + 3600));
    let mut server = Server::start(vec![response("candidate", Some("candidate-refresh"))]).await;
    let override_client = signing_in(
        &server,
        credentials.clone(),
        client::OAuthClient::new("dev-override", "dev-secret"),
        Arc::new(NoBrowser),
        SIGN_IN_WAIT,
    );
    let candidate = override_client
        .exchange_code(&prefs(), "code", "http://127.0.0.1:1/callback", "verifier")
        .await
        .unwrap();
    assert_eq!(candidate.client_id, "dev-override");
    assert_eq!(
        override_client
            .finish_pending_login(&prefs())
            .await
            .unwrap(),
        Some(candidate)
    );
    drop(override_client);
    let built_in = google(&server, credentials.clone());
    assert!(
        built_in
            .finish_pending_login(&prefs())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        built_in.token(&prefs()).await.unwrap().expose_secret(),
        "old-access"
    );
    server.finish().await;
}
