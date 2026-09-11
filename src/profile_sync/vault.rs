//! Google-only password sync. Passwords move between the OS keychain and a
//! replaceable app-data vault; SQLite keeps only revisions. The contract is the
//! credential section of `docs/agents/PROFILE_SYNC_HANDOVER.md`.
mod import;
mod pass;
#[cfg(test)]
pub(crate) mod tests;

use super::control::Control;
use crate::{
    credentials::Credentials,
    model::{Account, ConnectionTarget, SmtpAuth},
    store::Store,
};
use async_trait::async_trait;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use shep_profile_core::history;
pub use shep_profile_core::vault::Field;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub use import::{Import, activate, record_failure, stage, test, unstage};
pub use pass::reconcile;

pub(crate) const STORAGE_KEY: &str = "profile_credentials_v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub profile: Uuid,
    pub generation: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Key { sequence: u32 },
    Vault { revision: u64 },
}

/// Verified Drive metadata for one credential file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFile {
    pub id: String,
    pub key: Uuid,
    pub kind: Kind,
    pub size: u64,
    pub sha256: String,
}

pub struct NewFile {
    pub kind: Kind,
    pub key: Uuid,
    pub bytes: Vec<u8>,
}

/// Drive app-data storage for credential files. Implementations create and
/// delete whole files; there is no in-place update.
#[async_trait]
pub trait Remote: Send + Sync {
    async fn list(&self, scope: Scope) -> anyhow::Result<Vec<RemoteFile>>;
    async fn download(&self, scope: Scope, file: &RemoteFile) -> anyhow::Result<Vec<u8>>;
    async fn create(&self, scope: Scope, file: NewFile) -> anyhow::Result<RemoteFile>;
    async fn delete(&self, file: &RemoteFile) -> anyhow::Result<()>;
}

/// Mail server login check used before an imported password is activated.
#[async_trait]
pub trait Tester: Send + Sync {
    async fn test(
        &self,
        account: &Account,
        target: ConnectionTarget,
        secret: &SecretString,
    ) -> anyhow::Result<()>;
}

pub struct MailTester;
#[async_trait]
impl Tester for MailTester {
    async fn test(
        &self,
        account: &Account,
        target: ConnectionTarget,
        secret: &SecretString,
    ) -> anyhow::Result<()> {
        match target {
            ConnectionTarget::Incoming => {
                crate::providers::mail::test_incoming(account, secret).await
            }
            ConnectionTarget::Smtp => crate::providers::mail::test_smtp(account, secret).await,
        }
        .map(|_| ())
    }
}

/// Accounts removed in the shared causal history.
#[async_trait]
pub trait Tombstones: Send + Sync {
    async fn removed(&self, account: Uuid) -> anyhow::Result<bool>;
}
#[async_trait]
impl Tombstones for super::replica::Replica {
    async fn removed(&self, account: Uuid) -> anyhow::Result<bool> {
        let target = history::target(&shep_profile_core::Action::AccountRemoved { id: account });
        Ok(!self.versions(target, None).await?.is_empty())
    }
}

pub struct Context<'a> {
    pub store: &'a Store,
    pub credentials: &'a Credentials,
    pub remote: &'a dyn Remote,
    pub tester: &'a dyn Tester,
    pub history: &'a dyn Tombstones,
    pub control: &'a Control,
    /// Explicit retry of revisions whose connection test failed before.
    pub retry_failed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Synced {
    /// Last vault revision this device published or imported for the slot.
    pub revision: u64,
    /// Last revision whose connection test failed; not retried automatically.
    pub failed: u64,
}

/// Device-local password sync state. It never holds a password, a digest of
/// one or key material.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Local {
    pub binding: Option<history::Binding>,
    pub device: Option<Uuid>,
    pub revision: u64,
    /// The toggle was turned off; remove this device's entries when online.
    pub withdraw: bool,
    pub slots: BTreeMap<String, Synced>,
    /// Native accounts whose staged keychain slots may still need deleting.
    pub staged: BTreeSet<String>,
}
impl Local {
    pub fn synced(&self, account: Uuid, field: Field) -> Synced {
        self.slots
            .get(&slot_key(account, field))
            .copied()
            .unwrap_or_default()
    }
}

pub fn slot_key(account: Uuid, field: Field) -> String {
    format!("{account}:{}", field.as_str())
}

/// Everything one pass needs, read in one cache transaction.
#[derive(Clone, Debug)]
pub struct Plan {
    pub binding: history::Binding,
    pub device: Uuid,
    pub publish: bool,
    pub local: Local,
    pub accounts: Vec<Account>,
    /// Native account ID to shared account UUID.
    pub mapping: BTreeMap<String, Uuid>,
    pub suppressed: BTreeSet<Uuid>,
    pub reconnect: BTreeSet<String>,
}
impl Plan {
    pub fn scope(&self) -> Scope {
        Scope {
            profile: self.binding.profile,
            generation: self.binding.generation,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub published: usize,
    pub removed: usize,
    pub imported: usize,
    /// Imports whose connection test failed; the previous password was kept.
    pub failed: usize,
    /// Entries whose last import failed; they wait for an explicit retry.
    pub held: usize,
    /// Entries waiting for a reviewed reconnection or a complete pair.
    pub waiting: usize,
    pub unreadable: usize,
    pub rotated: bool,
    pub withdrawn: bool,
    /// A newer vault format is present; this device will not rewrite it.
    pub read_only: bool,
}

#[derive(Debug, Default)]
pub struct Outcome {
    pub report: Report,
    pub imports: Vec<Import>,
}

/// Passwords a native account needs: incoming always, SMTP when separate.
pub fn fields(account: &Account) -> Vec<Field> {
    let mut fields = vec![Field::Incoming];
    if account.smtp_separate_password && account.smtp_auth != SmtpAuth::None {
        fields.push(Field::Smtp);
    }
    fields
}

pub fn keychain_key(account: &str, field: Field) -> String {
    match field {
        Field::Incoming => account.to_owned(),
        Field::Smtp => format!("{account}:smtp"),
    }
}
/// Staging never shares a key with an active slot: IDs cannot contain ':'.
pub fn staged_key(account: &str, field: Field) -> String {
    format!("{account}:vault-{}", field.as_str())
}

/// The portable connection another client computes endpoints from.
pub fn portable(
    account: &Account,
    shared: Uuid,
) -> anyhow::Result<shep_profile_core::account::Connection> {
    let mut exported = account.clone();
    exported.id = shared.to_string();
    super::metadata::export_account(&exported, shared)?
        .into_iter()
        .find_map(|change| match change.action {
            shep_profile_core::Action::AccountConnection { account } => Some(account),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("The account has no portable connection."))
}
