//! Durable moves under the caller's account lock(s). Network timeouts belong in
//! the connection adapter; never cancel the observation of a SQLite commit.
use super::{MoveReceipt, journal::*};
use crate::{model::StoredMail, store::Store};
use anyhow::Context;

#[derive(Debug)]
pub enum SubmissionError {
    /// Proven no mutation: pre-submission failure, or an atomic APPEND rejection.
    /// A MOVE NO can describe partial success (RFC 6851 §3.3), so is Unconfirmed.
    NotApplied(String),
    Unconfirmed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inspection {
    Unresolved,
    DestinationPresent,
    SourceIntactNoDestinationCopy,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait Connection: Send {
    fn identities(&self) -> Vec<(String, String)>;
    /// Establish sessions/capabilities/UIDVALIDITY and a selectable destination,
    /// without copying or deleting mail.
    async fn prepare(&mut self, record: &MoveRecord) -> anyhow::Result<()>;
    async fn inspect(&mut self, _record: &MoveRecord) -> anyhow::Result<Inspection> {
        Ok(Inspection::Unresolved)
    }
    async fn submit(
        &mut self,
        record: &MoveRecord,
        raw: Option<Vec<u8>>,
    ) -> Result<Option<String>, SubmissionError>;
    /// Revalidate and remove only this source UID, never EXPUNGE the mailbox.
    async fn finish_source(&mut self, record: &MoveRecord) -> anyhow::Result<()>;
    async fn locate(&mut self, receipt: &MoveReceipt) -> anyhow::Result<StoredMail>;
}

fn identity(connection: &dyn Connection, record: &MoveRecord) -> anyhow::Result<()> {
    let expected: std::collections::HashSet<_> = [
        record.original.account_id.as_str(),
        record.receipt.account.as_str(),
    ]
    .into_iter()
    .collect();
    let actual = connection.identities();
    let unique: std::collections::HashSet<_> = actual.iter().map(|(id, _)| id.as_str()).collect();
    anyhow::ensure!(
        actual.len() == unique.len()
            && unique == expected
            && actual.len() == record.receipt.connections.len()
            && actual
                .iter()
                .all(|value| record.receipt.connections.contains(value)),
        "The move's incoming connection changed. Restore its connection settings before recovery."
    );
    Ok(())
}

/// Begin exactly one new provider operation. Success acknowledges the provider
/// even if local cache cleanup needs recovery; `record.error` describes that.
/// Missing destination IDs deliberately return before any additional lookup.
pub async fn start(
    store: &Store,
    connection: &mut dyn Connection,
    record: MoveRecord,
) -> anyhow::Result<MoveRecord> {
    anyhow::ensure!(
        record.stage == MoveStage::Started,
        "This move was already started."
    );
    identity(connection, &record)?;
    connection.prepare(&record).await?;
    let transfer = record.original.account_id != record.receipt.account;
    let raw = if transfer {
        let raw = store.raw_message(record.original.id.clone()).await?;
        anyhow::ensure!(
            record
                .receipt
                .fingerprint
                .as_ref()
                .is_some_and(|f| f.matches(&raw)),
            "The original changed before upload. Refresh its folder."
        );
        Some(raw)
    } else {
        None
    };
    store.prepare_mail_move(record.clone()).await?;
    submit_prepared(store, connection, record, raw).await
}

async fn submit_prepared(
    store: &Store,
    connection: &mut dyn Connection,
    record: MoveRecord,
    raw: Option<Vec<u8>>,
) -> anyhow::Result<MoveRecord> {
    let transfer = record.original.account_id != record.receipt.account;
    let remote = match connection.submit(&record, raw).await {
        Ok(remote) => remote,
        Err(SubmissionError::NotApplied(error)) => {
            store.reject_mail_move(record).await.context("The server did not apply this move, but its recovery record could not be released.")?;
            anyhow::bail!("{error}");
        }
        Err(SubmissionError::Unconfirmed(error)) => {
            return failed(store, record, format!("This move is unconfirmed. Your cached message is retained; Retry move will check its result. {error}")).await;
        }
    };
    let mut receipt = record.receipt.clone();
    receipt.current = MoveReceipt::server(
        &record.original,
        &receipt.account,
        &receipt.folder,
        remote,
        receipt
            .fingerprint
            .clone()
            .context("Missing original identity proof")?,
    )
    .current;
    let record = store.checkpoint_mail_move(record,
        if transfer { MoveStage::Copied } else { MoveStage::Committed }, receipt).await
        .context("The server acknowledged the operation, but saving its receipt failed. Your cached message is retained; Retry move will check its result.")?;
    let record = if transfer {
        finish(store, connection, record).await?
    } else {
        record
    };
    cache(store, record).await
}

async fn failed(store: &Store, record: MoveRecord, error: String) -> anyhow::Result<MoveRecord> {
    store
        .fail_mail_move(record, error.clone())
        .await
        .context("The original is retained, but saving the recovery error failed.")?;
    anyhow::bail!("{error}")
}

async fn finish(
    store: &Store,
    connection: &mut dyn Connection,
    record: MoveRecord,
) -> anyhow::Result<MoveRecord> {
    if let Err(error) = connection.finish_source(&record).await {
        return failed(store,record,format!("The destination has a confirmed copy; source cleanup is unconfirmed. Retry recovery for the same destination. {error:#}")).await;
    }
    store.checkpoint_mail_move(record.clone(), MoveStage::Committed, record.receipt).await
        .context("The source removal was acknowledged, but saving its receipt failed. Recovery must keep the confirmed destination copy.")
}

/// A known UID allows atomic cache completion without another network request.
async fn cache(store: &Store, record: MoveRecord) -> anyhow::Result<MoveRecord> {
    if record.receipt.current.is_none() {
        return Ok(record);
    }
    match store.commit_mail_move_cache(record.clone()).await {
        Ok(located) => Ok(located),
        Err(error) => store.fail_mail_move(record, format!("The message was moved, but updating its cache failed. Retry recovery. {error:#}")).await,
    }
}

/// Resume only after proving the original UID survives and no destination copy
/// exists. Otherwise use an exact existing copy, without repeating an upload.
pub async fn recover(
    store: &Store,
    connection: &mut dyn Connection,
    record: MoveRecord,
) -> anyhow::Result<MoveRecord> {
    identity(connection, &record)?;
    current(store, &record).await?;
    if record.stage != MoveStage::Started {
        return recover_reviewed(store, connection, record, false).await;
    }
    let inspection = match connection.inspect(&record).await {
        Ok(inspection) => inspection,
        Err(error) => {
            return failed(
                store,
                record,
                format!("The move could not be checked. Its original is retained. {error:#}"),
            )
            .await;
        }
    };
    match inspection {
        Inspection::SourceIntactNoDestinationCopy => {
            anyhow::ensure!(
                record.original.account_id == record.receipt.account,
                "An unconfirmed cross-account upload cannot be repeated."
            );
            if let Err(error) = connection.prepare(&record).await {
                return failed(
                    store,
                    record,
                    format!("The move could not be prepared. No new move was submitted. {error:#}"),
                )
                .await;
            }
            current(store, &record).await?;
            submit_prepared(store, connection, record, None).await
        }
        Inspection::DestinationPresent => recover_reviewed(store, connection, record, true).await,
        Inspection::Unresolved => anyhow::bail!(
            "The move could not be verified. Review its source and destination before choosing a recovery option."
        ),
    }
}

async fn current(store: &Store, record: &MoveRecord) -> anyhow::Result<()> {
    let saved = store.mail_move(record.token.clone()).await?;
    anyhow::ensure!(
        serde_json::to_string(&saved)? == serde_json::to_string(record)?,
        "The move recovery changed. Refresh before trying again."
    );
    Ok(())
}

/// A newer destination can replace a pending move only after proving its old
/// destination has no copy and its exact original is still present.
pub async fn release_unapplied(
    store: &Store,
    connection: &mut dyn Connection,
    record: &MoveRecord,
) -> anyhow::Result<bool> {
    identity(connection, record)?;
    current(store, record).await?;
    if record.stage != MoveStage::Started
        || record.original.account_id != record.receipt.account
        || connection.inspect(record).await? != Inspection::SourceIntactNoDestinationCopy
    {
        return Ok(false);
    }
    store.reject_mail_move(record.clone()).await?;
    Ok(true)
}

pub async fn retarget_source(
    store: &Store,
    connection: &mut dyn Connection,
    record: MoveRecord,
) -> anyhow::Result<crate::model::Mail> {
    if release_unapplied(store, connection, &record).await? {
        return Ok(record.original);
    }
    let recovered = recover(store, connection, record).await?;
    anyhow::ensure!(
        recovered.stage == MoveStage::Located,
        "The earlier move's destination identity and cache need recovery before moving elsewhere."
    );
    recovered
        .receipt
        .current
        .context("The earlier move has no verified destination identity.")
}

/// Explicit review authorizes using a verified existing destination copy after
/// an unconfirmed submission. It never authorizes a new MOVE or APPEND.
pub async fn recover_reviewed(
    store: &Store,
    connection: &mut dyn Connection,
    record: MoveRecord,
    use_existing_copy: bool,
) -> anyhow::Result<MoveRecord> {
    identity(connection, &record)?;
    let saved = store.mail_move(record.token.clone()).await?;
    anyhow::ensure!(
        serde_json::to_string(&saved)? == serde_json::to_string(&record)?,
        "The move recovery changed. Refresh before trying again."
    );
    match record.stage {
        MoveStage::Started if !use_existing_copy => anyhow::bail!(
            "This operation was interrupted before its result was saved. Check its source and destination before resolving it; it will not be repeated automatically."
        ),
        MoveStage::Located | MoveStage::Kept => return Ok(record),
        _ => {}
    }
    let mut resolved = match connection.locate(&record.receipt).await {
        Ok(value) => value,
        Err(error) => return failed(store, record, format!("The cached original is available. Finding its moved copy failed; retry recovery. {error:#}")).await,
    };
    // Recheck before any source removal. A provider is not allowed to replace a
    // message solely because its subject or Message-ID happens to match.
    if record
        .receipt
        .fingerprint
        .as_ref()
        .is_none_or(|f| !f.matches(&resolved.raw))
        || resolved.summary.account_id != record.receipt.account
        || resolved.summary.folder != record.receipt.folder
    {
        return failed(store,record,"The destination lookup returned different content or a different folder. The original is retained.".into()).await;
    }
    resolved.summary.timestamp = record.original.timestamp;
    let mut resolved_receipt = record.receipt.clone();
    resolved_receipt.current = Some(resolved.summary.clone());
    if let Err(error) = record.validate_receipt(&resolved_receipt) {
        return failed(store, record, format!("The destination lookup returned an invalid identity. The source was retained. {error:#}")).await;
    }
    let record = if record.stage == MoveStage::Started {
        store
            .checkpoint_mail_move(record, MoveStage::Copied, resolved_receipt)
            .await?
    } else {
        record
    };
    let record = if record.stage == MoveStage::Copied {
        // Retain the original acknowledgment even if a later UIDVALIDITY change
        // required another exact lookup. Its replacement is committed atomically
        // with the cache only after the source removal is acknowledged.
        finish(store, connection, record).await?
    } else {
        record
    };
    match store.resolve_mail_move(record.clone(), resolved).await {
        Ok(located) => Ok(located),
        Err(error) => failed(
            store,
            record,
            format!(
                "The server move is committed; updating its cache failed. Retry recovery. {error:#}"
            ),
        )
        .await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mocked_inspection_failure_and_ambiguous_result_never_submit() {
        for rejected in [false, true] {
            let store = Store::memory().unwrap();
            let original = crate::model::parse_mail(
                "work",
                "42.7",
                "INBOX",
                b"Subject: proof\r\n\r\nOriginal".to_vec(),
                true,
                false,
            )
            .unwrap();
            store.upsert(vec![original.clone()]).await.unwrap();
            let mut receipt = MoveReceipt::server(
                &original.summary,
                "work",
                "Archive",
                None,
                crate::mail_actions::Fingerprint::of(&original.raw),
            );
            receipt.connections = vec![("work".into(), "mock".into())];
            let record = MoveRecord::new(original.summary.clone(), receipt);
            store.prepare_mail_move(record.clone()).await.unwrap();
            let mut connection = MockConnection::new();
            connection
                .expect_identities()
                .returning(|| vec![("work".into(), "mock".into())]);
            connection.expect_inspect().times(1).returning(move |_| {
                if rejected {
                    anyhow::bail!("LIST rejected");
                }
                Ok(Inspection::Unresolved)
            });
            connection.expect_prepare().times(0);
            connection.expect_submit().times(0);
            connection.expect_finish_source().times(0);
            assert!(
                recover(&store, &mut connection, record.clone())
                    .await
                    .is_err()
            );
            assert_eq!(
                store.mail_move(record.token).await.unwrap().stage,
                MoveStage::Started
            );
            assert_eq!(
                store.raw_message(original.summary.id).await.unwrap(),
                original.raw
            );
        }
    }
}
