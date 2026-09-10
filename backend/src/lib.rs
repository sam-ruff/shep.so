pub mod config;
pub mod google;
pub mod mail;
pub mod profiles;

use axum::{
    Json, Router,
    extract::{Extension, Query, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use config::Config;
use google::{Identity, LoginVerifier};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use tower_http::services::ServeDir;
use zeroize::Zeroizing;

const SESSION_SECONDS: u64 = 8 * 3600;
const LOGIN_SECONDS: u64 = 600;
pub(crate) const SESSION_COOKIE: &str = "__Host-shep_session";
const LOGIN_COOKIE: &str = "__Host-shep_login";
struct Pending {
    created: Instant,
    cookie: String,
    verifier: Zeroizing<String>,
    nonce: String,
}
#[derive(Clone)]
pub(crate) struct Session {
    created: Instant,
    pub(crate) identity: Identity,
    csrf: String,
}
#[derive(Clone)]
pub struct AppState {
    config: Arc<Config>,
    verifier: Arc<dyn LoginVerifier>,
    provider: Arc<dyn profiles::ProfileProvider>,
    pending: Arc<Mutex<HashMap<String, Pending>>>,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    profiles: Arc<profiles::ProfileState>,
    auth_slots: Arc<Semaphore>,
    mail: Arc<mail::MailHub>,
}
impl AppState {
    pub fn new(
        config: Arc<Config>,
        verifier: Arc<dyn LoginVerifier>,
        provider: Arc<dyn profiles::ProfileProvider>,
    ) -> Self {
        Self {
            mail: Arc::new(mail::MailHub::new(config.mail_endpoints.clone())),
            config,
            verifier,
            provider,
            pending: Default::default(),
            sessions: Default::default(),
            profiles: Default::default(),
            auth_slots: Arc::new(Semaphore::new(8)),
        }
    }
    async fn session(&self, headers: &HeaderMap) -> Option<Session> {
        let token = cookie(headers, SESSION_COOKIE)?;
        let mut sessions = self.sessions.lock().await;
        sessions.retain(|_, s| {
            s.created.elapsed() < Duration::from_secs(SESSION_SECONDS)
                && self.config.permits(&s.identity.email, &s.identity.subject)
        });
        // Provider grants live exactly as long as their session.
        self.profiles
            .retain_sessions(|key| sessions.contains_key(key))
            .await;
        sessions.get(&hash(&token)).cloned()
    }
}
pub(crate) fn token() -> String {
    let mut value = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut value);
    URL_SAFE_NO_PAD.encode(value)
}
pub(crate) fn hash(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}
pub(crate) fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        for part in value.to_str().ok()?.split(';') {
            if let Some((key, value)) = part.trim().split_once('=')
                && key == name
            {
                if found.is_some()
                    || value.len() != 43
                    || !value
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                {
                    return None;
                }
                found = Some(value.to_owned());
            }
        }
    }
    found
}
fn set_cookie(response: &mut Response, name: &str, value: &str, age: u64) {
    let value = format!("{name}={value}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={age}");
    response.headers_mut().append(
        header::SET_COOKIE,
        HeaderValue::from_str(&value).expect("generated cookie"),
    );
}
pub fn app(state: AppState) -> Router {
    let web = ServeDir::new(&state.config.web_dir).append_index_html_on_directories(true);
    let private = Router::new()
        .route("/api/session", get(session_info))
        .route("/api/logout", post(logout))
        .route("/api/capabilities", get(capabilities))
        .merge(mail::routes(state.clone()))
        .merge(mail::receipt_routes())
        .merge(profiles::routes())
        .route("/app", get(|| async { Redirect::to("/app/") }))
        .nest_service("/app/", web)
        .layer(middleware::from_fn_with_state(state.clone(), authorize));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/beta", get(beta))
        .route("/auth/start", get(start))
        .route("/auth/callback", get(callback))
        .merge(profiles::callback_routes())
        .merge(private)
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}
async fn beta() -> Html<&'static str> {
    Html(include_str!("beta.html"))
}
async fn start(State(state): State<AppState>, headers: HeaderMap) -> Response {
    // Public login starts are bounded. No allowlist means nobody may sign in.
    if state.config.allowed_emails.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Beta access is not configured yet.",
        )
            .into_response();
    }
    if headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "cross-site")
    {
        return (
            StatusCode::FORBIDDEN,
            "Start sign-in from the Shep beta page.",
        )
            .into_response();
    }
    let mut pending = state.pending.lock().await;
    pending.retain(|_, p| p.created.elapsed() < Duration::from_secs(LOGIN_SECONDS));
    if let Some(old) = cookie(&headers, LOGIN_COOKIE) {
        pending.retain(|_, p| p.cookie != hash(&old));
    }
    if pending.len() >= 256 {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Sign-in is busy. Try again shortly.",
        )
            .into_response();
    }
    let nonce = token();
    let verifier = Zeroizing::new(token());
    let id = token();
    let browser = token();
    let mut url =
        url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").expect("constant URL");
    url.query_pairs_mut().extend_pairs([
        ("client_id", state.config.google_client_id.as_str()),
        ("redirect_uri", state.config.callback().as_str()),
        ("response_type", "code"),
        ("scope", "openid email"),
        ("state", id.as_str()),
        ("nonce", nonce.as_str()),
        ("code_challenge", hash(&verifier).as_str()),
        ("code_challenge_method", "S256"),
        ("prompt", "select_account"),
    ]);
    pending.insert(
        id,
        Pending {
            created: Instant::now(),
            cookie: hash(&browser),
            verifier,
            nonce,
        },
    );
    let mut response = Redirect::to(url.as_str()).into_response();
    set_cookie(&mut response, LOGIN_COOKIE, &browser, LOGIN_SECONDS);
    response
}
#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}
async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> Response {
    let denied = || {
        (
            StatusCode::FORBIDDEN,
            "Sign-in was not accepted. Return to /beta to try an allowed account.",
        )
            .into_response()
    };
    let Some(browser) = cookie(&headers, LOGIN_COOKIE) else {
        return denied();
    };
    let Some(id) = query.state else {
        return denied();
    };
    let pending = {
        let mut entries = state.pending.lock().await;
        if !entries.get(&id).is_some_and(|p| {
            p.cookie == hash(&browser) && p.created.elapsed() < Duration::from_secs(LOGIN_SECONDS)
        }) {
            return denied();
        }
        entries.remove(&id).expect("matched pending login")
    };
    if query.error.is_some() {
        return denied();
    }
    let Some(code) = query.code.filter(|c| !c.is_empty() && c.len() <= 4096) else {
        return denied();
    };
    let Ok(_permit) = state.auth_slots.clone().try_acquire_owned() else {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            "Sign-in is busy. Start again from /beta.",
        )
            .into_response();
    };
    let Ok(identity) = state
        .verifier
        .exchange(&code, &pending.verifier, &pending.nonce)
        .await
    else {
        return denied();
    };
    if !state.config.permits(&identity.email, &identity.subject) {
        return denied();
    }
    let mut sessions = state.sessions.lock().await;
    sessions.retain(|_, s| s.created.elapsed() < Duration::from_secs(SESSION_SECONDS));
    if let Some(old) = cookie(&headers, SESSION_COOKIE) {
        sessions.remove(&hash(&old));
        state.profiles.forget(&hash(&old)).await;
    }
    if sessions.len() >= 1024 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "Sign-in is busy. Try again later.",
        )
            .into_response();
    }
    let session = token();
    sessions.insert(
        hash(&session),
        Session {
            created: Instant::now(),
            identity,
            csrf: token(),
        },
    );
    let mut response = Redirect::to("/app/").into_response();
    set_cookie(&mut response, SESSION_COOKIE, &session, SESSION_SECONDS);
    set_cookie(&mut response, LOGIN_COOKIE, "", 0);
    response
}
async fn authorize(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let Some(session) = state.session(request.headers()).await else {
        return if request.uri().path().starts_with("/app") {
            Redirect::to("/beta").into_response()
        } else {
            (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error":"Sign in to the Shep beta."})),
            )
                .into_response()
        };
    };
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        let origin = request
            .headers()
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok());
        let csrf = request
            .headers()
            .get("x-shep-csrf")
            .and_then(|v| v.to_str().ok());
        use subtle::ConstantTimeEq;
        if origin != Some(state.config.origin.as_str())
            || !csrf.is_some_and(|c| bool::from(c.as_bytes().ct_eq(session.csrf.as_bytes())))
        {
            return (
                StatusCode::FORBIDDEN,
                "Refresh the page before trying this action again.",
            )
                .into_response();
        }
    }
    request.extensions_mut().insert(session);
    next.run(request).await
}
#[derive(Serialize)]
struct SessionInfo {
    email: String,
    csrf: String,
    user_id: String,
}
async fn session_info(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Json<SessionInfo> {
    Json(SessionInfo {
        user_id: hash(&format!(
            "{}\0{}",
            state.config.google_client_id, session.identity.subject
        )),
        email: session.identity.email,
        csrf: session.csrf,
    })
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie(&headers, SESSION_COOKIE) {
        state.sessions.lock().await.remove(&hash(&token));
        state.profiles.forget(&hash(&token)).await;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    set_cookie(&mut response, SESSION_COOKIE, "", 0);
    response
}
async fn capabilities(State(state): State<AppState>) -> Json<serde_json::Value> {
    let endpoints: Vec<_> = state
        .config
        .mail_endpoints
        .iter()
        .map(|e| serde_json::json!({"host":e.host,"port":e.port,"service":e.service}))
        .collect();
    Json(
        serde_json::json!({"beta":true,"mail":!endpoints.is_empty(),"endpoints":endpoints,"calendar":false,"backups":false,"sent_copy":state.config.mail_endpoints.iter().any(|e| e.service == mail::policy::Service::Imap)}),
    )
}
async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    static CSP: std::sync::LazyLock<HeaderValue> = std::sync::LazyLock::new(|| {
        let runtime = shep_mail_core::document::runtime_csp_source();
        let print_runtime = shep_mail_core::printing::runtime_csp_source();
        HeaderValue::from_str(&format!("default-src 'self'; script-src 'self' 'wasm-unsafe-eval' {runtime} {print_runtime}; worker-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; frame-src 'self'; frame-ancestors 'none'; form-action 'self'; base-uri 'none'")).expect("Static reader CSP")
    });
    headers.insert("content-security-policy", CSP.clone());
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    response
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod browser_tests;
