//! IMAP adapter for the durable mail-move runner. Opening connections and all
//! capability/UID checks finish before the journal permits a mutating command.
use super::*;
use crate::mail_actions::{
    MoveReceipt, connection_key,
    journal::MoveRecord,
    runner::{Connection, SubmissionError},
};

pub struct ImapMoveConnection {
    source: Account,
    source_secret: SecretString,
    destination: Option<(Account, SecretString)>,
    source_session: Option<async_imap::Session<Tls>>,
    destination_session: Option<async_imap::Session<Tls>>,
}
impl ImapMoveConnection {
    pub fn new(
        source: Account,
        source_secret: SecretString,
        destination: Option<(Account, SecretString)>,
    ) -> Self {
        Self {
            source,
            source_secret,
            destination,
            source_session: None,
            destination_session: None,
        }
    }
    fn destination(&self) -> (&Account, &SecretString) {
        match &self.destination {
            Some((account, secret)) => (account, secret),
            None => (&self.source, &self.source_secret),
        }
    }
}

#[async_trait]
impl Connection for ImapMoveConnection {
    fn identities(&self) -> Vec<(String, String)> {
        let mut identities = vec![(self.source.id.clone(), connection_key(&self.source))];
        if let Some((account, _)) = &self.destination {
            identities.push((account.id.clone(), connection_key(account)));
        }
        identities
    }
    async fn prepare(&mut self, record: &MoveRecord) -> anyhow::Result<()> {
        anyhow::ensure!(
            record.original.account_id == self.source.id
                && record.receipt.account == self.destination().0.id
                && self.source.protocol == Protocol::Imap
                && self.destination().0.protocol == Protocol::Imap,
            "This move requires the original IMAP connections."
        );
        receipts::quoted(&record.receipt.folder)?;
        chrono::DateTime::from_timestamp(record.original.timestamp, 0)
            .context("Invalid message date")?;
        let (source, destination) = tokio::time::timeout(Duration::from_secs(45), async {
            let mut source = imap(&self.source, &self.source_secret).await?;
            let uid = validate_uid(
                &record.original,
                source.select(&record.original.folder).await?.uid_validity,
            )?;
            anyhow::ensure!(
                uid.parse::<u32>()? != 0,
                "The source message has an invalid UID. Refresh its folder."
            );
            let capability = if self.destination.is_some() {
                "UIDPLUS"
            } else {
                "MOVE"
            };
            anyhow::ensure!(
                source.capabilities().await?.has_str(capability),
                "The source server needs {capability} to move this message safely."
            );
            let destination = match &self.destination {
                Some((account, secret)) => Some(imap(account, secret).await?),
                None => None,
            };
            Ok::<_, anyhow::Error>((source, destination))
        })
        .await
        .context("Connecting timed out. No move was submitted; try again.")??;
        self.source_session = Some(source);
        self.destination_session = destination;
        Ok(())
    }
    async fn submit(
        &mut self,
        record: &MoveRecord,
        raw: Option<Vec<u8>>,
    ) -> Result<Option<String>, SubmissionError> {
        // Take ownership so even this adapter cannot submit twice on one preflight.
        let operation = async {
            if let Some(mut destination) = self.destination_session.take() {
                let raw = raw.context("The upload has no original content")?;
                receipts::append_message(
                    &mut destination,
                    &record.original,
                    &record.receipt.folder,
                    &raw,
                )
                .await
            } else {
                anyhow::ensure!(
                    self.destination.is_none(),
                    "The destination connection is unavailable"
                );
                let mut source = self
                    .source_session
                    .take()
                    .context("The source connection is unavailable")?;
                let (_, uid) = record
                    .original
                    .remote_id
                    .split_once('.')
                    .context("Missing source UID")?;
                receipts::move_message(&mut source, uid, &record.receipt.folder).await
            }
            // Drop a completed connection. Waiting for LOGOUT here can turn an
            // already acknowledged write into a timeout with a lost receipt.
        };
        match tokio::time::timeout(Duration::from_secs(60), operation).await {
            Ok(Ok(uid)) => Ok(uid),
            Ok(Err(error)) if error.downcast_ref::<receipts::UploadRejected>().is_some() => {
                Err(SubmissionError::NotApplied(format!("{error:#}")))
            }
            Ok(Err(error)) => Err(SubmissionError::Unconfirmed(format!("{error:#}"))),
            Err(_) => Err(SubmissionError::Unconfirmed(
                "The server acknowledgment timed out.".into(),
            )),
        }
    }
    async fn finish_source(&mut self, record: &MoveRecord) -> anyhow::Result<()> {
        anyhow::ensure!(
            record.stage == crate::mail_actions::journal::MoveStage::Copied,
            "Source cleanup needs a verified destination copy"
        );
        tokio::time::timeout(Duration::from_secs(45), async {
            let mut source = match self.source_session.take() {
                Some(session) => session,
                None => imap(&self.source, &self.source_secret).await?,
            };
            // Recheck UIDVALIDITY/capabilities after an interrupted copy and
            // remove exactly the original UID; tagged OK acknowledges cleanup.
            finish_transfer_commands(&mut source, &record.original).await
        })
        .await
        .context("Source cleanup timed out. Its destination copy is retained.")?
    }
    async fn locate(&mut self, receipt: &MoveReceipt) -> anyhow::Result<StoredMail> {
        let (account, secret) = self.destination();
        tokio::time::timeout(
            Duration::from_secs(120),
            recovery::resolve(account, secret, receipt),
        )
        .await
        .context("Finding the moved message timed out. Its cached original is retained.")?
    }
}
