//! Device-local reconciliation state. Current values are distinct from the last
//! common values so a delayed pull cannot turn an offline edit into an overwrite.
use super::{enrollment::Options, metadata};
use crate::model::{Account, Preferences};
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use shep_profile_core::{Action, Change, history};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub(crate) const STORAGE_KEY: &str = "profile_replication_v1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub local: Option<Change>,
    pub remote: Option<Change>,
    pub revision: u64,
}

/// One immutable native intent at a time. A stalled/conflicting field must not
/// combine unrelated edits into the same unpublishable operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub operation: Uuid,
    pub expected_revision: u64,
    pub local: Change,
    pub change: Change,
}
impl Pending {
    pub fn edit(&self) -> history::LocalEdit {
        history::LocalEdit {
            operation: self.operation,
            expected_revision: self.expected_revision,
            changes: vec![self.change.clone()],
            resolutions: vec![],
        }
    }
    pub fn target(&self) -> String {
        history::target(&self.change.action)
    }
}

/// Minted by the exclusive history owner after checking the actual field. An
/// idempotent Edit reply alone may report a newer state after a lost reply.
#[derive(Clone, Debug)]
pub struct Admitted {
    pub(super) binding: history::Binding,
    pub(super) pending: Pending,
    pub(super) revision: u64,
}
impl Admitted {
    pub(crate) fn into_parts(self) -> (history::Binding, Pending, u64) {
        (self.binding, self.pending, self.revision)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct State {
    pub binding: history::Binding,
    pub revision: u64,
    /// Same local-to-shared orientation as first-device seeds.
    pub accounts: BTreeMap<String, Uuid>,
    /// Accounts already present when joining remain local until explicit linking.
    pub local_only: BTreeSet<String>,
    /// Removing only on this device cannot publish a tombstone or re-add it.
    pub suppressed: BTreeSet<Uuid>,
    pub fields: BTreeMap<String, Field>,
    pub pending: Option<Pending>,
}
impl State {
    pub fn new(
        binding: history::Binding,
        revision: u64,
        accounts: &[Account],
        prefs: &Preferences,
        mapping: BTreeMap<String, Uuid>,
        common: Vec<Change>,
    ) -> anyhow::Result<Self> {
        let local_only = accounts
            .iter()
            .filter(|a| !mapping.contains_key(&a.id))
            .map(|a| a.id.clone())
            .collect();
        let mut state = Self {
            binding,
            revision,
            accounts: mapping,
            local_only,
            suppressed: Default::default(),
            fields: Default::default(),
            pending: None,
        };
        for (target, local) in state.values(accounts, prefs)? {
            state.fields.insert(
                target,
                Field {
                    local: Some(local),
                    remote: None,
                    revision,
                },
            );
        }
        let mut seen = BTreeSet::new();
        for change in common {
            let target = history::target(&change.action);
            ensure!(
                seen.insert(target.clone()),
                "The common profile values repeat a field."
            );
            let mut local = change.clone();
            local.extra.clear();
            if let Action::AccountConnection { account } = &mut local.action {
                account.extra.clear();
            }
            if let Action::SettingRemoved { key } = local.action {
                local.action = Action::Setting {
                    key,
                    value: metadata::setting_value(key, &Preferences::default())
                        .context("The reset setting is not supported here")?,
                };
            }
            state.fields.insert(
                target,
                Field {
                    local: Some(local),
                    remote: Some(change),
                    revision,
                },
            );
        }
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        self.binding.storage_key()?;
        ensure!(
            self.revision <= i64::MAX as u64,
            "The saved profile checkpoint is invalid."
        );
        let ids: BTreeSet<_> = self.accounts.values().collect();
        ensure!(
            ids.len() == self.accounts.len()
                && ids.iter().all(|id| !id.is_nil())
                && self
                    .accounts
                    .keys()
                    .all(|id| !id.is_empty() && !self.local_only.contains(id))
                && self.suppressed.iter().all(|id| ids.contains(id)),
            "The saved shared account mapping is ambiguous."
        );
        for (target, field) in &self.fields {
            ensure!(
                field.revision <= self.revision,
                "The saved profile field is ahead of its checkpoint."
            );
            for change in field.local.iter().chain(field.remote.iter()) {
                ensure!(
                    &history::target(&change.action) == target,
                    "The saved profile field identity changed."
                );
                self.validate_change(change)?;
            }
        }
        if let Some(pending) = &self.pending {
            ensure!(
                !pending.operation.is_nil()
                    && pending.expected_revision <= self.revision
                    && pending.target() == history::target(&pending.local.action)
                    && pending.local.extra.is_empty(),
                "The saved profile edit has invalid identity or revision."
            );
            let mut normalized = pending.change.clone();
            normalized.extra.clear();
            if let Action::AccountConnection { account } = &mut normalized.action {
                account.extra.clear();
            }
            ensure!(
                normalized == pending.local,
                "The saved local intent differs from its shared edit."
            );
            self.validate_change(&pending.change)?;
        }
        Ok(())
    }

    fn validate_change(&self, change: &Change) -> anyhow::Result<()> {
        // Validate portable values, optional extensions and per-record guards.
        shep_profile_core::Operation {
            format: shep_profile_core::FORMAT.into(),
            major: 1,
            minor: 0,
            requires: vec![
                "causal-v1".into(),
                "accounts-v1".into(),
                "settings-v1".into(),
            ],
            namespace: self.binding.namespace.clone(),
            profile: self.binding.profile,
            generation: self.binding.generation,
            device: self.binding.profile,
            operation: self.binding.generation,
            parents: vec![],
            changes: vec![change.clone()],
            extra: Default::default(),
        }
        .encode()?;
        Ok(())
    }

    pub fn values(
        &self,
        accounts: &[Account],
        prefs: &Preferences,
    ) -> anyhow::Result<BTreeMap<String, Change>> {
        let mut values = BTreeMap::new();
        for key in metadata::SETTINGS {
            let change = Change {
                action: Action::Setting {
                    key: *key,
                    value: metadata::setting_value(*key, prefs)
                        .context("The native setting is unavailable")?,
                },
                extra: Default::default(),
            };
            values.insert(history::target(&change.action), change);
        }
        for account in accounts {
            let Some(shared) = self.accounts.get(&account.id) else {
                continue;
            };
            if self.suppressed.contains(shared) {
                continue;
            }
            let mut portable = account.clone();
            portable.id = shared.to_string();
            for change in metadata::export_account(&portable, *shared)? {
                values.insert(history::target(&change.action), change);
            }
        }
        Ok(values)
    }

    pub fn capture(
        &mut self,
        accounts: &[Account],
        prefs: &Preferences,
        options: Options,
    ) -> anyhow::Result<Option<Pending>> {
        self.validate()?;
        if !options.enabled {
            return Ok(None);
        }
        if let Some(pending) = &self.pending {
            return Ok(allowed(&pending.change, options).then(|| pending.clone()));
        }
        if options.accounts {
            let local: BTreeSet<_> = accounts.iter().map(|a| a.id.as_str()).collect();
            for (id, shared) in &self.accounts {
                if !local.contains(id.as_str()) {
                    self.suppressed.insert(*shared);
                }
            }
            for account in accounts {
                if !self.accounts.contains_key(&account.id)
                    && !self.local_only.contains(&account.id)
                {
                    let shared = Uuid::new_v4();
                    self.accounts.insert(account.id.clone(), shared);
                }
            }
        }
        for (target, local) in self.values(accounts, prefs)? {
            if !allowed(&local, options) {
                continue;
            }
            let field = self.fields.get(&target);
            if field.and_then(|f| f.local.as_ref()) == Some(&local) {
                continue;
            }
            let mut change = local.clone();
            if let Some(remote) = field.and_then(|f| f.remote.as_ref()) {
                change.extra = remote.extra.clone();
                if let (
                    Action::AccountConnection { account: old },
                    Action::AccountConnection { account: new },
                ) = (&remote.action, &mut change.action)
                {
                    new.extra = old.extra.clone();
                }
            }
            let pending = Pending {
                operation: Uuid::new_v4(),
                expected_revision: field.map_or(self.revision, |f| f.revision),
                local,
                change,
            };
            self.pending = Some(pending.clone());
            self.validate()?;
            return Ok(Some(pending));
        }
        Ok(None)
    }

    /// Called only after the history owner acknowledges the exact saved edit.
    /// The current SQL preference may already be newer; do not read it back here.
    pub fn acknowledge(&mut self, pending: &Pending, revision: u64) -> anyhow::Result<()> {
        ensure!(
            self.pending.as_ref() == Some(pending),
            "The saved profile edit changed before its acknowledgment."
        );
        ensure!(
            revision >= pending.expected_revision,
            "The profile edit acknowledgment is older than its basis."
        );
        self.revision = self.revision.max(revision);
        self.fields.insert(
            pending.target(),
            Field {
                local: Some(pending.local.clone()),
                remote: Some(pending.change.clone()),
                revision,
            },
        );
        self.pending = None;
        self.validate()
    }
}

pub fn allowed(change: &Change, options: Options) -> bool {
    options.enabled
        && match change.action {
            Action::AccountConnection { .. }
            | Action::AccountName { .. }
            | Action::AccountRemoved { .. } => options.accounts,
            Action::Setting { .. } | Action::SettingRemoved { .. } => options.settings,
            _ => true,
        }
}
