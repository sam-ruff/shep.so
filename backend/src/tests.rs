use super::*;
use crate::google::VerificationFailed;
use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request as HttpRequest;
use http_body_util::BodyExt;
use jsonwebtoken::DecodingKey;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use std::{
    collections::HashSet,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use tower::ServiceExt;

pub(crate) struct FakeGoogle {
    identity: Option<Identity>,
    calls: AtomicUsize,
}
#[async_trait]
impl LoginVerifier for FakeGoogle {
    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<Identity, VerificationFailed> {
        assert_eq!(code, "fixture-code");
        assert_eq!(verifier.len(), 43);
        assert_eq!(nonce.len(), 43);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.identity.clone().ok_or(VerificationFailed)
    }
}
pub(crate) fn config() -> Config {
    Config {
        origin: "https://shep.example.test".into(),
        google_client_id: "fixture-client".into(),
        google_client_secret: Zeroizing::new("fixture-secret".into()),
        allowed_emails: HashSet::from(["owner@example.test".into()]),
        allowed_subjects: HashSet::new(),
        web_dir: "../web/dist".into(),
        bind: "127.0.0.1:3080".parse().unwrap(),
        mail_endpoints: Vec::new(),
    }
}
pub(crate) fn state(email: &str) -> (AppState, Arc<FakeGoogle>) {
    let verifier = Arc::new(FakeGoogle {
        identity: Some(Identity {
            subject: "owner-subject".into(),
            email: email.into(),
        }),
        calls: AtomicUsize::new(0),
    });
    (
        AppState::new(Arc::new(config()), verifier.clone()),
        verifier,
    )
}
pub(crate) async fn body(response: Response) -> String {
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}
async fn begin(state: &AppState) -> (String, String) {
    let response = app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .uri("/auth/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = url::Url::parse(response.headers()[header::LOCATION].to_str().unwrap()).unwrap();
    let params = location.query_pairs().collect::<HashMap<_, _>>();
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(params["scope"], "openid email");
    assert!(!params.contains_key("access_type"));
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("HttpOnly; Secure; SameSite=Lax")
    );
    (
        params["state"].to_string(),
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .into(),
    )
}
async fn finish(state: &AppState, id: &str, cookie: &str) -> Response {
    app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .uri(format!("/auth/callback?state={id}&code=fixture-code"))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}
pub(crate) async fn login(state: &AppState) -> (String, String) {
    let (id, cookie) = begin(state).await;
    let response = finish(state, &id, &cookie).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let cookie = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .find_map(|h| {
            let h = h.to_str().unwrap();
            h.starts_with(SESSION_COOKIE)
                .then(|| h.split(';').next().unwrap().to_owned())
        })
        .unwrap();
    let response = app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .uri("/api/session")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let data: serde_json::Value = serde_json::from_str(&body(response).await).unwrap();
    (cookie, data["csrf"].as_str().unwrap().to_owned())
}
#[tokio::test]
async fn anonymous_app_assets_and_apis_are_gated() {
    let (state, _) = state("owner@example.test");
    for path in ["/app/", "/app/assets/index.js", "/app"] {
        let r = app(state.clone())
            .oneshot(
                HttpRequest::builder()
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::SEE_OTHER);
        assert_eq!(r.headers()[header::LOCATION], "/beta");
    }
    for path in ["/api/session", "/api/capabilities"] {
        let r = app(state.clone())
            .oneshot(
                HttpRequest::builder()
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    }
}
#[tokio::test]
async fn unconfigured_allowlist_rejects_everyone() {
    let (mut c, verifier) = (config(), state("owner@example.test").1);
    c.allowed_emails.clear();
    let state = AppState::new(Arc::new(c), verifier);
    let r = app(state)
        .oneshot(
            HttpRequest::builder()
                .uri("/auth/start")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::SERVICE_UNAVAILABLE);
}
#[tokio::test]
async fn owner_login_issues_session_and_denies_other_accounts() {
    let (state, verifier) = state("owner@example.test");
    let (cookie, _) = login(&state).await;
    assert!(cookie.starts_with(SESSION_COOKIE));
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
    let (other, verifier) = self::state("not-owner@example.test");
    let (id, cookie) = begin(&other).await;
    let r = finish(&other, &id, &cookie).await;
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert!(!r.headers().contains_key(header::SET_COOKIE));
    assert!(other.sessions.lock().await.is_empty());
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn callback_state_cookie_expiry_and_replay_are_checked_before_exchange() {
    let (state, verifier) = state("owner@example.test");
    let (id, cookie) = begin(&state).await;
    assert_eq!(
        finish(
            &state,
            &id,
            "__Host-shep_login=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        finish(&state, "wrong-state", &cookie).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        finish(&state, &id, &cookie).await.status(),
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        finish(&state, &id, &cookie).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
    let (id, cookie) = begin(&state).await;
    state.pending.lock().await.get_mut(&id).unwrap().created =
        Instant::now() - Duration::from_secs(LOGIN_SECONDS + 1);
    assert_eq!(
        finish(&state, &id, &cookie).await.status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(verifier.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn expired_and_duplicate_session_cookies_are_rejected() {
    let (state, _) = state("owner@example.test");
    let (cookie, _) = login(&state).await;
    let r = app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .uri("/api/session")
                .header(header::COOKIE, format!("{cookie}; {cookie}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    for session in state.sessions.lock().await.values_mut() {
        session.created = Instant::now() - Duration::from_secs(SESSION_SECONDS + 1);
    }
    let r = app(state)
        .oneshot(
            HttpRequest::builder()
                .uri("/api/session")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}
#[tokio::test]
async fn writes_require_same_origin_and_csrf_and_logout_revokes() {
    let (state, _) = state("owner@example.test");
    let (cookie, csrf) = login(&state).await;
    for (origin, token) in [
        ("https://evil.example.test", csrf.as_str()),
        ("https://shep.example.test", "wrong"),
    ] {
        let r = app(state.clone())
            .oneshot(
                HttpRequest::builder()
                    .method("POST")
                    .uri("/api/logout")
                    .header(header::COOKIE, &cookie)
                    .header(header::ORIGIN, origin)
                    .header("x-shep-csrf", token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }
    let r = app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/api/logout")
                .header(header::COOKIE, &cookie)
                .header(header::ORIGIN, "https://shep.example.test")
                .header("x-shep-csrf", csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NO_CONTENT);
    assert!(
        r.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let r = app(state)
        .oneshot(
            HttpRequest::builder()
                .uri("/api/session")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}
#[tokio::test]
async fn headers_forbid_caching_and_framing_without_leaking_callback() {
    let (state, _) = state("owner@example.test");
    let r = app(state)
        .oneshot(
            HttpRequest::builder()
                .uri("/beta")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
    assert!(
        r.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    assert_eq!(r.headers()["referrer-policy"], "no-referrer");
    let csp = r.headers()["content-security-policy"].to_str().unwrap();
    assert!(csp.contains(&shep_mail_core::document::runtime_csp_source()));
    assert!(csp.contains(&shep_mail_core::printing::runtime_csp_source()));
    assert!(
        !csp.split("script-src")
            .nth(1)
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .contains("'unsafe-inline'")
    );
}
#[test]
fn public_origin_and_bind_are_strict() {
    let mut c = config();
    assert!(c.validate().is_ok());
    c.origin = "http://shep.example.test".into();
    assert!(c.validate().is_err());
    c.origin = "https://shep.example.test/auth".into();
    assert!(c.validate().is_err());
    c.origin = "https://shep.example.test".into();
    c.bind = "0.0.0.0:3080".parse().unwrap();
    assert!(c.validate().is_err());
}
#[test]
fn allowlist_is_exact_and_optional_subject_pin_is_enforced() {
    let mut c = config();
    assert!(c.permits("OWNER@example.test", "owner-subject"));
    assert!(!c.permits("owner@example.test.attacker.test", "owner-subject"));
    c.allowed_subjects.insert("owner-subject".into());
    assert!(!c.permits("owner@example.test", "other-subject"));
}
#[test]
fn signed_google_tokens_require_correct_issuer_audience_nonce_email_and_expiry() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = serde_json::json!({"sub":"fixture-subject","email":"owner@example.test","email_verified":true,"nonce":"fixture-nonce","aud":"fixture-client","iss":"https://accounts.google.com","exp":now+3600});
    let key = EncodingKey::from_rsa_pem(include_bytes!("../tests/fixtures/oidc-test-private.pem"))
        .unwrap();
    let public =
        DecodingKey::from_rsa_pem(include_bytes!("../tests/fixtures/oidc-test-public.pem"))
            .unwrap();
    let sign = |c: &serde_json::Value| {
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), c, &key).unwrap()
    };
    assert!(
        google::verify_token(&sign(&claims), &public, "fixture-client", "fixture-nonce").is_ok()
    );
    for (field, value) in [
        ("iss", serde_json::json!("https://attacker.test")),
        ("aud", serde_json::json!("other-client")),
        ("nonce", serde_json::json!("other-nonce")),
        ("email_verified", serde_json::json!(false)),
        ("exp", serde_json::json!(now - 3600)),
        ("azp", serde_json::json!("another-presenter")),
    ] {
        let mut invalid = claims.clone();
        invalid[field] = value;
        assert!(
            google::verify_token(&sign(&invalid), &public, "fixture-client", "fixture-nonce")
                .is_err(),
            "accepted invalid {field}"
        );
    }
    let symmetric = jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(b"fixture-only"),
    )
    .unwrap();
    assert!(google::verify_token(&symmetric, &public, "fixture-client", "fixture-nonce").is_err());
}

#[tokio::test]
async fn permitted_session_can_read_web_assets_and_mail_reports_unavailable() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("index.html"), "protected fixture app").unwrap();
    std::fs::write(directory.path().join("fixture.js"), "fixture-script").unwrap();
    let (_, verifier) = state("owner@example.test");
    let mut config = config();
    config.web_dir = directory.path().to_owned();
    let state = AppState::new(Arc::new(config), verifier);
    let (cookie, csrf) = login(&state).await;
    for (path, expected) in [
        ("/app/", "protected fixture app"),
        ("/app/fixture.js", "fixture-script"),
    ] {
        let response = app(state.clone())
            .oneshot(
                HttpRequest::builder()
                    .uri(path)
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body(response).await, expected);
    }
    let response = app(state)
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri("/api/mail/send")
                .header(header::COOKIE, cookie)
                .header(header::ORIGIN, "https://shep.example.test")
                .header("x-shep-csrf", csrf)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    assert!(
        body(response)
            .await
            .contains("No mail operation was performed")
    );
}

#[tokio::test]
async fn browser_profile_identity_survives_login_rotation_but_is_scoped_to_google_subject_and_client()
 {
    let (state, _) = state("owner@example.test");
    let mut profiles = Vec::new();
    for _ in 0..2 {
        let (cookie, _) = login(&state).await;
        let response = app(state.clone())
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/session")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&body(response).await).unwrap();
        profiles.push(value["user_id"].as_str().unwrap().to_owned());
    }
    assert_eq!(profiles[0], profiles[1]);
    assert_eq!(profiles[0].len(), 43);
    assert_ne!(profiles[0], hash("fixture-client\0different-subject"));
    assert_ne!(profiles[0], hash("different-client\0owner-subject"));
    assert_eq!(profiles[0], hash("fixture-client\0owner-subject"));
}
