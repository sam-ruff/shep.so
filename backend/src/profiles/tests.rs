use super::*;
use crate::tests::{body, config, login};
use crate::{AppState, app, google::Identity, tests::FakeGoogle};
use axum::body::Body;
use axum::http::{Request as HttpRequest, header};
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

/// Scripted Google: exchange/refresh answers are configured per test and the
/// Drive side is a small in-memory app-data store. Nothing leaves the process.
#[derive(Default)]
pub(crate) struct FixtureProvider {
    pub expected_code: std::sync::Mutex<String>,
    pub exchange_subject: std::sync::Mutex<Option<String>>,
    pub exchange_scope: std::sync::Mutex<Option<String>>,
    pub exchange_refresh: std::sync::Mutex<Option<String>>,
    pub expires_in: std::sync::Mutex<u64>,
    pub fail_exchange: std::sync::Mutex<bool>,
    pub fail_refresh: std::sync::Mutex<bool>,
    pub refresh_scope: std::sync::Mutex<Option<String>>,
    pub exchanges: AtomicUsize,
    pub refreshes: AtomicUsize,
    pub drive_calls: AtomicUsize,
    pub generated: AtomicUsize,
    pub tokens_seen: std::sync::Mutex<Vec<String>>,
    pub files: std::sync::Mutex<Vec<Value>>,
}
impl FixtureProvider {
    pub fn granting(subject: &str, scope: &str) -> Arc<Self> {
        Self::scripted("consent-code", subject, scope)
    }
    /// The production HTTPS gate exchanges the browser's fixture code for the
    /// signed-in synthetic owner's Drive-only grant.
    pub fn browser_gate() -> Arc<Self> {
        Self::scripted(
            "consent-fixture",
            "synthetic-owner-fixture",
            "openid email https://www.googleapis.com/auth/drive.appdata",
        )
    }
    fn scripted(code: &str, subject: &str, scope: &str) -> Arc<Self> {
        let provider = Self::default();
        *provider.expected_code.lock().unwrap() = code.into();
        *provider.exchange_subject.lock().unwrap() = Some(subject.into());
        *provider.exchange_scope.lock().unwrap() = Some(scope.into());
        *provider.exchange_refresh.lock().unwrap() = Some("fixture-refresh".into());
        *provider.expires_in.lock().unwrap() = 3600;
        Arc::new(provider)
    }
}
#[async_trait]
impl ProfileProvider for FixtureProvider {
    fn live(&self) -> bool {
        false
    }
    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<Tokens, ProviderFailed> {
        assert_eq!(verifier.len(), 43);
        assert_eq!(nonce.len(), 43);
        self.exchanges.fetch_add(1, Ordering::SeqCst);
        if *self.fail_exchange.lock().unwrap() || code != *self.expected_code.lock().unwrap() {
            return Err(ProviderFailed);
        }
        Ok(Tokens {
            access_token: Zeroizing::new(format!(
                "access-{}",
                self.exchanges.load(Ordering::SeqCst)
            )),
            refresh_token: self
                .exchange_refresh
                .lock()
                .unwrap()
                .clone()
                .map(Zeroizing::new),
            expires_in: *self.expires_in.lock().unwrap(),
            scope: self.exchange_scope.lock().unwrap().clone(),
            subject: self.exchange_subject.lock().unwrap().clone(),
        })
    }
    async fn refresh(&self, refresh_token: &str) -> Result<Tokens, ProviderFailed> {
        assert_eq!(refresh_token, "fixture-refresh");
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        if *self.fail_refresh.lock().unwrap() {
            return Err(ProviderFailed);
        }
        Ok(Tokens {
            access_token: Zeroizing::new("access-renewed".into()),
            refresh_token: None,
            expires_in: 3600,
            scope: self.refresh_scope.lock().unwrap().clone(),
            subject: None,
        })
    }
    async fn drive(
        &self,
        access_token: &str,
        request: DriveRequest,
    ) -> Result<Value, ProviderFailed> {
        self.drive_calls.fetch_add(1, Ordering::SeqCst);
        self.tokens_seen
            .lock()
            .unwrap()
            .push(access_token.to_owned());
        let files = self.files.lock().unwrap();
        Ok(match request {
            DriveRequest::About => {
                serde_json::json!({"user": {"permissionId": "fixture-permission", "emailAddress": "owner@example.test"}})
            }
            DriveRequest::List { .. } => {
                serde_json::json!({"incompleteSearch": false, "files": *files})
            }
            DriveRequest::StartPageToken => serde_json::json!({"startPageToken": "1"}),
            DriveRequest::Changes { .. } => {
                serde_json::json!({"newStartPageToken": "1", "changes": []})
            }
            DriveRequest::Metadata { file_id } => files
                .iter()
                .find(|f| f["id"] == file_id)
                .cloned()
                .unwrap_or(serde_json::json!({"missing": true})),
            DriveRequest::Media { file_id } => {
                serde_json::json!({"media": files.iter().find(|f| f["id"] == file_id).and_then(|f| f["media"].as_str()).unwrap_or("")})
            }
            DriveRequest::GenerateIds { count } => {
                let ids: Vec<String> = (0..count)
                    .map(|_| {
                        format!(
                            "generated-{}",
                            self.generated.fetch_add(1, Ordering::SeqCst)
                        )
                    })
                    .collect();
                serde_json::json!({ "ids": ids })
            }
            DriveRequest::Create { metadata, media } => {
                let mut file = metadata.clone();
                file["media"] = Value::String(media);
                file["size"] = Value::String(file["media"].as_str().unwrap().len().to_string());
                file["ownedByMe"] = Value::Bool(true);
                file["trashed"] = Value::Bool(false);
                file["spaces"] = serde_json::json!(["appDataFolder"]);
                drop(files);
                self.files.lock().unwrap().push(file.clone());
                file
            }
        })
    }
}

fn state_with(provider: Arc<FixtureProvider>, namespace: Option<&str>) -> AppState {
    let mut config = config();
    config.profile_namespace = namespace.map(str::to_owned);
    let verifier = Arc::new(FakeGoogle::identity(Identity {
        subject: "owner-subject".into(),
        email: "owner@example.test".into(),
    }));
    AppState::new(Arc::new(config), verifier, provider)
}
async fn get(state: &AppState, path: &str, cookie: &str) -> (StatusCode, Value) {
    let response = app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .uri(path)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let text = body(response).await;
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}
async fn post(
    state: &AppState,
    path: &str,
    cookie: &str,
    csrf: &str,
    json: Value,
) -> (StatusCode, Value) {
    let response = app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri(path)
                .header(header::COOKIE, cookie)
                .header(header::ORIGIN, "https://shep.example.test")
                .header("x-shep-csrf", csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let text = body(response).await;
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}
/// Start consent and return the pending state parameter from the Google URL.
async fn start(
    state: &AppState,
    cookie: &str,
    csrf: &str,
    drive: bool,
    calendar: &str,
) -> (String, url::Url) {
    let (status, value) = post(
        state,
        "/api/profiles/connect",
        cookie,
        csrf,
        serde_json::json!({"drive": drive, "calendar": calendar}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let url = url::Url::parse(value["url"].as_str().unwrap()).unwrap();
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    (params["state"].clone(), url)
}
async fn finish(state: &AppState, cookie: &str, query: &str) -> String {
    let response = app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .uri(format!("/auth/google/callback?{query}"))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    response.headers()[header::LOCATION]
        .to_str()
        .unwrap()
        .to_owned()
}

#[test]
fn requested_scopes_are_exact_and_granted_access_is_bound_to_the_request() {
    let both = Requested {
        drive: true,
        calendar: Calendar::Edit,
    };
    assert_eq!(
        both.scopes(),
        vec![
            "openid",
            "email",
            DRIVE_SCOPE,
            CALENDAR_WRITE_SCOPE,
            CALENDAR_LIST_SCOPE
        ]
    );
    let read = Requested {
        drive: false,
        calendar: Calendar::Read,
    };
    assert_eq!(
        read.scopes(),
        vec!["openid", "email", CALENDAR_READ_SCOPE, CALENDAR_LIST_SCOPE]
    );
    // Omitted scope inherits the request; broader grants are intersected.
    assert_eq!(
        Access::from_scope(None, both),
        Access {
            drive: true,
            calendar_read: true,
            calendar_write: true
        }
    );
    assert_eq!(
        Access::from_scope(
            Some(
                "openid email https://www.googleapis.com/auth/drive.appdata https://www.googleapis.com/auth/calendar"
            ),
            Requested {
                drive: false,
                calendar: Calendar::Read
            }
        ),
        Access {
            drive: false,
            calendar_read: true,
            calendar_write: false
        }
    );
    // A partial grant keeps the other service usable.
    assert_eq!(
        Access::from_scope(
            Some("openid email https://www.googleapis.com/auth/drive.appdata"),
            both
        ),
        Access {
            drive: true,
            calendar_read: false,
            calendar_write: false
        }
    );
}

#[tokio::test]
async fn consent_exchange_binds_scopes_and_principal_and_proxies_drive_without_exposing_tokens() {
    let provider = FixtureProvider::granting(
        "owner-subject",
        "openid email https://www.googleapis.com/auth/drive.appdata",
    );
    let state = state_with(provider.clone(), Some("so.shep.fixture"));
    let (cookie, csrf) = login(&state).await;
    let (_, before) = get(&state, "/api/profiles/connection", &cookie).await;
    assert_eq!(before["available"], true);
    assert_eq!(before["live"], false);
    assert_eq!(before["connected"], false);
    assert_eq!(before["namespace"], "so.shep.fixture");
    // Drive is refused before consent.
    let (status, value) = post(
        &state,
        "/api/profiles/drive",
        &cookie,
        &csrf,
        serde_json::json!({"op": "list"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{value}");
    let (id, url) = start(&state, &cookie, &csrf, true, "read").await;
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(
        params["scope"],
        "openid email https://www.googleapis.com/auth/drive.appdata https://www.googleapis.com/auth/calendar.events.readonly https://www.googleapis.com/auth/calendar.calendarlist.readonly"
    );
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["access_type"], "offline");
    assert_eq!(params["prompt"], "consent");
    assert_eq!(
        params["redirect_uri"],
        "https://shep.example.test/auth/google/callback"
    );
    let (_, pending) = get(&state, "/api/profiles/connection", &cookie).await;
    assert_eq!(pending["pending"]["drive"], true);
    assert_eq!(pending["pending"]["calendar"], "read");
    let location = finish(&state, &cookie, &format!("state={id}&code=consent-code")).await;
    assert_eq!(location, "/app/#profiles=connected");
    let (_, after) = get(&state, "/api/profiles/connection", &cookie).await;
    assert_eq!(after["connected"], true);
    assert_eq!(after["principal"], "drive:fixture-permission");
    assert_eq!(after["requested"]["calendar"], "read");
    // Google granted only Drive: Calendar stays off without hiding Drive.
    assert_eq!(after["granted"]["drive"], true);
    assert_eq!(after["granted"]["calendar_read"], false);
    assert!(after["pending"].is_null());
    assert!(!after.to_string().contains("access-"));
    assert!(!after.to_string().contains("fixture-refresh"));
    let (status, value) = post(
        &state,
        "/api/profiles/drive",
        &cookie,
        &csrf,
        serde_json::json!({"op": "list"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["incompleteSearch"], false);
    assert_eq!(
        provider.tokens_seen.lock().unwrap().last().unwrap(),
        "access-1"
    );
    // Invalid proxy requests never reach Google.
    let calls = provider.drive_calls.load(Ordering::SeqCst);
    for bad in [
        serde_json::json!({"op": "media", "file_id": "../etc"}),
        serde_json::json!({"op": "generate_ids", "count": 0}),
        serde_json::json!({"op": "create", "metadata": {"name": "x"}, "media": "{}"}),
        serde_json::json!({"op": "create", "metadata": {"id": "abc"}, "media": "x".repeat(MAX_MEDIA_BYTES + 1)}),
        serde_json::json!({"op": "changes", "page_token": ""}),
    ] {
        let (status, _) = post(&state, "/api/profiles/drive", &cookie, &csrf, bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    assert_eq!(provider.drive_calls.load(Ordering::SeqCst), calls);
    // Create round-trips through the fixture store and a later metadata read.
    let (status, created) = post(
        &state,
        "/api/profiles/drive",
        &cookie,
        &csrf,
        serde_json::json!({"op": "create", "metadata": {"id": "generated-0", "name": "shep-profile-x.json", "mimeType": "application/json", "parents": ["appDataFolder"], "appProperties": {"shepType": "profile"}}, "media": "{\"exact\":true}"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let (_, metadata) = post(
        &state,
        "/api/profiles/drive",
        &cookie,
        &csrf,
        serde_json::json!({"op": "metadata", "file_id": "generated-0"}),
    )
    .await;
    assert_eq!(metadata["size"], "14");
    let (_, media) = post(
        &state,
        "/api/profiles/drive",
        &cookie,
        &csrf,
        serde_json::json!({"op": "media", "file_id": "generated-0"}),
    )
    .await;
    assert_eq!(media["media"], "{\"exact\":true}");
}

#[tokio::test]
async fn denied_consent_and_failed_exchange_preserve_the_active_grant_and_choices() {
    let provider = FixtureProvider::granting(
        "owner-subject",
        "openid email https://www.googleapis.com/auth/drive.appdata",
    );
    let state = state_with(provider.clone(), Some("so.shep.fixture"));
    let (cookie, csrf) = login(&state).await;
    let (id, _) = start(&state, &cookie, &csrf, true, "off").await;
    finish(&state, &cookie, &format!("state={id}&code=consent-code")).await;
    // Changing the next-sign-in choice does not change saved access.
    let (id, _) = start(&state, &cookie, &csrf, true, "edit").await;
    let (_, during) = get(&state, "/api/profiles/connection", &cookie).await;
    assert_eq!(during["requested"]["calendar"], "edit");
    assert_eq!(during["granted"]["calendar_write"], false);
    assert_eq!(during["granted"]["drive"], true);
    let location = finish(&state, &cookie, &format!("state={id}&error=access_denied")).await;
    assert_eq!(location, "/app/#profiles=denied");
    let (_, after) = get(&state, "/api/profiles/connection", &cookie).await;
    assert_eq!(after["connected"], true);
    assert_eq!(after["granted"]["drive"], true);
    assert_eq!(after["granted"]["calendar_write"], false);
    assert_eq!(after["requested"]["calendar"], "edit");
    assert!(after["pending"].is_null());
    assert_eq!(provider.exchanges.load(Ordering::SeqCst), 1);
    // A replayed or foreign state cannot finish.
    let location = finish(&state, &cookie, &format!("state={id}&code=consent-code")).await;
    assert_eq!(location, "/app/#profiles=failed");
    // A failed exchange keeps the grant too.
    *provider.fail_exchange.lock().unwrap() = true;
    let (id, _) = start(&state, &cookie, &csrf, true, "edit").await;
    let location = finish(&state, &cookie, &format!("state={id}&code=consent-code")).await;
    assert_eq!(location, "/app/#profiles=failed");
    let (_, after) = get(&state, "/api/profiles/connection", &cookie).await;
    assert_eq!(after["connected"], true);
    assert_eq!(after["granted"]["drive"], true);
    // Consent from a different Google account is never bound to this session.
    *provider.fail_exchange.lock().unwrap() = false;
    *provider.exchange_subject.lock().unwrap() = Some("someone-else".into());
    let (id, _) = start(&state, &cookie, &csrf, true, "off").await;
    let location = finish(&state, &cookie, &format!("state={id}&code=consent-code")).await;
    assert_eq!(location, "/app/#profiles=mismatch");
    let (_, after) = get(&state, "/api/profiles/connection", &cookie).await;
    assert_eq!(after["connected"], true);
    // Drive still works with the original grant.
    let (status, _) = post(
        &state,
        "/api/profiles/drive",
        &cookie,
        &csrf,
        serde_json::json!({"op": "start_page_token"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn expired_access_is_refreshed_server_side_and_refresh_failure_is_actionable() {
    let provider = FixtureProvider::granting(
        "owner-subject",
        "openid email https://www.googleapis.com/auth/drive.appdata",
    );
    *provider.expires_in.lock().unwrap() = 10;
    let state = state_with(provider.clone(), Some("so.shep.fixture"));
    let (cookie, csrf) = login(&state).await;
    let (id, _) = start(&state, &cookie, &csrf, true, "off").await;
    finish(&state, &cookie, &format!("state={id}&code=consent-code")).await;
    let (status, _) = post(
        &state,
        "/api/profiles/drive",
        &cookie,
        &csrf,
        serde_json::json!({"op": "list"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(provider.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(
        provider.tokens_seen.lock().unwrap().last().unwrap(),
        "access-renewed"
    );
    // A narrowed refresh scope turns Drive off with a reconnect instruction.
    let mut state2 = state_with(provider.clone(), Some("so.shep.fixture"));
    *provider.refresh_scope.lock().unwrap() = Some("openid email".into());
    let (cookie2, csrf2) = login(&state2).await;
    let (id, _) = start(&state2, &cookie2, &csrf2, true, "off").await;
    finish(&state2, &cookie2, &format!("state={id}&code=consent-code")).await;
    let (status, value) = post(
        &state2,
        "/api/profiles/drive",
        &cookie2,
        &csrf2,
        serde_json::json!({"op": "list"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(value["error"].as_str().unwrap().contains("Reconnect"));
    let (_, info) = get(&state2, "/api/profiles/connection", &cookie2).await;
    assert_eq!(info["granted"]["drive"], false);
    // Refresh failure is reported without dropping the connection.
    *provider.refresh_scope.lock().unwrap() = None;
    *provider.fail_refresh.lock().unwrap() = true;
    state2 = state_with(provider.clone(), Some("so.shep.fixture"));
    let (cookie3, csrf3) = login(&state2).await;
    let (id, _) = start(&state2, &cookie3, &csrf3, true, "off").await;
    finish(&state2, &cookie3, &format!("state={id}&code=consent-code")).await;
    let (status, value) = post(
        &state2,
        "/api/profiles/drive",
        &cookie3,
        &csrf3,
        serde_json::json!({"op": "list"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(value["error"].as_str().unwrap().contains("Retry"));
    let (_, info) = get(&state2, "/api/profiles/connection", &cookie3).await;
    assert_eq!(info["connected"], true);
}

#[tokio::test]
async fn disconnect_logout_and_new_login_remove_grants_and_unconfigured_namespace_is_explicit() {
    let provider = FixtureProvider::granting(
        "owner-subject",
        "openid email https://www.googleapis.com/auth/drive.appdata",
    );
    let state = state_with(provider.clone(), Some("so.shep.fixture"));
    let (cookie, csrf) = login(&state).await;
    let (id, _) = start(&state, &cookie, &csrf, true, "off").await;
    finish(&state, &cookie, &format!("state={id}&code=consent-code")).await;
    let (status, _) = post(
        &state,
        "/api/profiles/disconnect",
        &cookie,
        &csrf,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, info) = get(&state, "/api/profiles/connection", &cookie).await;
    assert_eq!(info["connected"], false);
    assert!(state.profiles.grants.lock().await.is_empty());
    // Logout clears the grant with the session.
    let (id, _) = start(&state, &cookie, &csrf, true, "off").await;
    finish(&state, &cookie, &format!("state={id}&code=consent-code")).await;
    assert_eq!(state.profiles.grants.lock().await.len(), 1);
    let (status, _) = post(&state, "/api/logout", &cookie, &csrf, Value::Null).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(state.profiles.grants.lock().await.is_empty());
    assert!(state.profiles.pending.lock().await.is_empty());
    // A fresh login replacing an old session drops that session's grant.
    let (cookie, csrf) = login(&state).await;
    let (id, _) = start(&state, &cookie, &csrf, true, "off").await;
    finish(&state, &cookie, &format!("state={id}&code=consent-code")).await;
    assert_eq!(state.profiles.grants.lock().await.len(), 1);
    let (cookie2, _) = login(&state).await;
    assert_ne!(cookie, cookie2);
    let (_, info) = get(&state, "/api/profiles/connection", &cookie2).await;
    assert_eq!(info["connected"], false);
    // The CSRF header is required for state changes.
    let response = app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/api/profiles/connect")
                .header(header::COOKIE, &cookie2)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{\"drive\":true,\"calendar\":\"off\"}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    // Without a namespace the server says so and refuses to start consent.
    let state = state_with(provider, None);
    let (cookie, csrf) = login(&state).await;
    let (_, info) = get(&state, "/api/profiles/connection", &cookie).await;
    assert_eq!(info["available"], false);
    assert!(info["reason"].as_str().unwrap().contains("not configured"));
    let (status, _) = post(
        &state,
        "/api/profiles/connect",
        &cookie,
        &csrf,
        serde_json::json!({"drive": true, "calendar": "off"}),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let (status, _) = post(
        &state,
        "/api/profiles/connect",
        &cookie,
        &csrf,
        serde_json::json!({"drive": false, "calendar": "off"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
