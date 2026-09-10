//! Browser Google provider consent and the app-data Drive proxy.
//!
//! The beta identity gate never grants Drive or Calendar access. This module
//! runs a second, explicit OAuth consent bound to the signed-in session, keeps
//! the resulting tokens only in the in-memory session store, refreshes them
//! server-side and proxies a fixed set of Drive app-data operations. The
//! browser never sees an access or refresh token, and nothing here is logged.
use crate::{AppState, Session, hash, token};
use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Extension, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

const PENDING_SECONDS: u64 = 600;
const REFRESH_MARGIN: Duration = Duration::from_secs(60);
pub const MAX_MEDIA_BYTES: usize = 1024 * 1024;
pub const MAX_JSON_BYTES: usize = 256 * 1024;
const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.appdata";
const CALENDAR_LIST_SCOPE: &str = "https://www.googleapis.com/auth/calendar.calendarlist.readonly";
const CALENDAR_READ_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events.readonly";
const CALENDAR_WRITE_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events";

#[derive(Debug, Clone, Copy)]
pub struct ProviderFailed;
impl std::fmt::Display for ProviderFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Google provider request failed")
    }
}
impl std::error::Error for ProviderFailed {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Calendar {
    #[default]
    Off,
    Read,
    Edit,
}
/// The next-sign-in choices, kept separate from the active grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Requested {
    pub drive: bool,
    pub calendar: Calendar,
}
impl Requested {
    pub fn any(self) -> bool {
        self.drive || self.calendar != Calendar::Off
    }
    /// Exact scope list for this choice, mirroring desktop `consent.rs`.
    pub fn scopes(self) -> Vec<&'static str> {
        let mut scopes = vec!["openid", "email"];
        if self.drive {
            scopes.push(DRIVE_SCOPE);
        }
        match self.calendar {
            Calendar::Off => {}
            Calendar::Read => {
                scopes.push(CALENDAR_READ_SCOPE);
                scopes.push(CALENDAR_LIST_SCOPE);
            }
            Calendar::Edit => {
                scopes.push(CALENDAR_WRITE_SCOPE);
                scopes.push(CALENDAR_LIST_SCOPE);
            }
        }
        scopes
    }
}
/// Interpretation of the actual granted scopes, restricted to the request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Access {
    pub drive: bool,
    pub calendar_read: bool,
    pub calendar_write: bool,
}
impl Access {
    /// An omitted fresh-response scope inherits the requested set (RFC 6749 5.1);
    /// a broader grant is intersected with what was asked for.
    pub fn from_scope(scope: Option<&str>, requested: Requested) -> Self {
        let granted: Vec<&str> = match scope {
            Some(scope) => scope.split_ascii_whitespace().collect(),
            None => requested.scopes(),
        };
        let has = |name: &str| granted.contains(&name);
        let list = has(CALENDAR_LIST_SCOPE) || has("https://www.googleapis.com/auth/calendar");
        let write = has(CALENDAR_WRITE_SCOPE) || has("https://www.googleapis.com/auth/calendar");
        let read = write
            || has(CALENDAR_READ_SCOPE)
            || has("https://www.googleapis.com/auth/calendar.readonly");
        Self {
            drive: requested.drive && has(DRIVE_SCOPE),
            calendar_read: requested.calendar != Calendar::Off && list && read,
            calendar_write: requested.calendar == Calendar::Edit && list && write,
        }
    }
}

/// Fresh token material from a code exchange or refresh.
pub struct Tokens {
    pub access_token: Zeroizing<String>,
    pub refresh_token: Option<Zeroizing<String>>,
    pub expires_in: u64,
    pub scope: Option<String>,
    /// Verified OpenID subject of the consenting account, when an ID token
    /// was returned. A refresh response omits it.
    pub subject: Option<String>,
}

/// One proxied Drive app-data operation. Every variant is bounded here and
/// mapped to a fixed Google endpoint by the provider.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum DriveRequest {
    About,
    List {
        #[serde(default)]
        page_token: Option<String>,
    },
    StartPageToken,
    Changes {
        page_token: String,
    },
    Metadata {
        file_id: String,
    },
    Media {
        file_id: String,
    },
    GenerateIds {
        count: u8,
    },
    Create {
        metadata: Value,
        media: String,
    },
}
impl DriveRequest {
    fn validate(&self) -> Result<(), &'static str> {
        let token_ok = |t: &str| !t.is_empty() && t.len() <= 1024 && t.is_ascii();
        let id_ok = |id: &str| {
            !id.is_empty()
                && id.len() <= 255
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        };
        match self {
            Self::About | Self::StartPageToken => Ok(()),
            Self::List { page_token } => page_token
                .as_deref()
                .is_none_or(token_ok)
                .then_some(())
                .ok_or("Invalid page token."),
            Self::Changes { page_token } => token_ok(page_token)
                .then_some(())
                .ok_or("Invalid page token."),
            Self::Metadata { file_id } | Self::Media { file_id } => {
                id_ok(file_id).then_some(()).ok_or("Invalid file identity.")
            }
            Self::GenerateIds { count } => (1..=10)
                .contains(count)
                .then_some(())
                .ok_or("Invalid count."),
            Self::Create { metadata, media } => {
                if media.len() > MAX_MEDIA_BYTES {
                    return Err("The profile record exceeds the supported size.");
                }
                let object = metadata.as_object().ok_or("Invalid file metadata.")?;
                if serde_json::to_vec(metadata)
                    .map(|v| v.len())
                    .unwrap_or(usize::MAX)
                    > 4096
                    || !object.get("id").and_then(Value::as_str).is_some_and(id_ok)
                {
                    return Err("Invalid file metadata.");
                }
                Ok(())
            }
        }
    }
    fn needs_drive(&self) -> bool {
        !matches!(self, Self::About)
    }
}

#[async_trait]
pub trait ProfileProvider: Send + Sync {
    /// Whether a real Google project is configured; fixtures report false so the
    /// browser can say that live provider access is not connected.
    fn live(&self) -> bool;
    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<Tokens, ProviderFailed>;
    async fn refresh(&self, refresh_token: &str) -> Result<Tokens, ProviderFailed>;
    async fn drive(
        &self,
        access_token: &str,
        request: DriveRequest,
    ) -> Result<Value, ProviderFailed>;
}

struct Grant {
    access_token: Zeroizing<String>,
    refresh_token: Option<Zeroizing<String>>,
    expires: Instant,
    requested: Requested,
    access: Access,
    principal: Option<String>,
}
struct Pending {
    created: Instant,
    session: String,
    verifier: Zeroizing<String>,
    nonce: String,
    requested: Requested,
}
#[derive(Default)]
pub struct ProfileState {
    grants: Mutex<HashMap<String, Grant>>,
    pending: Mutex<HashMap<String, Pending>>,
    /// Remembered next-sign-in choices per session, independent of the grant.
    choices: Mutex<HashMap<String, Requested>>,
}
impl ProfileState {
    /// Drop grant material for sessions that no longer exist.
    pub async fn retain_sessions(&self, alive: impl Fn(&str) -> bool) {
        self.grants.lock().await.retain(|s, _| alive(s));
        self.pending.lock().await.retain(|_, p| alive(&p.session));
        self.choices.lock().await.retain(|s, _| alive(s));
    }
    pub async fn forget(&self, session: &str) {
        self.grants.lock().await.remove(session);
        self.pending
            .lock()
            .await
            .retain(|_, p| p.session != session);
        self.choices.lock().await.remove(session);
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/profiles/connection", get(connection))
        .route("/api/profiles/connect", post(connect))
        .route("/api/profiles/disconnect", post(disconnect))
        .route(
            "/api/profiles/drive",
            // One operation record plus its metadata envelope; media itself is
            // bounded again by the request validator.
            post(drive).layer(axum::extract::DefaultBodyLimit::max(
                MAX_MEDIA_BYTES + 64 * 1024,
            )),
        )
}
pub fn callback_routes() -> Router<AppState> {
    Router::new().route("/auth/google/callback", get(callback))
}

#[derive(Serialize)]
struct Connection {
    available: bool,
    live: bool,
    namespace: Option<String>,
    reason: Option<&'static str>,
    connected: bool,
    email: String,
    principal: Option<String>,
    requested: Requested,
    granted: Access,
    pending: Option<Requested>,
}
fn session_key(headers: &HeaderMap) -> Option<String> {
    crate::cookie(headers, crate::SESSION_COOKIE).map(|t| hash(&t))
}
async fn connection(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    headers: HeaderMap,
) -> Response {
    let Some(key) = session_key(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let profiles = &state.profiles;
    let grants = profiles.grants.lock().await;
    let grant = grants.get(&key);
    let choice = profiles
        .choices
        .lock()
        .await
        .get(&key)
        .copied()
        .or(grant.map(|g| g.requested))
        .unwrap_or_default();
    let pending = profiles
        .pending
        .lock()
        .await
        .values()
        .find(|p| p.session == key && p.created.elapsed() < Duration::from_secs(PENDING_SECONDS))
        .map(|p| p.requested);
    let namespace = state.config.profile_namespace.clone();
    Json(Connection {
        available: namespace.is_some(),
        live: state.provider.live(),
        reason: namespace
            .is_none()
            .then_some("Profile sync is not configured on this beta server."),
        namespace,
        connected: grant.is_some(),
        email: session.identity.email,
        principal: grant.and_then(|g| g.principal.clone()),
        requested: choice,
        granted: grant.map(|g| g.access).unwrap_or_default(),
        pending,
    })
    .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConnectRequest {
    drive: bool,
    calendar: Calendar,
}
async fn connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ConnectRequest>,
) -> Response {
    let Some(key) = session_key(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let requested = Requested {
        drive: request.drive,
        calendar: request.calendar,
    };
    if !requested.any() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error":"Choose Drive app data or Calendar access before connecting."})),
        )
            .into_response();
    }
    if state.config.profile_namespace.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(
                serde_json::json!({"error":"Profile sync is not configured on this beta server."}),
            ),
        )
            .into_response();
    }
    let profiles = &state.profiles;
    profiles.choices.lock().await.insert(key.clone(), requested);
    let mut pending = profiles.pending.lock().await;
    pending.retain(|_, p| {
        p.session != key && p.created.elapsed() < Duration::from_secs(PENDING_SECONDS)
    });
    if pending.len() >= 256 {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({"error":"Google connection is busy. Try again shortly."})),
        )
            .into_response();
    }
    let id = token();
    let nonce = token();
    let verifier = Zeroizing::new(token());
    let mut url =
        url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").expect("constant URL");
    let scope = requested.scopes().join(" ");
    url.query_pairs_mut().extend_pairs([
        ("client_id", state.config.google_client_id.as_str()),
        ("redirect_uri", state.config.google_callback().as_str()),
        ("response_type", "code"),
        ("scope", scope.as_str()),
        ("state", id.as_str()),
        ("nonce", nonce.as_str()),
        ("code_challenge", hash(&verifier).as_str()),
        ("code_challenge_method", "S256"),
        ("access_type", "offline"),
        ("prompt", "consent"),
        ("include_granted_scopes", "false"),
    ]);
    pending.insert(
        id,
        Pending {
            created: Instant::now(),
            session: key,
            verifier,
            nonce,
            requested,
        },
    );
    Json(serde_json::json!({"url": url.as_str()})).into_response()
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}
async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let outcome = |name: &str| Redirect::to(&format!("/app/#profiles={name}")).into_response();
    let Some(session) = state.session(&headers).await else {
        return Redirect::to("/beta").into_response();
    };
    let Some(key) = session_key(&headers) else {
        return Redirect::to("/beta").into_response();
    };
    let Some(id) = query.state else {
        return outcome("failed");
    };
    let pending = {
        let mut entries = state.profiles.pending.lock().await;
        let matches = entries.get(&id).is_some_and(|p| {
            p.session == key && p.created.elapsed() < Duration::from_secs(PENDING_SECONDS)
        });
        if !matches {
            return outcome("failed");
        }
        entries.remove(&id).expect("matched pending consent")
    };
    // Denied or cancelled consent keeps the existing grant and choices intact.
    if query.error.is_some() {
        return outcome("denied");
    }
    let Some(code) = query.code.filter(|c| !c.is_empty() && c.len() <= 4096) else {
        return outcome("failed");
    };
    let Ok(_permit) = state.auth_slots.clone().try_acquire_owned() else {
        return outcome("failed");
    };
    let Ok(tokens) = state
        .provider
        .exchange(&code, &pending.verifier, &pending.nonce)
        .await
    else {
        return outcome("failed");
    };
    // The consenting account must be the signed-in beta identity; a different
    // account's grant is never bound to this session.
    if tokens.subject.as_deref() != Some(session.identity.subject.as_str()) {
        return outcome("mismatch");
    }
    let access = Access::from_scope(tokens.scope.as_deref(), pending.requested);
    let principal = if access.drive {
        match state
            .provider
            .drive(&tokens.access_token, DriveRequest::About)
            .await
        {
            Ok(about) => about
                .pointer("/user/permissionId")
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty() && p.len() <= 128 && p.is_ascii())
                .map(|p| format!("drive:{p}")),
            Err(_) => None,
        }
    } else {
        None
    };
    if access.drive && principal.is_none() {
        return outcome("failed");
    }
    let mut grants = state.profiles.grants.lock().await;
    let previous = grants.remove(&key);
    grants.insert(
        key,
        Grant {
            access_token: tokens.access_token,
            // Google omits the refresh token on re-consent; keep the earlier one.
            refresh_token: tokens
                .refresh_token
                .or(previous.and_then(|g| g.refresh_token)),
            expires: Instant::now() + Duration::from_secs(tokens.expires_in.clamp(30, 86_400)),
            requested: pending.requested,
            access,
            principal,
        },
    );
    outcome("connected")
}

async fn disconnect(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(key) = session_key(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    // Local disconnect only: the project-wide grant is not revoked on behalf of
    // the user's other devices.
    state.profiles.forget(&key).await;
    StatusCode::NO_CONTENT.into_response()
}

async fn drive(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<DriveRequest>,
) -> Response {
    let reject = |status: StatusCode, message: &str| {
        (status, Json(serde_json::json!({"error": message}))).into_response()
    };
    let Some(key) = session_key(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if let Err(message) = request.validate() {
        return reject(StatusCode::BAD_REQUEST, message);
    }
    let mut grants = state.profiles.grants.lock().await;
    let Some(grant) = grants.get_mut(&key) else {
        return reject(
            StatusCode::CONFLICT,
            "Connect Google with Drive app data in Preferences before using profiles.",
        );
    };
    if request.needs_drive() && !grant.access.drive {
        return reject(
            StatusCode::CONFLICT,
            "Google did not grant Drive app data access. Reconnect Google in Preferences and approve that permission.",
        );
    }
    if grant.expires.saturating_duration_since(Instant::now()) < REFRESH_MARGIN {
        let Some(refresh) = grant.refresh_token.as_ref() else {
            return reject(
                StatusCode::CONFLICT,
                "The Google connection expired. Reconnect Google in Preferences.",
            );
        };
        match state.provider.refresh(refresh).await {
            Ok(tokens) => {
                // Refresh keeps the requested set: scope changes narrow, never widen.
                let access = Access::from_scope(tokens.scope.as_deref(), grant.requested);
                grant.access_token = tokens.access_token;
                if let Some(token) = tokens.refresh_token {
                    grant.refresh_token = Some(token);
                }
                grant.expires =
                    Instant::now() + Duration::from_secs(tokens.expires_in.clamp(30, 86_400));
                grant.access = access;
                if request.needs_drive() && !access.drive {
                    return reject(
                        StatusCode::CONFLICT,
                        "Google no longer grants Drive app data access. Reconnect Google in Preferences.",
                    );
                }
            }
            Err(_) => {
                return reject(
                    StatusCode::BAD_GATEWAY,
                    "Google did not renew the connection. Retry, or reconnect Google in Preferences.",
                );
            }
        }
    }
    let access_token = grant.access_token.clone();
    drop(grants);
    match state.provider.drive(&access_token, request).await {
        Ok(value) => Json(value).into_response(),
        Err(_) => reject(
            StatusCode::BAD_GATEWAY,
            "Google Drive did not answer. Retry discovery.",
        ),
    }
}

/// Production provider: Google's token endpoint and Drive v3 over HTTPS.
pub struct GoogleProfiles {
    config: Arc<crate::config::Config>,
    login: Arc<crate::google::Google>,
    client: reqwest::Client,
}
impl GoogleProfiles {
    pub fn new(
        config: Arc<crate::config::Config>,
        login: Arc<crate::google::Google>,
    ) -> Result<Self, reqwest::Error> {
        Ok(Self {
            config,
            login,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .https_only(true)
                .build()?,
        })
    }
    async fn tokens(
        &self,
        form: &[(&str, &str)],
        nonce: Option<&str>,
    ) -> Result<Tokens, ProviderFailed> {
        #[derive(Deserialize)]
        struct Raw {
            access_token: String,
            refresh_token: Option<String>,
            expires_in: Option<u64>,
            scope: Option<String>,
            id_token: Option<String>,
        }
        let response = self
            .client
            .post("https://oauth2.googleapis.com/token")
            .form(form)
            .send()
            .await
            .map_err(|_| ProviderFailed)?;
        let raw: Raw = bounded_json(response, MAX_JSON_BYTES).await?;
        if raw.access_token.is_empty() || raw.access_token.len() > 4096 {
            return Err(ProviderFailed);
        }
        let subject = match (raw.id_token, nonce) {
            (Some(id_token), Some(nonce)) => Some(
                self.login
                    .verify_id_token(&id_token, nonce)
                    .await
                    .map_err(|_| ProviderFailed)?
                    .subject,
            ),
            _ => None,
        };
        Ok(Tokens {
            access_token: Zeroizing::new(raw.access_token),
            refresh_token: raw.refresh_token.map(Zeroizing::new),
            expires_in: raw.expires_in.unwrap_or(3600),
            scope: raw.scope,
            subject,
        })
    }
}
#[async_trait]
impl ProfileProvider for GoogleProfiles {
    fn live(&self) -> bool {
        true
    }
    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<Tokens, ProviderFailed> {
        self.tokens(
            &[
                ("code", code),
                ("client_id", self.config.google_client_id.as_str()),
                ("client_secret", self.config.google_client_secret.as_str()),
                ("redirect_uri", self.config.google_callback().as_str()),
                ("grant_type", "authorization_code"),
                ("code_verifier", verifier),
            ],
            Some(nonce),
        )
        .await
    }
    async fn refresh(&self, refresh_token: &str) -> Result<Tokens, ProviderFailed> {
        self.tokens(
            &[
                ("refresh_token", refresh_token),
                ("client_id", self.config.google_client_id.as_str()),
                ("client_secret", self.config.google_client_secret.as_str()),
                ("grant_type", "refresh_token"),
            ],
            None,
        )
        .await
    }
    async fn drive(
        &self,
        access_token: &str,
        request: DriveRequest,
    ) -> Result<Value, ProviderFailed> {
        const BASE: &str = "https://www.googleapis.com/drive/v3";
        let bearer = format!("Bearer {access_token}");
        let get = |url: String, query: Vec<(&str, String)>| {
            self.client
                .get(url)
                .header(reqwest::header::AUTHORIZATION, &bearer)
                .query(&query)
        };
        let response = match request {
            DriveRequest::About => get(
                format!("{BASE}/about"),
                vec![("fields", "user(permissionId,emailAddress)".into())],
            ),
            DriveRequest::List { page_token } => {
                let mut query = vec![
                    ("spaces", "appDataFolder".to_owned()),
                    ("pageSize", "50".to_owned()),
                    ("q", "appProperties has { key='shepType' and value='profile' }".to_owned()),
                    (
                        "fields",
                        "nextPageToken,incompleteSearch,files(id,name,mimeType,size,trashed,ownedByMe,spaces,appProperties,md5Checksum)".to_owned(),
                    ),
                ];
                if let Some(token) = page_token {
                    query.push(("pageToken", token));
                }
                get(format!("{BASE}/files"), query)
            }
            DriveRequest::StartPageToken => get(format!("{BASE}/changes/startPageToken"), vec![]),
            DriveRequest::Changes { page_token } => get(
                format!("{BASE}/changes"),
                vec![
                    ("pageToken", page_token),
                    ("spaces", "appDataFolder".into()),
                    ("includeRemoved", "true".into()),
                    ("restrictToMyDrive", "false".into()),
                    ("pageSize", "50".into()),
                    (
                        "fields",
                        "nextPageToken,newStartPageToken,changes(fileId,removed,file(id,name,mimeType,size,trashed,ownedByMe,spaces,appProperties,md5Checksum))".into(),
                    ),
                ],
            ),
            DriveRequest::Metadata { file_id } => get(
                format!("{BASE}/files/{file_id}"),
                vec![(
                    "fields",
                    "id,name,mimeType,size,trashed,ownedByMe,spaces,appProperties,md5Checksum".into(),
                )],
            ),
            DriveRequest::Media { file_id } => {
                let response = get(format!("{BASE}/files/{file_id}"), vec![("alt", "media".into())])
                    .send()
                    .await
                    .map_err(|_| ProviderFailed)?;
                let bytes = bounded_bytes(response, MAX_MEDIA_BYTES).await?;
                let text = String::from_utf8(bytes).map_err(|_| ProviderFailed)?;
                return Ok(serde_json::json!({ "media": text }));
            }
            DriveRequest::GenerateIds { count } => get(
                format!("{BASE}/files/generateIds"),
                vec![("count", count.to_string()), ("space", "appDataFolder".into())],
            ),
            DriveRequest::Create { metadata, media } => {
                let boundary = format!("shep-{}", token());
                let body = format!(
                    "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{}\r\n--{boundary}\r\nContent-Type: application/json\r\n\r\n{media}\r\n--{boundary}--",
                    serde_json::to_string(&metadata).map_err(|_| ProviderFailed)?
                );
                self.client
                    .post("https://www.googleapis.com/upload/drive/v3/files")
                    .header(reqwest::header::AUTHORIZATION, &bearer)
                    .header(
                        reqwest::header::CONTENT_TYPE,
                        format!("multipart/related; boundary={boundary}"),
                    )
                    .query(&[
                        ("uploadType", "multipart"),
                        (
                            "fields",
                            "id,name,mimeType,size,trashed,ownedByMe,spaces,appProperties,md5Checksum",
                        ),
                    ])
                    .body(body)
            }
        }
        .send()
        .await
        .map_err(|_| ProviderFailed)?;
        let status = response.status().as_u16();
        // 404 metadata reads are a meaningful discovery answer, not a failure.
        if status == 404 {
            return Ok(serde_json::json!({ "missing": true }));
        }
        bounded_json(response, MAX_JSON_BYTES).await
    }
}
async fn bounded_bytes(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, ProviderFailed> {
    if !response.status().is_success()
        || response.content_length().is_some_and(|n| n > limit as u64)
    {
        return Err(ProviderFailed);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ProviderFailed)? {
        if bytes.len() + chunk.len() > limit {
            return Err(ProviderFailed);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
async fn bounded_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    limit: usize,
) -> Result<T, ProviderFailed> {
    let bytes = bounded_bytes(response, limit).await?;
    serde_json::from_slice(&bytes).map_err(|_| ProviderFailed)
}

#[cfg(test)]
pub(crate) mod tests;
