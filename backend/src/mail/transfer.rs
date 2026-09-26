//! Cross-account moves between two IMAP accounts. The upload and the source
//! cleanup are separate requests, so the browser saves the destination receipt
//! before the original is removed. Neither step is repeated by the gateway.
use super::*;
use base64::Engine;
use shep_mail_core::providers::mail::transfer::TransferFailure;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait HostedTransfer: Send + Sync {
    /// Check the source, then upload `raw` into `folder` of the destination.
    async fn transfer(
        &self,
        source: &Connection,
        destination: &Connection,
        mail: &Mail,
        folder: &str,
        raw: &[u8],
    ) -> Result<Option<String>, TransferFailure>;
    /// Remove exactly the original UID after its copy was acknowledged.
    async fn finish_transfer(&self, source: &Connection, mail: &Mail) -> anyhow::Result<()>;
}

#[async_trait]
impl HostedTransfer for Servers {
    async fn transfer(
        &self,
        source: &Connection,
        destination: &Connection,
        mail: &Mail,
        folder: &str,
        raw: &[u8],
    ) -> Result<Option<String>, TransferFailure> {
        let origin = self
            .client(source, false)
            .map_err(TransferFailure::NotApplied)?;
        let target = self
            .client(destination, false)
            .map_err(TransferFailure::NotApplied)?;
        origin
            .transfer_source(&source.account, &source.password, mail)
            .await
            .map_err(TransferFailure::NotApplied)?;
        target
            .transfer_upload(
                &destination.account,
                &destination.password,
                mail,
                folder,
                raw,
            )
            .await
    }
    async fn finish_transfer(&self, source: &Connection, mail: &Mail) -> anyhow::Result<()> {
        self.client(source, false)?
            .finish_transfer(&source.account, &source.password, mail)
            .await
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/mail/transfer", post(transfer))
        .route("/api/mail/transfer/finish", post(finish))
}

fn refused() -> Response {
    (
        StatusCode::CONFLICT,
        Json(serde_json::json!({
            "error": "The message was not moved. It stays in its original account; check both accounts and the destination folder, then retry.",
            "refused": true,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Transfer {
    source: Connection,
    destination: Connection,
    mail: Mail,
    folder: String,
    /// Base64 of the cached original message.
    raw: String,
}
async fn transfer(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<Transfer>,
) -> Response {
    for connection in [&request.source, &request.destination] {
        if let Err((status, message)) = allowed(&state, connection, false) {
            return fail(status, message);
        }
    }
    let raw = base64::engine::general_purpose::STANDARD
        .decode(request.raw.as_bytes())
        .ok()
        .filter(|raw| !raw.is_empty() && raw.len() <= MAX_MESSAGE_BYTES);
    let valid = request.source.account.protocol == Protocol::Imap
        && request.destination.account.protocol == Protocol::Imap
        && request.source.account.id != request.destination.account.id
        && valid_mail(&request.source, &request.mail)
        && valid_folder(&request.folder);
    let (true, Some(raw)) = (valid, raw) else {
        return fail(
            StatusCode::BAD_REQUEST,
            "Choose a cached message and a folder in another IMAP account.",
        );
    };
    let result = tokio::time::timeout(
        Duration::from_secs(90),
        state.mail.transfers.transfer(
            &request.source,
            &request.destination,
            &request.mail,
            &request.folder,
            &raw,
        ),
    )
    .await;
    match result {
        Ok(Ok(remote_id)) => {
            Json(serde_json::json!({"committed":true,"remote_id":remote_id})).into_response()
        }
        Ok(Err(TransferFailure::NotApplied(_))) => refused(),
        _ => fail(
            StatusCode::BAD_GATEWAY,
            "The other account did not confirm the copy. The original was kept; check the destination folder before trying again.",
        ),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Finish {
    source: Connection,
    mail: Mail,
}
async fn finish(
    State(state): State<AppState>,
    Extension(_permit): Extension<Arc<Admission>>,
    Json(request): Json<Finish>,
) -> Response {
    if let Err((status, message)) = allowed(&state, &request.source, false) {
        return fail(status, message);
    }
    if request.source.account.protocol != Protocol::Imap
        || !valid_mail(&request.source, &request.mail)
    {
        return fail(
            StatusCode::BAD_REQUEST,
            "Choose the original message in its IMAP account.",
        );
    }
    match tokio::time::timeout(
        Duration::from_secs(45),
        state
            .mail
            .transfers
            .finish_transfer(&request.source, &request.mail),
    )
    .await
    {
        Ok(Ok(())) => Json(serde_json::json!({"committed":true})).into_response(),
        _ => fail(
            StatusCode::BAD_GATEWAY,
            "The original could not be removed yet. Its copy in the other account is kept; retry.",
        ),
    }
}

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;
