//! Isolated native cache-recovery fixture. It never reads keychain credentials
//! or contacts a provider; Refresh supplies the fictional acknowledged copy.
use crate::{
    mail_actions::{journal::*, *},
    model::*,
    store::Store,
};
fn mode() -> Option<String> {
    std::env::args().find_map(|arg| {
        if arg == "--move-recovery" {
            Some("committed".into())
        } else {
            arg.strip_prefix("--move-recovery=").map(str::to_owned)
        }
    })
}
pub async fn seed(store: &Store) -> anyhow::Result<()> {
    if mode().is_none() {
        return Ok(());
    }
    let original=parse_mail("preview-work","42.700","INBOX",b"From: Morgan <morgan@example.test>\r\nTo: alex@studio.example\r\nSubject: Recovered keepsake\r\n\r\nPreserved original message.\r\n\r\nThis cached letter remains readable while its moved copy needs a destination identity.\r\nAll original content stays available after restarting Shep.".to_vec(),true,false)?;
    store.upsert(vec![original.clone()]).await?;
    let mode = mode().unwrap();
    let destination = if mode == "copied" {
        "preview-personal"
    } else {
        "preview-work"
    };
    let mut receipt = MoveReceipt::server(
        &original.summary,
        destination,
        "Projects",
        None,
        Fingerprint::of(&original.raw),
    );
    let accounts: Vec<Account> = store.get("accounts").await?;
    receipt.connections = accounts
        .iter()
        .filter(|a| a.id == "preview-work" || a.id == destination)
        .map(|a| (a.id.clone(), connection_key(a)))
        .collect();
    let record = MoveRecord::new(original.summary, receipt);
    store.prepare_mail_move(record.clone()).await?;
    if mode != "unconfirmed" {
        let mut receipt = record.receipt.clone();
        if mode == "copied" {
            receipt.current = Some(
                parse_mail(
                    destination,
                    "91.701",
                    "Projects",
                    original.raw.clone(),
                    true,
                    false,
                )?
                .summary,
            );
        }
        store
            .checkpoint_mail_move(
                record,
                if mode == "copied" {
                    MoveStage::Copied
                } else {
                    MoveStage::Committed
                },
                receipt,
            )
            .await?;
    }
    Ok(())
}
pub async fn refresh(store: &Store) -> anyhow::Result<()> {
    if mode().as_deref() != Some("committed") {
        return Ok(());
    }
    for record in store.pending_mail_moves(None, None).await? {
        if record.stage != MoveStage::Committed {
            continue;
        }
        let raw = store.raw_message(record.original.id.clone()).await?;
        let mut resolved = parse_mail(
            &record.receipt.account,
            "91.701",
            &record.receipt.folder,
            raw,
            false,
            false,
        )?;
        resolved.summary.timestamp = record.original.timestamp;
        store.resolve_mail_move(record, resolved).await?;
    }
    Ok(())
}

/// Native tests use the production journal runner with an object-scoped fixture
/// connection. No keychain access or network traffic is possible here.
pub async fn recover_move(
    store: &Store,
    record: MoveRecord,
    action: RecoveryAction,
) -> anyhow::Result<MoveRecord> {
    anyhow::ensure!(
        mode().is_some(),
        "This preview has no move-recovery fixture."
    );
    let accounts: Vec<Account> = store.get("accounts").await?;
    let identities = accounts
        .iter()
        .filter(|a| a.id == record.original.account_id || a.id == record.receipt.account)
        .map(|a| (a.id.clone(), connection_key(a)))
        .collect();
    let mut connection = FixtureConnection {
        store: store.clone(),
        identities,
    };
    runner::recover_reviewed(
        store,
        &mut connection,
        record,
        action == RecoveryAction::UseExistingCopy,
    )
    .await
}
struct FixtureConnection {
    store: Store,
    identities: Vec<(String, String)>,
}
#[async_trait::async_trait]
impl runner::Connection for FixtureConnection {
    fn identities(&self) -> Vec<(String, String)> {
        self.identities.clone()
    }
    async fn prepare(&mut self, _: &MoveRecord) -> anyhow::Result<()> {
        anyhow::bail!("Recovery must not submit a new move")
    }
    async fn submit(
        &mut self,
        _: &MoveRecord,
        _: Option<Vec<u8>>,
    ) -> Result<Option<String>, runner::SubmissionError> {
        Err(runner::SubmissionError::NotApplied(
            "Recovery must not submit a new move".into(),
        ))
    }
    async fn finish_source(&mut self, record: &MoveRecord) -> anyhow::Result<()> {
        let saved = self.store.mail_move(record.token.clone()).await?;
        anyhow::ensure!(
            saved.stage == MoveStage::Copied && saved.receipt.current.is_some(),
            "Verify the destination before source cleanup"
        );
        let count: u64 = self.store.get("fixture_move_cleanup").await?;
        self.store.put("fixture_move_cleanup", count + 1).await?;
        Ok(())
    }
    async fn locate(&mut self, receipt: &MoveReceipt) -> anyhow::Result<StoredMail> {
        tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
        let attempts: u64 = self.store.get("fixture_move_lookup").await?;
        self.store.put("fixture_move_lookup", attempts + 1).await?;
        anyhow::ensure!(
            mode().as_deref() != Some("fail-once") || attempts > 0,
            "Fixture destination is temporarily unavailable. Try recovery again."
        );
        let record = self
            .store
            .mail_move(receipt.recovery.clone().unwrap())
            .await?;
        let raw = self.store.raw_message(record.original.id).await?;
        parse_mail(
            &receipt.account,
            "91.701",
            &receipt.folder,
            raw,
            false,
            false,
        )
    }
}
