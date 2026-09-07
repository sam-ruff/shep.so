//! Durable provider progress. A surviving Started/Copied record is never proof
//! that an unacknowledged command was rejected, and must not be replayed blindly.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoveStage {
    Started,
    Copied,
    Committed,
    Located,
    Kept,
}
impl MoveStage {
    pub fn key(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::Copied => "copied",
            Self::Committed => "committed",
            Self::Located => "located",
            Self::Kept => "kept",
        }
    }
    pub fn allows(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Started, Self::Copied | Self::Committed)
                | (Self::Copied, Self::Copied | Self::Committed)
                | (Self::Committed, Self::Located)
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveRecord {
    pub token: String,
    pub original: Mail,
    pub receipt: MoveReceipt,
    pub stage: MoveStage,
    pub error: Option<String>,
    #[serde(default)]
    pub attempted: i64,
    #[serde(default)]
    pub retained: Option<Mail>,
}
impl MoveRecord {
    pub fn new(original: Mail, mut receipt: MoveReceipt) -> Self {
        let token = uuid::Uuid::new_v4().to_string();
        receipt.recovery = Some(token.clone());
        Self {
            token,
            original,
            receipt,
            stage: MoveStage::Started,
            error: None,
            attempted: 0,
            retained: None,
        }
    }
    pub fn finished(&self) -> bool {
        matches!(self.stage, MoveStage::Located | MoveStage::Kept)
    }
    pub fn resolved_mail(&self) -> Option<&Mail> {
        if self.stage == MoveStage::Kept {
            self.retained.as_ref()
        } else {
            self.receipt.current.as_ref()
        }
    }
    pub fn validate_receipt(&self, receipt: &MoveReceipt) -> anyhow::Result<()> {
        anyhow::ensure!(
            receipt.account == self.receipt.account
                && receipt.folder == self.receipt.folder
                && receipt.connections == self.receipt.connections
                && receipt.fingerprint == self.receipt.fingerprint
                && receipt.recovery.as_deref() == Some(self.token.as_str()),
            "The move destination or message identity changed. Refresh its folders."
        );
        if let Some(current) = &receipt.current {
            anyhow::ensure!(
                current.account_id == receipt.account
                    && current.folder == receipt.folder
                    && !current.remote_id.is_empty()
                    && current.id
                        == format!(
                            "{}:{}:{}",
                            current.account_id, current.folder, current.remote_id
                        ),
                "The server returned an inconsistent destination identity."
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryAction {
    Retry,
    UseExistingCopy,
    KeepLocal,
}
