//! Device-local enrollment choices. These never form part of a shared operation.
use super::*;
use crate::model::Preferences;
use shep_profile_core::history;

pub(crate) const STORAGE_KEY: &str = "profile_enrollment_v1";
pub(crate) const SEED_KEY: &str = "profile_enrollment_seed_v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedChunk {
    pub operation: Uuid,
    pub expected_revision: Option<u64>,
    pub changes: Vec<shep_profile_core::Change>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seed {
    pub binding_key: String,
    pub chunks: Vec<SeedChunk>,
    /// Maps this device's account IDs to stable shared IDs; the map stays local.
    pub account_ids: std::collections::BTreeMap<String, Uuid>,
}
impl Seed {
    pub(crate) fn create(
        selection: &Selection,
        options: Options,
        accounts: &[crate::model::Account],
        preferences: &Preferences,
    ) -> anyhow::Result<Self> {
        use shep_profile_core::{Action, Change};
        let mut changes = vec![Change {
            action: Action::ProfileName {
                name: selection.name.clone(),
            },
            extra: Default::default(),
        }];
        let mut account_ids = std::collections::BTreeMap::new();
        let mut identities = std::collections::HashSet::new();
        if options.accounts {
            for account in accounts {
                let id = Uuid::parse_str(&account.id)
                    .ok()
                    .filter(|id| !id.is_nil())
                    .unwrap_or_else(Uuid::new_v4);
                anyhow::ensure!(
                    identities.insert(id) && !account_ids.contains_key(&account.id),
                    "Resolve duplicate account identities before sharing this workspace."
                );
                account_ids.insert(account.id.clone(), id);
                let mut portable = account.clone();
                portable.id = id.to_string();
                changes.extend(super::metadata::export_account(&portable, id)?);
            }
        }
        if options.settings {
            for key in super::metadata::SETTINGS {
                if let Some(value) = super::metadata::setting_value(*key, preferences) {
                    changes.push(Change {
                        action: Action::Setting { key: *key, value },
                        extra: Default::default(),
                    });
                }
            }
        }
        let chunks = changes
            .chunks(shep_profile_core::MAX_CHANGES)
            .map(|changes| SeedChunk {
                operation: Uuid::new_v4(),
                expected_revision: None,
                changes: changes.to_vec(),
            })
            .collect();
        let seed = Self {
            binding_key: selection.binding.storage_key()?,
            chunks,
            account_ids,
        };
        seed.validate(selection)?;
        Ok(seed)
    }

    pub(crate) fn validate(&self, selection: &Selection) -> anyhow::Result<()> {
        use shep_profile_core::{Action, Operation};
        selection.validate()?;
        anyhow::ensure!(
            selection.origin == Origin::Create
                && !selection.ready
                && self.binding_key == selection.binding.storage_key()?
                && !self.chunks.is_empty(),
            "The saved setup does not match this pending profile. Keep it for recovery."
        );
        let mut operations = std::collections::HashSet::new();
        let mut targets = std::collections::HashSet::new();
        let mut connections = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        let mut profile_names = 0;
        for chunk in &self.chunks {
            anyhow::ensure!(
                operations.insert(chunk.operation)
                    && chunk.expected_revision.is_none_or(|v| v <= i64::MAX as u64),
                "The saved setup contains duplicate operations or an invalid revision."
            );
            // Use the shared codec to validate values, forbidden extensions and
            // per-record bounds before committing first-device intent.
            Operation {
                format: shep_profile_core::FORMAT.into(),
                major: 1,
                minor: 0,
                requires: vec![
                    "causal-v1".into(),
                    "accounts-v1".into(),
                    "settings-v1".into(),
                ],
                namespace: selection.binding.namespace.clone(),
                profile: selection.binding.profile,
                generation: selection.binding.generation,
                device: selection.binding.profile,
                operation: chunk.operation,
                parents: vec![],
                changes: chunk.changes.clone(),
                extra: Default::default(),
            }
            .encode()?;
            for change in &chunk.changes {
                let target = match &change.action {
                    Action::ProfileName { name } => {
                        anyhow::ensure!(name == &selection.name, "The saved profile name changed.");
                        profile_names += 1;
                        "profile:name".into()
                    }
                    Action::AccountConnection { account } => {
                        connections.insert(account.id);
                        format!("{}:connection", account.id)
                    }
                    Action::AccountName { id, .. } => {
                        names.insert(*id);
                        format!("{id}:name")
                    }
                    Action::Setting { key, .. } => format!("setting:{key:?}"),
                    _ => anyhow::bail!("The initial setup cannot contain removals."),
                };
                anyhow::ensure!(targets.insert(target), "The saved setup repeats a field.");
            }
        }
        let mapped: std::collections::BTreeSet<_> = self.account_ids.values().copied().collect();
        anyhow::ensure!(
            profile_names == 1
                && connections == names
                && connections == mapped
                && self.account_ids.len() == mapped.len()
                && self.account_ids.keys().all(|id| !id.is_empty()),
            "The saved account mappings are incomplete. Keep the original setup for recovery."
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub enabled: bool,
    pub accounts: bool,
    pub settings: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            enabled: false,
            accounts: true,
            settings: true,
        }
    }
}
impl Options {
    pub fn validate(self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.enabled || self.accounts || self.settings,
            "Choose Accounts or Settings before enabling profile sync."
        );
        Ok(())
    }
}

/// Device-local controls change only fields the user actually touched. An older
/// choice cannot overwrite a newer enrollment/Google lifecycle's other fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub enabled: Option<bool>,
    pub accounts: Option<bool>,
    pub settings: Option<bool>,
}
impl Changes {
    pub fn apply(self, mut options: Options) -> Options {
        if let Some(v) = self.enabled {
            options.enabled = v;
        }
        if let Some(v) = self.accounts {
            options.accounts = v;
        }
        if let Some(v) = self.settings {
            options.settings = v;
        }
        options
    }
    pub fn empty(self) -> bool {
        self == Self::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    Create,
    Join,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub binding: history::Binding,
    pub name: String,
    pub origin: Origin,
    /// False until the initial pull/publication/application finishes. Saving a
    /// local choice alone cannot display a confirmed cloud connection.
    pub ready: bool,
}
impl Selection {
    pub fn validate(&self) -> anyhow::Result<()> {
        Binding::new(
            self.binding.principal.clone(),
            self.binding.namespace.clone(),
        )?;
        self.binding.storage_key()?;
        anyhow::ensure!(
            !self.name.trim().is_empty()
                && self.name.len() <= 256
                && !self.name.chars().any(char::is_control),
            "Use a nonempty shared profile name of at most 256 bytes without control characters."
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enrollment {
    pub revision: u64,
    pub options: Options,
    pub selection: Option<Selection>,
    pub last_success: Option<i64>,
}
impl Enrollment {
    pub fn validate(&self) -> anyhow::Result<()> {
        self.options.validate()?;
        anyhow::ensure!(
            self.revision <= i64::MAX as u64,
            "The profile sync revision is invalid."
        );
        if let Some(selection) = &self.selection {
            selection.validate()?;
        }
        anyhow::ensure!(
            !self.options.enabled || self.selection.is_some(),
            "Choose a shared profile before enabling sync."
        );
        Ok(())
    }
    pub(crate) fn advance(&mut self) -> anyhow::Result<()> {
        self.revision = self
            .revision
            .checked_add(1)
            .filter(|v| *v <= i64::MAX as u64)
            .context("The profile sync revision is exhausted")?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub enrollment: Enrollment,
    pub preferences_revision: u64,
    pub connections_revision: u64,
    pub google_revision: u64,
    pub google_identity: String,
    pub available: bool,
    pub accounts: usize,
}

pub(crate) fn google_available(preferences: &Preferences) -> bool {
    !preferences.google_lifecycle.disconnected
        && !preferences.google_lifecycle.cleanup_pending
        && preferences.google_grant.access.drive_allowed()
        && !preferences.active_google_client().is_empty()
        && preferences.google_connection_id.starts_with("drive:")
        && preferences.google_connection_id.len() > 6
}

pub(crate) fn check_google(preferences: &Preferences, selection: &Selection) -> anyhow::Result<()> {
    selection.validate()?;
    anyhow::ensure!(
        google_available(preferences),
        "Connect Google with Drive permission before syncing profiles."
    );
    anyhow::ensure!(
        preferences.google_connection_id == selection.binding.principal,
        "This profile belongs to another Google account. Reconnect its original account or choose another local workspace."
    );
    Ok(())
}
