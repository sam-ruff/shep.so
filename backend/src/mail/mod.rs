//! Authenticated, transient mail operations. Browser storage owns durable state.
mod outgoing;
pub mod policy;
mod sent;
pub fn receipt_routes() -> Router<AppState> {
    outgoing::receipt_routes().merge(sent::receipt_routes())
}
#[cfg(test)]
mod tests;

use crate::{AppState, Session};
use async_trait::async_trait;
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Extension, Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::post,
};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use shep_mail_core::{
    mail_actions::{Flags, MoveReceipt},
    model::*,
    providers::mail::{DeliveryFailure, gateway::PinnedMail},
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc};
use tokio_stream::{StreamExt, wrappers::ReceiverStream};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub account: Account,
    pub password: SecretString,
}
impl Connection {
    fn validate(&self) -> Result<(), &'static str> {
        let a = &self.account;
        if a.validate().is_err()
            || a.id.is_empty()
            || a.id.len() > 64
            || !a
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || a.username.len() > 1024
            || a.smtp_username.len() > 1024
            || self.password.expose_secret().is_empty()
            || self.password.expose_secret().len() > 4096
            || self.password.expose_secret().contains(['\r', '\n', '\0'])
        {
            return Err("Check the account, username, password and server settings.");
        }
        Ok(())
    }
}

#[async_trait]
pub trait HostedMail: Send + Sync {
    async fn probe(&self, connection: &Connection, smtp: bool) -> anyhow::Result<()>;
    async fn sync(
        &self,
        connection: &Connection,
        known: &HashSet<String>,
        folder: &str,
        output: mpsc::Sender<MailSyncItem>,
    ) -> anyhow::Result<Vec<String>>;
    async fn flags(&self, connection: &Connection, mail: &Mail, flags: Flags)
    -> anyhow::Result<()>;
    async fn move_mail(
        &self,
        connection: &Connection,
        mail: &Mail,
        folder: &str,
    ) -> anyhow::Result<Option<String>>;
    async fn resolve_move(
        &self,
        _connection: &Connection,
        _receipt: &MoveReceipt,
    ) -> anyhow::Result<Mail> {
        anyhow::bail!("Move recovery is unavailable.")
    }
    async fn sent(
        &self,
        _connection: &Connection,
    ) -> anyhow::Result<Box<dyn shep_mail_core::providers::mail::sent::SentConnection>> {
        anyhow::bail!("Sent recovery is unavailable.")
    }
    async fn send(
        &self,
        connection: &Connection,
        envelope: lettre::address::Envelope,
        raw: Vec<u8>,
    ) -> Result<(), DeliveryFailure>;
}
struct Servers {
    endpoints: Vec<policy::Endpoint>,
}
impl Servers {
    fn client(&self, connection: &Connection, smtp: bool) -> anyhow::Result<PinnedMail> {
        let a = &connection.account;
        let peer = if smtp {
            policy::resolve(
                &self.endpoints,
                &a.smtp_host,
                a.smtp_port,
                policy::Service::Smtp,
            )
        } else {
            policy::resolve(&self.endpoints, &a.host, a.port, policy::incoming(a))
        }
        .map_err(anyhow::Error::msg)?;
        Ok(PinnedMail::new(peer, peer))
    }
}
#[async_trait]
impl HostedMail for Servers {
    async fn probe(&self, c: &Connection, smtp: bool) -> anyhow::Result<()> {
        let client = self.client(c, smtp)?;
        if smtp {
            client.probe_smtp(&c.account, &c.password).await
        } else {
            client.probe_incoming(&c.account, &c.password).await
        }
    }
    async fn sync(
        &self,
        c: &Connection,
        known: &HashSet<String>,
        folder: &str,
        output: mpsc::Sender<MailSyncItem>,
    ) -> anyhow::Result<Vec<String>> {
        self.client(c, false)?
            .sync_folder(&c.account, &c.password, known, folder, output)
            .await
    }
    async fn flags(&self, c: &Connection, mail: &Mail, flags: Flags) -> anyhow::Result<()> {
        self.client(c, false)?
            .set_flags(&c.account, &c.password, mail, flags)
            .await
    }
    async fn move_mail(
        &self,
        c: &Connection,
        mail: &Mail,
        folder: &str,
    ) -> anyhow::Result<Option<String>> {
        self.client(c, false)?
            .move_mail(&c.account, &c.password, mail, folder)
            .await
    }
    async fn resolve_move(&self, c: &Connection, receipt: &MoveReceipt) -> anyhow::Result<Mail> {
        Ok(self
            .client(c, false)?
            .resolve_move(&c.account, &c.password, receipt)
            .await?
            .summary)
    }
    async fn sent(
        &self,
        c: &Connection,
    ) -> anyhow::Result<Box<dyn shep_mail_core::providers::mail::sent::SentConnection>> {
        Ok(Box::new(
            self.client(c, false)?.sent(&c.account, &c.password).await?,
        ))
    }
    async fn send(
        &self,
        c: &Connection,
        envelope: lettre::address::Envelope,
        raw: Vec<u8>,
    ) -> Result<(), DeliveryFailure> {
        let client = self.client(c, true).map_err(|_| {
            DeliveryFailure::Rejected("SMTP endpoint is not enabled for the beta.".into())
        })?;
        client
            .send_raw(&c.account, &c.password, &envelope, &raw)
            .await
    }
}
pub struct MailHub {
    pub transport: Arc<dyn HostedMail>,
    slots: Arc<Semaphore>,
    users: Mutex<HashMap<String, Arc<Semaphore>>>,
    outgoing: Mutex<HashMap<String, outgoing::Submission>>,
    copies: Mutex<HashMap<String, sent::Copy>>,
}
impl MailHub {
    pub fn new(endpoints: Vec<policy::Endpoint>) -> Self {
        Self {
            transport: Arc::new(Servers { endpoints }),
            slots: Arc::new(Semaphore::new(8)),
            users: Default::default(),
            outgoing: Default::default(),
            copies: Default::default(),
        }
    }
    async fn admit(&self, subject: &str) -> Result<Arc<Admission>, ()> {
        let global = self.slots.clone().try_acquire_owned().map_err(|_| ())?;
        let mut users = self.users.lock().await;
        // Inactive identities have no operation guard holding another reference.
        users.retain(|_, s| Arc::strong_count(s) > 1 || s.available_permits() < 2);
        let user = users
            .entry(subject.into())
            .or_insert_with(|| Arc::new(Semaphore::new(2)))
            .clone();
        let user = user.try_acquire_owned().map_err(|_| ())?;
        Ok(Arc::new(Admission {
            _global: global,
            _user: user,
        }))
    }
}
struct Admission {
    _global: OwnedSemaphorePermit,
    _user: OwnedSemaphorePermit,
}
pub fn routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/mail/probe", post(probe))
        .route("/api/mail/sync", post(sync))
        .route("/api/mail/flags", post(flags))
        .route("/api/mail/move", post(move_mail))
        .route("/api/mail/resolve-move", post(resolve_move))
        .merge(outgoing::routes())
        .merge(sent::routes())
        // Admission runs before JSON is buffered, bounding body memory as well
        // as connections. Eight global/two identity operations; no waiting queue.
        .layer(axum::extract::DefaultBodyLimit::max(36 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(state, admit))
}
async fn admit(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let session = request
        .extensions()
        .get::<Session>()
        .expect("outer authentication gate");
    if state.config.mail_endpoints.is_empty() {
        return unavailable();
    }
    let Ok(permit) = state.mail.admit(&session.identity.subject).await else {
        return fail(
            StatusCode::TOO_MANY_REQUESTS,
            "Mail is busy. Keep browsing and try this action again shortly.",
        );
    };
    request.extensions_mut().insert(permit);
    next.run(request).await
}
fn fail(status: StatusCode, message: &'static str) -> Response {
    (status, Json(serde_json::json!({"error":message}))).into_response()
}
fn unavailable() -> Response {
    fail(
        StatusCode::NOT_IMPLEMENTED,
        "The mail transport is not enabled yet. No mail operation was performed.",
    )
}
fn allowed(state: &AppState, c: &Connection, smtp: bool) -> Result<(), (StatusCode, &'static str)> {
    c.validate().map_err(|m| (StatusCode::BAD_REQUEST, m))?;
    let a = &c.account;
    let endpoint = if smtp {
        policy::resolve(
            &state.config.mail_endpoints,
            &a.smtp_host,
            a.smtp_port,
            policy::Service::Smtp,
        )
    } else {
        policy::resolve(
            &state.config.mail_endpoints,
            &a.host,
            a.port,
            policy::incoming(a),
        )
    };
    endpoint.map(|_| ()).map_err(|m| (StatusCode::FORBIDDEN, m))
}
fn valid_folder(folder: &str) -> bool {
    !folder.is_empty() && folder.len() <= 1024 && !folder.contains(['\r', '\n', '\0'])
}
fn valid_mail(c: &Connection, m: &Mail) -> bool {
    m.account_id == c.account.id
        && valid_folder(&m.folder)
        && !m.remote_id.is_empty()
        && m.remote_id.len() <= 1024
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    connection: Connection,
    #[serde(default)]
    smtp: bool,
}
async fn probe(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<Probe>,
) -> Response {
    if let Err((status, message)) = allowed(&state, &request.connection, request.smtp) {
        return fail(status, message);
    }
    match tokio::time::timeout(
        Duration::from_secs(40),
        state
            .mail
            .transport
            .probe(&request.connection, request.smtp),
    )
    .await
    {
        Ok(Ok(())) => Json(serde_json::json!({"connected":true,"sent":false})).into_response(),
        _ => fail(
            StatusCode::BAD_GATEWAY,
            "Could not verify the mail connection. Check TLS, hostname, port and password, then retry.",
        ),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SyncRequest {
    connection: Connection,
    folder: String,
    #[serde(default)]
    known: HashSet<String>,
}
async fn sync(
    State(state): State<AppState>,
    Extension(permit): Extension<Arc<Admission>>,
    Json(request): Json<SyncRequest>,
) -> Response {
    if let Err((status, message)) = allowed(&state, &request.connection, false) {
        return fail(status, message);
    }
    if !valid_folder(&request.folder)
        || request.known.len() > 100_000
        || request.known.iter().any(|id| id.len() > 2048)
    {
        return fail(
            StatusCode::BAD_REQUEST,
            "Choose a valid folder and cache identity list.",
        );
    }
    let (bytes, receiver) = mpsc::channel::<Bytes>(1);
    let transport = state.mail.transport.clone();
    tokio::spawn(async move {
        let _permit = permit;
        let (events, mut receive) = mpsc::channel(1);
        let mut provider = tokio::spawn(async move {
            transport
                .sync(&request.connection, &request.known, &request.folder, events)
                .await
        });
        // Drain all buffered events before publishing the completion marker.
        loop {
            tokio::select! {
                _=bytes.closed()=>{provider.abort();let _=provider.await;return;}
                event=tokio::time::timeout(Duration::from_secs(45),receive.recv())=>{
                    match event {
                        Ok(Some(event))=>{
                            if !emit(&bytes, move || match event {
                                MailSyncItem::Message(mail)=>{
                                    let reply=mailparse::parse_mail(&mail.raw).ok().map(|p|shep_mail_core::compose::ReplyHeaders::parse(&p).envelope());
                                    let sent_message_id = unique_sent_identity(&mail.raw);
                                    serde_json::json!({"kind":"message","mail":mail,"reply":reply,"sent_message_id":sent_message_id})
                                },
                                MailSyncItem::Flags(flags)=>serde_json::json!({"kind":"flags","flags":flags}),
                                MailSyncItem::Reconcile{account,folder,live_ids}=>serde_json::json!({"kind":"reconcile","account":account,"folder":folder,"live_ids":live_ids}),
                                MailSyncItem::Folders(account,folders)=>serde_json::json!({"kind":"folders","account":account,"folders":folders}),
                                MailSyncItem::SentFolder(account,folder)=>serde_json::json!({"kind":"sent_folder","account":account,"folder":folder}),
                                MailSyncItem::SkippedLarge=>serde_json::json!({"kind":"skipped_large"}),
                            }).await { provider.abort();let _=provider.await;return; }
                        }
                        Ok(None)=>break,
                        Err(_)=>{
                            provider.abort();let _=provider.await;
                            let _=emit(&bytes,|| serde_json::json!({"kind":"error","error":"Mail sync stopped responding. Your cached mail was kept; retry Refresh."})).await;
                            return;
                        }
                    }
                }
            }
        }
        let result = match (&mut provider).await {
            Ok(Ok(folders)) => serde_json::json!({"kind":"done","folders":folders}),
            _ => {
                serde_json::json!({"kind":"error","error":"Mail sync did not finish. Your cached mail was kept; check the connection and retry."})
            }
        };
        let _ = emit(&bytes, move || result).await;
    });
    let stream = ReceiverStream::new(receiver).map(Ok::<_, std::convert::Infallible>);
    (
        [(header::CONTENT_TYPE, "application/x-ndjson; charset=utf-8")],
        Body::from_stream(stream),
    )
        .into_response()
}
async fn emit(
    output: &mpsc::Sender<Bytes>,
    value: impl FnOnce() -> serde_json::Value + Send + 'static,
) -> bool {
    let data = tokio::task::spawn_blocking(move || {
        let mut data = serde_json::to_vec(&value()).expect("serializable sync event");
        data.push(b'\n');
        Bytes::from(data)
    })
    .await;
    match data {
        Ok(data) => output.send(data).await.is_ok(),
        Err(_) => false,
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeFlags {
    connection: Connection,
    mail: Mail,
    unread: Option<bool>,
    starred: Option<bool>,
}
async fn flags(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<ChangeFlags>,
) -> Response {
    if let Err((status, message)) = allowed(&state, &request.connection, false) {
        return fail(status, message);
    }
    if !valid_mail(&request.connection, &request.mail)
        || (request.unread.is_none() && request.starred.is_none())
    {
        return fail(
            StatusCode::BAD_REQUEST,
            "Choose a message from this account and a read/flag change.",
        );
    }
    match tokio::time::timeout(
        Duration::from_secs(40),
        state.mail.transport.flags(
            &request.connection,
            &request.mail,
            Flags {
                unread: request.unread,
                starred: request.starred,
            },
        ),
    )
    .await
    {
        Ok(Ok(())) => Json(serde_json::json!({"committed":true})).into_response(),
        _ => fail(
            StatusCode::BAD_GATEWAY,
            "The server did not confirm the flag change. Refresh to check its state, then retry if needed.",
        ),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Move {
    connection: Connection,
    mail: Mail,
    folder: String,
}
async fn move_mail(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<Move>,
) -> Response {
    if let Err((status, message)) = allowed(&state, &request.connection, false) {
        return fail(status, message);
    }
    if !valid_mail(&request.connection, &request.mail) || !valid_folder(&request.folder) {
        return fail(
            StatusCode::BAD_REQUEST,
            "Choose a message and destination in this account.",
        );
    }
    match tokio::time::timeout(
        Duration::from_secs(40),
        state
            .mail
            .transport
            .move_mail(&request.connection, &request.mail, &request.folder),
    )
    .await
    {
        Ok(Ok(remote_id)) => {
            Json(serde_json::json!({"committed":true,"remote_id":remote_id})).into_response()
        }
        _ => fail(
            StatusCode::BAD_GATEWAY,
            "The server did not confirm the move. Refresh both folders before trying again.",
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveMove {
    connection: Connection,
    receipt: MoveReceipt,
}
async fn resolve_move(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<ResolveMove>,
) -> Response {
    if let Err((status, message)) = allowed(&state, &request.connection, false) {
        return fail(status, message);
    }
    let receipt = &request.receipt;
    let valid = receipt.account == request.connection.account.id
        && request.connection.account.protocol == Protocol::Imap
        && valid_folder(&receipt.folder)
        && receipt.connections.is_empty()
        && receipt
            .current
            .as_ref()
            .is_none_or(|m| valid_mail(&request.connection, m) && m.folder == receipt.folder)
        && receipt.fingerprint.as_ref().is_some_and(|f| {
            f.bytes <= 25 * 1024 * 1024
                && f.message_id
                    .as_ref()
                    .is_none_or(|id| id.len() <= 998 && !id.contains(['\r', '\n', '\0']))
        });
    if !valid {
        return fail(
            StatusCode::BAD_REQUEST,
            "Choose a valid move in this IMAP account.",
        );
    }
    match tokio::time::timeout(
        Duration::from_secs(40),
        state
            .mail
            .transport
            .resolve_move(&request.connection, receipt),
    )
    .await
    {
        Ok(Ok(mail)) => Json(serde_json::json!({"mail":mail})).into_response(),
        _ => fail(
            StatusCode::BAD_GATEWAY,
            "Could not identify one unchanged copy in the destination. Refresh the folder and choose the message to move back.",
        ),
    }
}

// Handover needs exactly one complete identity, unlike reply hints which may
// recover a usable reference from a malformed message. Runs in the emit worker.
fn unique_sent_identity(raw: &[u8]) -> Option<String> {
    use mailparse::MailHeaderMap;
    let prefix = raw.get(..raw.len().min(64 * 1024))?;
    let end = prefix
        .windows(4)
        .position(|s| s == b"\r\n\r\n")
        .map(|i| i + 4)
        .or_else(|| prefix.windows(2).position(|s| s == b"\n\n").map(|i| i + 2))?;
    let (headers, _) = mailparse::parse_headers(&prefix[..end]).ok()?;
    let ids = headers.get_all_values("Message-ID");
    (ids.len() == 1
        && ids[0].len() <= 1024
        && shep_mail_core::compose::message_ids(&ids[0]) == [ids[0].clone()])
    .then(|| ids[0].clone())
}
