//! In-memory submission receipts, never persisted passwords or mail. A send is
//! allowed only against an existing server-issued reservation. After a restart,
//! old client IDs are unknown and cannot silently become a fresh send.
use super::*;
use axum::{extract::Path, routing::get};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use shep_mail_core::outgoing::PreparedMessage;
use std::time::Instant;

#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    Reserved,
    Submitting,
    Delivered,
    Rejected,
    Uncertain,
    Cancelled,
}
pub(super) struct Submission {
    pub(super) subject: String,
    pub(super) created: Instant,
    digest: Option<[u8; 32]>,
    phase: Phase,
}
pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/mail/send", post(send))
        .route("/api/mail/outgoing/prepare", post(prepare))
}
pub fn receipt_routes() -> Router<AppState> {
    Router::new()
        .route("/api/mail/outgoing/reserve", post(reserve))
        .route("/api/mail/outgoing/{id}", get(status))
        .route("/api/mail/outgoing/{id}/cancel", post(cancel))
}
async fn reserve(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
) -> Response {
    if !state
        .config
        .mail_endpoints
        .iter()
        .any(|e| e.service == policy::Service::Smtp)
    {
        return unavailable();
    }
    let mut records = state.mail.outgoing.lock().await;
    records.retain(|_, r| {
        r.created.elapsed()
            < Duration::from_secs(if r.phase == Phase::Reserved {
                1800
            } else {
                8 * 3600
            })
    });
    if records.len() >= 1024 {
        return fail(
            StatusCode::TOO_MANY_REQUESTS,
            "Outgoing receipt capacity is full. Keep your draft and try later.",
        );
    }
    let mut id = crate::token();
    while records.contains_key(&id) {
        id = crate::token();
    }
    records.insert(
        id.clone(),
        Submission {
            subject: session.identity.subject,
            created: Instant::now(),
            digest: None,
            phase: Phase::Reserved,
        },
    );
    (
        StatusCode::CREATED,
        Json(serde_json::json!({"id":id,"state":"reserved","expires_in":1800})),
    )
        .into_response()
}
fn receipt(id: &str, phase: Phase) -> Response {
    let (status, error) = match phase {
        Phase::Reserved | Phase::Cancelled => (StatusCode::OK, None),
        Phase::Submitting => (StatusCode::ACCEPTED, None),
        Phase::Delivered => (StatusCode::OK, None),
        Phase::Rejected => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Some(
                "The message was not accepted. Keep the draft, correct the SMTP settings or recipients, then review before sending again.",
            ),
        ),
        Phase::Uncertain => (
            StatusCode::CONFLICT,
            Some(
                "SMTP did not confirm delivery. Check Sent or contact the recipient before deciding to send again.",
            ),
        ),
    };
    (status,Json(serde_json::json!({"id":id,"state":phase,"error":error,"message_id":format!("<{id}@shep.so>"),"sent_copy":false}))).into_response()
}
async fn status(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
) -> Response {
    let records = state.mail.outgoing.lock().await;
    match records
        .get(&id)
        .filter(|r| r.subject == session.identity.subject)
    {
        Some(record) => receipt(&id, record.phase),
        None => fail(
            StatusCode::NOT_FOUND,
            "This submission is unknown to this server session. Do not resend automatically; review its delivery status first.",
        ),
    }
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    content_id: Option<String>,
    id: String,
    name: String,
    media_type: String,
    data: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Compose {
    id: String,
    connection: Connection,
    draft: Draft,
    #[serde(default)]
    files: Vec<File>,
}
async fn prepare(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Extension(permit): Extension<Arc<Admission>>,
    Json(request): Json<Compose>,
) -> Response {
    if let Err((status, message)) = allowed(&state, &request.connection, true) {
        return fail(status, message);
    }
    if request.draft.account_id != request.connection.account.id
        || request.files.len() > shep_mail_core::compose::MAX_ATTACHMENTS
    {
        return fail(
            StatusCode::BAD_REQUEST,
            "Choose this draft's account and valid attachments.",
        );
    }
    // Check the reservation before building MIME. IDs missing after a process
    // restart, guessed IDs, other users' IDs and expired reservations never send.
    {
        let records = state.mail.outgoing.lock().await;
        if !records.get(&request.id).is_some_and(|r| {
            r.subject == session.identity.subject
                && r.phase == Phase::Reserved
                && r.digest.is_none()
                && r.created.elapsed() < Duration::from_secs(1800)
        }) {
            return fail(
                StatusCode::CONFLICT,
                "This submission cannot be started. Keep your draft and review delivery before making a new send request.",
            );
        }
    }
    let Compose {
        id,
        connection,
        draft,
        files,
    } = request;
    let account = connection.account.clone();
    let message_id = format!("<{id}@shep.so>");
    let local_id = id.clone();
    let built = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let _permit = permit;
        let mut parts = Vec::new();
        let mut draft = draft;
        // Do not trust a separate attachment size/association claim from JSON.
        draft.attachments.clear();
        let mut bytes = 0usize;
        let mut ids = HashSet::new();
        for file in files {
            anyhow::ensure!(
                ids.insert(file.id.clone())
                    && !file.name.is_empty()
                    && file.name.len() <= 1024
                    && !file.name.contains(['\r', '\n', '\0']),
                "Invalid attachment"
            );
            let data = STANDARD.decode(file.data)?;
            bytes = bytes
                .checked_add(data.len())
                .ok_or_else(|| anyhow::anyhow!("Attachment size overflow"))?;
            anyhow::ensure!(
                bytes <= shep_mail_core::compose::MAX_ATTACHMENT_BYTES,
                "Attachments exceed the sending limit"
            );
            let attachment = DraftAttachment {
                content_id: file.content_id,
                id: file.id,
                name: file.name,
                media_type: file.media_type,
                size: data.len(),
            };
            draft.attachments.push(attachment.clone());
            parts.push(shep_mail_core::compose::FilePart {
                attachment,
                bytes: data,
            });
        }
        let message =
            shep_mail_core::compose::build_with_message_id(&account, &draft, parts, &message_id)?;
        let wire = PreparedMessage::from_message(&message)?;
        let digest = wire.digest(&account)?;
        let cached = parse_mail(&account.id,&format!("local-sent-{local_id}"),"Sent",message.formatted(),false,false)?;
        let reply = shep_mail_core::compose::ReplyHeaders::parse(&mailparse::parse_mail(&cached.raw)?).envelope();
        let local = serde_json::json!({"core":cached.summary,"text":cached.text,"reply":reply,"local":true});
        Ok((wire, digest, local))
    })
    .await;
    let Ok(Ok((wire, digest, local))) = built else {
        return fail(
            StatusCode::BAD_REQUEST,
            "Check To/Cc/Bcc, subject, reply headers and attachment sizes. Your draft was not sent.",
        );
    };
    let mut records = state.mail.outgoing.lock().await;
    let Some(record) = records
        .get_mut(&id)
        .filter(|r| r.subject == session.identity.subject)
    else {
        return fail(
            StatusCode::CONFLICT,
            "The reservation is unavailable. Review Outbox before another attempt.",
        );
    };
    if record.phase != Phase::Reserved
        || record.digest.is_some()
        || record.created.elapsed() >= Duration::from_secs(1800)
    {
        return fail(
            StatusCode::CONFLICT,
            "This reservation was already prepared, cancelled or expired. Review Outbox before another attempt.",
        );
    }
    record.digest = Some(digest);
    // Only the hash stays in this process; the client owns the exact MIME.
    (
        StatusCode::OK,
        Json(serde_json::json!({"id":id,"state":"reserved","wire":wire,"mail":local})),
    )
        .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Send {
    id: String,
    connection: Connection,
    wire: PreparedMessage,
}
async fn send(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Extension(permit): Extension<Arc<Admission>>,
    Json(request): Json<Send>,
) -> Response {
    if let Err((status, message)) = allowed(&state, &request.connection, true) {
        return fail(status, message);
    }
    let Send {
        id,
        connection,
        wire,
    } = request;
    {
        let records = state.mail.outgoing.lock().await;
        if !records.get(&id).is_some_and(|r| {
            r.subject == session.identity.subject
                && r.digest.is_some()
                && (r.phase != Phase::Reserved || r.created.elapsed() < Duration::from_secs(1800))
        }) {
            return fail(
                StatusCode::CONFLICT,
                "This submission is unprepared, expired or unknown. Review Outbox before another send.",
            );
        }
    }
    let account = connection.account.clone();
    let build_permit = permit.clone();
    let validated = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let _permit = build_permit;
        let digest = wire.digest(&account)?;
        let (envelope, raw) = wire.decode()?;
        Ok((digest, envelope, raw))
    })
    .await;
    let Ok(Ok((digest, envelope, raw))) = validated else {
        return fail(
            StatusCode::BAD_REQUEST,
            "The prepared message is invalid. Keep the draft and review Outbox.",
        );
    };
    {
        let mut records = state.mail.outgoing.lock().await;
        let Some(record) = records
            .get_mut(&id)
            .filter(|r| r.subject == session.identity.subject)
        else {
            return fail(
                StatusCode::CONFLICT,
                "The submission is unknown. Review delivery before another send.",
            );
        };
        if record.digest != Some(digest) {
            return fail(
                StatusCode::CONFLICT,
                "The prepared content or account changed. Its delivery state was kept.",
            );
        }
        if record.phase != Phase::Reserved {
            return receipt(&id, record.phase);
        }
        if record.created.elapsed() >= Duration::from_secs(1800) {
            return fail(
                StatusCode::CONFLICT,
                "The reservation expired. Review Outbox before another send.",
            );
        }
        record.phase = Phase::Submitting;
    }
    let hub = state.mail.clone();
    let task_id = id.clone();
    // An accepted send outlives an HTTP disconnect. Only status is retained when
    // the operation finishes; passwords and MIME are dropped, never written.
    let task = tokio::spawn(async move {
        let transport = hub.transport.clone();
        // Supervise provider panics even after the HTTP waiter disconnects.
        let operation = tokio::spawn(async move {
            let _permit = permit;
            tokio::time::timeout(
                Duration::from_secs(90),
                transport.send(&connection, envelope, raw),
            )
            .await
        });
        let phase = match operation.await {
            Ok(Ok(Ok(()))) => Phase::Delivered,
            Ok(Ok(Err(DeliveryFailure::Rejected(_)))) => Phase::Rejected,
            _ => Phase::Uncertain,
        };
        if let Some(record) = hub.outgoing.lock().await.get_mut(&task_id) {
            record.phase = phase;
        }
        phase
    });
    match task.await {
        Ok(phase) => receipt(&id, phase),
        Err(_) => {
            if let Some(record) = state.mail.outgoing.lock().await.get_mut(&id) {
                record.phase = Phase::Uncertain;
            }
            receipt(&id, Phase::Uncertain)
        }
    }
}

/// Atomically prevent an unused reservation from ever submitting. A GET of
/// "reserved" alone is not proof: another HTTP request could still start it.
async fn cancel(
    State(state): State<AppState>,
    Extension(session): Extension<Session>,
    Path(id): Path<String>,
) -> Response {
    let mut records = state.mail.outgoing.lock().await;
    let Some(record) = records
        .get_mut(&id)
        .filter(|r| r.subject == session.identity.subject)
    else {
        return fail(
            StatusCode::NOT_FOUND,
            "This receipt is unknown. Review delivery before another send.",
        );
    };
    if record.phase == Phase::Reserved {
        record.phase = Phase::Cancelled;
    }
    receipt(&id, record.phase)
}
