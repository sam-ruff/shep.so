//! Target-independent history contract shared by the native SQLite journal, the
//! in-memory journal and the browser WASM entry. No storage or HTTP lives here.
use crate::{Action, Change};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const PAGE_SIZE: usize = 50;
pub const APPLY_BATCH: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "This profile belongs to another account, application or generation. Reopen the intended profile."
    )]
    Binding,
    #[error(
        "The profile history could not be saved or read. Check device storage and retry the same operation."
    )]
    Storage,
    #[error("This profile history is already open. Close the other owner before retrying.")]
    Owned,
    #[error("The profile changed while it was being reviewed. Refresh the review before saving.")]
    Changed,
    #[error("This field has concurrent changes. Review every version before resolving it.")]
    Conflict,
    #[error("This profile or account was removed. Create a new identity before adding it again.")]
    Removed,
    #[error(
        "Some profile history is missing or waiting to be applied. Finish discovery before publishing local changes."
    )]
    Incomplete,
    #[error(
        "An immutable profile operation has different bytes or request data. Keep both records for review."
    )]
    Identity,
    #[error(
        "The profile history contains a causal cycle. Keep the local setup and review the source."
    )]
    Cycle,
    #[error(
        "Too many independent profile versions need reconciliation. Update Shep before publishing more changes."
    )]
    Heads,
    #[error("The profile worker is busy. Retry this same request shortly.")]
    Busy,
    #[error("The profile worker stopped. Reopen it and retry this same request.")]
    Stopped,
    #[error(transparent)]
    Record(#[from] crate::Error),
}
impl Error {
    /// Stable machine-readable classification for clients that cannot match
    /// on the Rust enum, such as the browser worker.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Binding => "binding",
            Self::Storage => "storage",
            Self::Owned => "owned",
            Self::Changed => "changed",
            Self::Conflict => "conflict",
            Self::Removed => "removed",
            Self::Incomplete => "incomplete",
            Self::Identity => "identity",
            Self::Cycle => "cycle",
            Self::Heads => "heads",
            Self::Busy => "busy",
            Self::Stopped => "stopped",
            Self::Record(crate::Error::Invalid) => "invalid",
            Self::Record(crate::Error::TooLarge) => "too_large",
            Self::Record(crate::Error::Upgrade) => "upgrade",
            Self::Record(crate::Error::LocalData) => "local_data",
        }
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// Provider identity/namespace are trusted inputs from the authenticated
/// transport, never a user-entered email or portable OAuth client/grant ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub namespace: String,
    pub principal: String,
    pub profile: Uuid,
    pub generation: Uuid,
}
impl Binding {
    pub(crate) fn validate(&self) -> Result<()> {
        if !crate::namespace(&self.namespace)
            || self.profile.is_nil()
            || self.generation.is_nil()
            || !crate::text(&self.principal, 320, false)
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
    pub fn storage_key(&self) -> Result<String> {
        self.validate()?;
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).map_err(|_| Error::Binding)?)
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    pub target: String,
    pub versions: Vec<Uuid>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalEdit {
    /// Generate once before enqueueing; reuse the whole request after a lost reply.
    pub operation: Uuid,
    pub expected_revision: u64,
    pub changes: Vec<Change>,
    #[serde(default)]
    pub resolutions: Vec<Resolution>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    State,
    Import {
        record: String,
    },
    Edit {
        edit: LocalEdit,
    },
    Drain,
    Fields {
        after: Option<String>,
    },
    Versions {
        target: String,
        after: Option<Uuid>,
    },
    Value {
        target: String,
        operation: Uuid,
    },
    ExportRecord {
        expected_revision: u64,
        after: u64,
    },
    /// Imported originals and acknowledged local writes; excludes unsent edits.
    ExportAcknowledgedRecord {
        expected_revision: u64,
        after: u64,
    },
    NextUpload,
    Reserve {
        operation: Uuid,
        file_id: String,
    },
    Confirm {
        operation: Uuid,
        file_id: String,
        sha256: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub device: Uuid,
    pub revision: u64,
    pub operations: u64,
    pub waiting: u64,
    pub ready: u64,
    pub queued: u64,
    pub fields: u64,
    pub conflicts: u64,
    pub removed: bool,
    #[serde(default)]
    pub initialized: bool,
}

/// Small projection for profile discovery and enrollment reviews. Counts describe
/// account definitions and setting intents (including explicit resets), not
/// credential availability or successful local application.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Overview {
    pub state: State,
    pub name: Option<String>,
    pub name_conflict: bool,
    pub accounts: u64,
    pub settings: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub target: String,
    pub versions: u64,
    pub conflict: bool,
    pub revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub operation: Uuid,
    pub device: Uuid,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Upload {
    pub operation: Uuid,
    pub record: String,
    pub sha256: String,
    pub file_id: Option<String>,
}
/// One original portable operation. Local device identity, queue state and
/// reserved upload IDs never leave the source journal through this interface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub position: u64,
    pub operation: Uuid,
    pub record: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Reply {
    State(State),
    Fields(Vec<Field>),
    Versions(Vec<Version>),
    Value(Change),
    Upload(Option<Upload>),
    Record(Option<Record>),
}

pub(crate) fn conflicting(target: &str, versions: u64) -> bool {
    versions > 1 && !target.ends_with(":removed")
}
pub(crate) fn valid_file_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
pub(crate) fn parse_uuid(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value).map_err(|_| Error::Storage)
}
pub(crate) fn account_id(action: &Action) -> Option<Uuid> {
    match action {
        Action::AccountConnection { account } => Some(account.id),
        Action::AccountName { id, .. } | Action::AccountRemoved { id } => Some(*id),
        _ => None,
    }
}
pub(crate) fn preserves_extensions(old: &Change, new: &Change) -> bool {
    old.extra.iter().all(|(k, v)| new.extra.get(k) == Some(v))
        && match (&old.action, &new.action) {
            (
                Action::AccountConnection { account: old },
                Action::AccountConnection { account: new },
            ) => old.extra.iter().all(|(k, v)| new.extra.get(k) == Some(v)),
            _ => true,
        }
}
pub fn target(action: &Action) -> String {
    match action {
        Action::AccountConnection { account } => format!("account:{}:connection", account.id),
        Action::AccountName { id, .. } => format!("account:{id}:name"),
        Action::AccountRemoved { id } => format!("account:{id}:removed"),
        Action::Setting { key, .. } | Action::SettingRemoved { key } => format!(
            "setting:{}",
            serde_json::to_value(key)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default()
        ),
        Action::ProfileName { .. } => "profile:name".into(),
        Action::ProfileRemoved => "profile:removed".into(),
        Action::ProfileSetup { .. } => "profile:setup".into(),
    }
}
