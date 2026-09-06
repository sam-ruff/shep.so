//! Transient Sent-copy receipts. The browser must commit the reserved identity,
//! destination and exact MIME before asking this process to begin APPEND.
use super::*;
use axum::{extract::Path, routing::get};
use mailparse::MailHeaderMap;
use serde::Serialize;
use sha2::{Digest, Sha256};
use shep_mail_core::{outgoing::PreparedMessage, providers::mail::sent::SentReceipt};
use std::time::Instant;

#[derive(Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Phase {
    Reserved,
    Copying,
    Saved { receipt: SentReceipt },
    Failed,
    Uncertain,
}
pub(super) struct Copy {
    pub(super) subject: String,
    pub(super) created: Instant,
    digest: [u8; 32],
    phase: Phase,
}
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/mail/sent/check", post(check))
        .route("/api/mail/sent/reserve", post(reserve))
        .route("/api/mail/sent/{id}/copy", post(copy))
}
pub(super) fn receipt_routes() -> Router<AppState> {
    Router::new().route("/api/mail/sent/{id}", get(status))
}
fn allowed_sent(state: &AppState, c: &Connection) -> Result<(), (StatusCode, &'static str)> {
    allowed(state, c, false)?;
    if c.account.protocol != Protocol::Imap {
        return Err((StatusCode::BAD_REQUEST, "POP3 keeps Sent copies locally."));
    }
    Ok(())
}
fn reply(id: &str, phase: &Phase) -> Response {
    let mut value = serde_json::to_value(phase).expect("serializable receipt");
    value["id"] = id.into();
    let error = match phase {
        Phase::Failed => Some(
            "Sent could not be checked before uploading. Keep the local copy, reconnect and check Sent before retrying.",
        ),
        Phase::Uncertain => Some(
            "The Sent upload was not confirmed. Check Sent before reviewing another upload; it could create a duplicate.",
        ),
        _ => None,
    };
    value["error"] = serde_json::json!(error);
    (
        if *phase == Phase::Copying {
            StatusCode::ACCEPTED
        } else {
            StatusCode::OK
        },
        Json(value),
    )
        .into_response()
}
async fn status(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
) -> Response {
    match state
        .mail
        .copies
        .lock()
        .await
        .get(&id)
        .filter(|r| r.subject == session.identity.subject)
    {
        Some(r) => reply(&id, &r.phase),
        None => fail(
            StatusCode::NOT_FOUND,
            "This Sent receipt is no longer available. Check the provider's Sent folder before reviewing another upload.",
        ),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Check {
    connection: Connection,
    message_id: String,
}
fn valid_message_id(value: &str) -> bool {
    value.len() <= 1024 && shep_mail_core::compose::message_ids(value) == [value]
}
async fn check(
    State(state): State<AppState>,
    Extension(permit): Extension<Arc<Admission>>,
    Json(request): Json<Check>,
) -> Response {
    if let Err((status, message)) = allowed_sent(&state, &request.connection) {
        return fail(status, message);
    }
    if !valid_message_id(&request.message_id) {
        return fail(
            StatusCode::BAD_REQUEST,
            "The original message identity is invalid. Keep its local copy.",
        );
    }
    let result = tokio::time::timeout(Duration::from_secs(90), async {
        let _permit = permit;
        let mut mailbox = state.mail.transport.sent(&request.connection).await?;
        let folder = mailbox.folder().to_owned();
        anyhow::ensure!(valid_folder(&folder), "Invalid Sent destination");
        let receipt = mailbox.find(&request.message_id).await?;
        anyhow::ensure!(
            receipt.as_ref().is_none_or(|r| r.folder == folder),
            "Changed Sent destination"
        );
        Ok::<_, anyhow::Error>(serde_json::json!({"folder":folder,"receipt":receipt}))
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => fail(
            StatusCode::BAD_GATEWAY,
            "Could not check Sent. Reconnect the account or choose its Sent folder, then retry. No copy was uploaded.",
        ),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Content {
    connection: Connection,
    wire: PreparedMessage,
    timestamp: i64,
}
struct Decoded {
    digest: [u8; 32],
    connection: Connection,
    raw: Vec<u8>,
    message_id: String,
    timestamp: i64,
}
impl Content {
    fn validate(&self, state: &AppState) -> Result<(), (StatusCode, &'static str)> {
        allowed_sent(state, &self.connection)?;
        if self.connection.account.sent_copy != SentCopyPolicy::Automatic
            || !valid_folder(&self.connection.account.sent_folder)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "Choose automatic Sent copies and check the destination before uploading.",
            ));
        }
        Ok(())
    }
    fn decode(self) -> anyhow::Result<Decoded> {
        let digest: [u8; 32] = Sha256::digest(serde_json::to_vec(&(
            self.wire.digest(&self.connection.account)?,
            self.timestamp,
        ))?)
        .into();
        let (_, raw) = self.wire.decode()?;
        let (headers, _) = mailparse::parse_headers(&raw)?;
        let ids = headers.get_all_values("Message-ID");
        anyhow::ensure!(
            ids.len() == 1 && valid_message_id(&ids[0]),
            "Invalid message identity"
        );
        anyhow::ensure!(
            chrono::DateTime::from_timestamp(self.timestamp, 0).is_some(),
            "Invalid original date"
        );
        Ok(Decoded {
            digest,
            connection: self.connection,
            raw,
            message_id: ids[0].clone(),
            timestamp: self.timestamp,
        })
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reserve {
    connection: Connection,
    wire: PreparedMessage,
    timestamp: i64,
    #[serde(default)]
    reviewed_retry: bool,
}
async fn reserve(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Extension(permit): Extension<Arc<Admission>>,
    Json(request): Json<Reserve>,
) -> Response {
    let reviewed = request.reviewed_retry;
    let content = Content {
        connection: request.connection,
        wire: request.wire,
        timestamp: request.timestamp,
    };
    if let Err((status, message)) = content.validate(&state) {
        return fail(status, message);
    }
    let decoded = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        content.decode().map(|decoded| decoded.digest)
    })
    .await;
    let Ok(Ok(digest)) = decoded else {
        return fail(
            StatusCode::BAD_REQUEST,
            "The saved Sent message is invalid. Keep the local copy and review Outbox.",
        );
    };
    let mut copies = state.mail.copies.lock().await;
    copies.retain(|_, r| {
        r.phase == Phase::Copying
            || r.created.elapsed()
                < Duration::from_secs(if r.phase == Phase::Reserved {
                    1800
                } else {
                    8 * 3600
                })
    });
    // A second tab/request cannot reserve another upload while this exact copy
    // is running or acknowledged. Retrying an ambiguous operation is explicit.
    if let Some((id, r)) = copies
        .iter()
        .filter(|(_, r)| r.subject == session.identity.subject && r.digest == digest)
        .max_by_key(|(_, r)| r.created)
        && (!reviewed || !matches!(r.phase, Phase::Failed | Phase::Uncertain))
    {
        return reply(id, &r.phase);
    }
    if copies.len() >= 1024 {
        return fail(
            StatusCode::TOO_MANY_REQUESTS,
            "Sent receipt capacity is full. Keep the local copy and try later.",
        );
    }
    let mut id = crate::token();
    while copies.contains_key(&id) {
        id = crate::token();
    }
    copies.insert(
        id.clone(),
        Copy {
            subject: session.identity.subject,
            created: Instant::now(),
            digest,
            phase: Phase::Reserved,
        },
    );
    reply(&id, &Phase::Reserved)
}
async fn copy(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Extension(permit): Extension<Arc<Admission>>,
    Path(id): Path<String>,
    Json(request): Json<Content>,
) -> Response {
    if let Err((status, message)) = request.validate(&state) {
        return fail(status, message);
    }
    let parsing_permit = permit.clone();
    let decoded = tokio::task::spawn_blocking(move || {
        let _permit = parsing_permit;
        request.decode()
    })
    .await;
    let Ok(Ok(Decoded {
        digest,
        connection,
        raw,
        message_id,
        timestamp,
    })) = decoded
    else {
        return fail(
            StatusCode::BAD_REQUEST,
            "The saved Sent message is invalid. Its upload was not started.",
        );
    };
    {
        let mut copies = state.mail.copies.lock().await;
        let Some(r) = copies
            .get_mut(&id)
            .filter(|r| r.subject == session.identity.subject)
        else {
            return fail(
                StatusCode::NOT_FOUND,
                "This Sent reservation is unknown. Check Sent before reviewing a new upload.",
            );
        };
        if r.digest != digest {
            return fail(
                StatusCode::CONFLICT,
                "The saved Sent content, date or account changed. The earlier upload state was kept.",
            );
        }
        if r.phase != Phase::Reserved {
            return reply(&id, &r.phase);
        }
        if r.created.elapsed() >= Duration::from_secs(1800) {
            return fail(
                StatusCode::CONFLICT,
                "This Sent reservation expired. Check Sent before reviewing a new upload.",
            );
        }
        r.phase = Phase::Copying;
    }
    let hub = state.mail.clone();
    let task_id = id.clone();
    let task = tokio::spawn(async move {
        let transport = hub.transport.clone();
        // Keep the operation and admission alive if its HTTP waiter disappears.
        // The supervisor records uncertainty even if a provider panics.
        let operation = tokio::spawn(async move {
            let _permit = permit;
            tokio::time::timeout(Duration::from_secs(90), async {
                let Ok(mut mailbox) = transport.sent(&connection).await else {
                    return Phase::Failed;
                };
                if mailbox.folder() != connection.account.sent_folder {
                    return Phase::Failed;
                }
                match mailbox.find(&message_id).await {
                    Ok(Some(receipt)) if receipt.folder == connection.account.sent_folder => {
                        return Phase::Saved { receipt };
                    }
                    Ok(None) => {}
                    _ => return Phase::Failed,
                }
                match mailbox.append(&raw, timestamp).await {
                    Ok(receipt) if receipt.folder == connection.account.sent_folder => {
                        Phase::Saved { receipt }
                    }
                    _ => Phase::Uncertain,
                }
            })
            .await
        });
        let phase = match operation.await {
            Ok(Ok(phase)) => phase,
            _ => Phase::Uncertain,
        };
        if let Some(r) = hub.copies.lock().await.get_mut(&task_id) {
            r.phase = phase.clone();
        }
        phase
    });
    match task.await {
        Ok(phase) => reply(&id, &phase),
        Err(_) => {
            if let Some(r) = state.mail.copies.lock().await.get_mut(&id) {
                r.phase = Phase::Uncertain;
            }
            reply(&id, &Phase::Uncertain)
        }
    }
}
