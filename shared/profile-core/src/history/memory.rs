//! Pure in-memory causal history with the same command contract and derived
//! state as the native SQLite journal. Clients that cannot use SQLite (the
//! browser worker) persist the stored records themselves and restore them in
//! sequence order; every command is atomic, so a failed command leaves the
//! journal exactly as it was.
use super::*;
use crate::{Action, Change, Operation};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// One durable record as the owning client stores it. `request` is present
/// only for local edits; `raw` is the exact immutable operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredRecord {
    pub seq: u64,
    pub operation: Uuid,
    pub raw: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(default)]
    pub uploaded: bool,
}

#[derive(Clone, Debug)]
struct Stored {
    seq: u64,
    device: Uuid,
    raw: Vec<u8>,
    sha256: String,
    request: Option<Vec<u8>>,
    parents: Vec<Uuid>,
    applied: bool,
    remaining: usize,
    uploaded: bool,
    file_id: Option<String>,
}
#[derive(Clone, Debug, Default)]
struct Target {
    account: Option<Uuid>,
    versions: BTreeMap<Uuid, usize>,
    visible: bool,
    revision: u64,
}
#[derive(Clone, Debug, Default)]
struct Counters {
    revision: u64,
    operations: u64,
    waiting: u64,
    ready: u64,
    queued: u64,
    fields: u64,
    conflicts: u64,
    removed: bool,
}

#[derive(Clone, Debug)]
pub struct MemoryJournal {
    binding: Binding,
    device: Uuid,
    next_seq: u64,
    operations: BTreeMap<Uuid, Stored>,
    sequence: BTreeMap<u64, Uuid>,
    children: BTreeMap<Uuid, BTreeSet<Uuid>>,
    heads: BTreeSet<Uuid>,
    removed_accounts: BTreeSet<Uuid>,
    targets: BTreeMap<String, Target>,
    file_ids: BTreeMap<String, Uuid>,
    counters: Counters,
}

impl MemoryJournal {
    /// A fresh journal for this device. The caller supplies the device UUID it
    /// generated and keeps; it must never be copied from another installation.
    pub fn new(binding: Binding, device: Uuid) -> Result<Self> {
        binding.validate()?;
        if device.is_nil() {
            return Err(Error::Binding);
        }
        Ok(Self {
            binding,
            device,
            next_seq: 1,
            operations: BTreeMap::new(),
            sequence: BTreeMap::new(),
            children: BTreeMap::new(),
            heads: BTreeSet::new(),
            removed_accounts: BTreeSet::new(),
            targets: BTreeMap::new(),
            file_ids: BTreeMap::new(),
            counters: Counters::default(),
        })
    }
    /// Rebuild from records the client persisted, in their stored sequence.
    /// Every record is applied through the ordinary import/insert path, so a
    /// record that no longer validates fails the whole restore.
    pub fn restore(
        binding: Binding,
        device: Uuid,
        records: impl IntoIterator<Item = StoredRecord>,
    ) -> Result<Self> {
        let mut journal = Self::new(binding, device)?;
        let mut last = 0;
        for record in records {
            if record.seq <= last {
                return Err(Error::Storage);
            }
            last = record.seq;
            let operation = Operation::decode(record.raw.as_bytes())?;
            journal.check_binding(&operation)?;
            if operation.operation != record.operation {
                return Err(Error::Identity);
            }
            if record.request.is_some() && operation.device != journal.device {
                return Err(Error::Binding);
            }
            journal.next_seq = record.seq;
            let mut attempt = journal.clone();
            attempt.insert(
                &operation,
                record.raw.as_bytes(),
                record.request.as_deref().map(str::as_bytes),
            )?;
            if let Some(file_id) = record.file_id {
                if !valid_file_id(&file_id) || attempt.file_ids.contains_key(&file_id) {
                    return Err(Error::Identity);
                }
                attempt.file_ids.insert(file_id.clone(), record.operation);
                let stored = attempt
                    .operations
                    .get_mut(&record.operation)
                    .ok_or(Error::Storage)?;
                stored.file_id = Some(file_id);
                if record.uploaded {
                    stored.uploaded = true;
                    attempt.counters.queued -= 1;
                }
            } else if record.uploaded {
                return Err(Error::Identity);
            }
            while attempt.counters.ready > 0 {
                attempt.apply_ready()?;
            }
            journal = attempt;
        }
        Ok(journal)
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn device(&self) -> Uuid {
        self.device
    }
    /// The durable form of one operation for the owning client's store.
    pub fn record(&self, operation: Uuid) -> Result<StoredRecord> {
        let stored = self.operations.get(&operation).ok_or(Error::Changed)?;
        Ok(StoredRecord {
            seq: stored.seq,
            operation,
            raw: String::from_utf8(stored.raw.clone()).map_err(|_| Error::Storage)?,
            request: stored
                .request
                .as_ref()
                .map(|r| String::from_utf8(r.clone()).map_err(|_| Error::Storage))
                .transpose()?,
            file_id: stored.file_id.clone(),
            uploaded: stored.uploaded,
        })
    }
    pub fn execute(&mut self, command: Command) -> Result<Reply> {
        match command {
            Command::State => Ok(Reply::State(self.state())),
            Command::Import { record } => self.import(record.as_bytes()).map(Reply::State),
            Command::Edit { edit } => self.edit(edit).map(Reply::State),
            Command::Drain => self.drain().map(Reply::State),
            Command::Fields { after } => Ok(Reply::Fields(self.fields(after.as_deref()))),
            Command::Versions { target, after } => {
                Ok(Reply::Versions(self.versions(&target, after)))
            }
            Command::Value { target, operation } => {
                self.value(&target, operation).map(Reply::Value)
            }
            Command::ExportRecord {
                expected_revision,
                after,
            } => self
                .export_kind(expected_revision, after, false)
                .map(Reply::Record),
            Command::ExportAcknowledgedRecord {
                expected_revision,
                after,
            } => self
                .export_kind(expected_revision, after, true)
                .map(Reply::Record),
            Command::NextUpload => Ok(Reply::Upload(self.next_upload()?)),
            Command::Reserve { operation, file_id } => {
                self.reserve(operation, &file_id).map(Reply::State)
            }
            Command::Confirm {
                operation,
                file_id,
                sha256,
            } => self.confirm(operation, &file_id, &sha256).map(Reply::State),
        }
    }
    fn atomic(&mut self, work: impl FnOnce(&mut Self) -> Result<()>) -> Result<State> {
        let mut attempt = self.clone();
        work(&mut attempt)?;
        *self = attempt;
        Ok(self.state())
    }
    pub fn state(&self) -> State {
        let c = &self.counters;
        let initialized = !c.removed
            && c.waiting == 0
            && c.ready == 0
            && self.setup_state().is_some_and(|(complete, _, _)| complete)
            && self
                .targets
                .get("profile:setup")
                .is_some_and(|t| t.visible && t.versions.len() == 1);
        State {
            device: self.device,
            revision: c.revision,
            operations: c.operations,
            waiting: c.waiting,
            ready: c.ready,
            queued: c.queued,
            fields: c.fields,
            conflicts: c.conflicts,
            removed: c.removed,
            initialized,
        }
    }
    pub fn overview(&self) -> Result<Overview> {
        let names = self.versions("profile:name", None);
        let name = if names.len() == 1 {
            match self.value("profile:name", names[0].operation)?.action {
                Action::ProfileName { name } => Some(name),
                _ => return Err(Error::Storage),
            }
        } else {
            None
        };
        let visible = |prefix: &str, suffix: Option<&str>| {
            self.targets
                .iter()
                .filter(|(k, t)| {
                    t.visible && k.starts_with(prefix) && suffix.is_none_or(|s| k.ends_with(s))
                })
                .count() as u64
        };
        Ok(Overview {
            state: self.state(),
            name,
            name_conflict: names.len() > 1,
            accounts: visible("account:", Some(":connection")),
            settings: visible("setting:", None),
        })
    }
    pub fn fields(&self, after: Option<&str>) -> Vec<Field> {
        let after = after.unwrap_or("");
        self.targets
            .iter()
            .filter(|(k, t)| t.visible && k.as_str() > after)
            .take(PAGE_SIZE)
            .map(|(k, t)| Field {
                target: k.clone(),
                versions: t.versions.len() as u64,
                conflict: conflicting(k, t.versions.len() as u64),
                revision: t.revision,
            })
            .collect()
    }
    pub fn versions(&self, target: &str, after: Option<Uuid>) -> Vec<Version> {
        let Some(t) = self.targets.get(target).filter(|t| t.visible) else {
            return Vec::new();
        };
        t.versions
            .keys()
            .filter(|id| after.is_none_or(|after| **id > after))
            .take(PAGE_SIZE)
            .filter_map(|id| {
                self.operations.get(id).map(|op| Version {
                    operation: *id,
                    device: op.device,
                })
            })
            .collect()
    }
    pub fn value(&self, target: &str, operation: Uuid) -> Result<Change> {
        let position = *self
            .targets
            .get(target)
            .filter(|t| t.visible)
            .and_then(|t| t.versions.get(&operation))
            .ok_or(Error::Changed)?;
        let stored = self.operations.get(&operation).ok_or(Error::Storage)?;
        Operation::decode(&stored.raw)?
            .changes
            .into_iter()
            .nth(position)
            .ok_or(Error::Storage)
    }
    pub fn next_upload(&self) -> Result<Option<Upload>> {
        self.sequence
            .values()
            .filter_map(|id| self.operations.get(id).map(|op| (*id, op)))
            .find(|(_, op)| op.request.is_some() && !op.uploaded)
            .map(|(id, op)| {
                Ok(Upload {
                    operation: id,
                    record: String::from_utf8(op.raw.clone()).map_err(|_| Error::Storage)?,
                    sha256: op.sha256.clone(),
                    file_id: op.file_id.clone(),
                })
            })
            .transpose()
    }
    fn export_kind(
        &self,
        expected_revision: u64,
        after: u64,
        acknowledged: bool,
    ) -> Result<Option<Record>> {
        let state = self.state();
        if state.revision != expected_revision {
            return Err(Error::Changed);
        }
        if !state.initialized {
            return Err(Error::Incomplete);
        }
        self.sequence
            .range(after.saturating_add(1)..)
            .filter_map(|(seq, id)| self.operations.get(id).map(|op| (*seq, *id, op)))
            .find(|(_, _, op)| !acknowledged || op.request.is_none() || op.uploaded)
            .map(|(position, operation, op)| {
                Ok(Record {
                    position,
                    operation,
                    record: String::from_utf8(op.raw.clone()).map_err(|_| Error::Storage)?,
                })
            })
            .transpose()
    }
    pub fn import(&mut self, raw: &[u8]) -> Result<State> {
        let operation = Operation::decode(raw)?;
        self.check_binding(&operation)?;
        let raw = raw.to_vec();
        self.atomic(move |journal| {
            if let Some(previous) = journal.operations.get(&operation.operation) {
                if previous.raw != raw {
                    return Err(Error::Identity);
                }
            } else {
                journal.insert(&operation, &raw, None)?;
            }
            journal.apply_ready()
        })
    }
    pub fn drain(&mut self) -> Result<State> {
        self.atomic(|journal| journal.apply_ready())
    }
    pub fn edit(&mut self, edit: LocalEdit) -> Result<State> {
        if edit.operation.is_nil() || edit.resolutions.len() > crate::MAX_CHANGES {
            return Err(crate::Error::Invalid.into());
        }
        let request = crate::json::encode(&edit)?;
        if let Some(previous) = self.operations.get(&edit.operation) {
            if previous.request.as_deref() != Some(request.as_slice()) {
                return Err(Error::Identity);
            }
            return Ok(self.state());
        }
        let c = &self.counters;
        if c.revision < edit.expected_revision {
            return Err(Error::Changed);
        }
        if c.removed {
            return Err(Error::Removed);
        }
        if c.waiting != 0 {
            return Err(Error::Incomplete);
        }
        if self.heads.len() > crate::MAX_PARENTS {
            return Err(Error::Heads);
        }
        let parents: Vec<Uuid> = self.heads.iter().copied().collect();
        let setup = self.setup_state();
        if setup.is_some_and(|(complete, device, _)| !complete && device != self.device) {
            return Err(Error::Incomplete);
        }
        let mut requires = vec![
            "causal-v1".to_owned(),
            "accounts-v1".to_owned(),
            "settings-v1".to_owned(),
        ];
        if setup.is_some()
            || edit
                .changes
                .iter()
                .any(|c| matches!(c.action, Action::ProfileSetup { .. }))
        {
            requires.push("initialization-v1".into());
        }
        let operation = Operation {
            format: crate::FORMAT.into(),
            major: 1,
            minor: 0,
            requires,
            namespace: self.binding.namespace.clone(),
            profile: self.binding.profile,
            generation: self.binding.generation,
            device: self.device,
            operation: edit.operation,
            parents,
            changes: edit.changes,
            extra: Default::default(),
        };
        let raw = operation.encode()?;
        let mut reviewed = BTreeSet::new();
        for resolution in &edit.resolutions {
            if resolution.versions.len() > crate::MAX_PARENTS
                || !reviewed.insert(resolution.target.clone())
                || !operation
                    .changes
                    .iter()
                    .any(|c| target(&c.action) == resolution.target)
            {
                return Err(Error::Conflict);
            }
            let actual = self.version_ids(&resolution.target)?;
            let supplied: BTreeSet<_> = resolution.versions.iter().copied().collect();
            if actual.len() <= 1
                || supplied.len() != resolution.versions.len()
                || supplied != actual
            {
                return Err(Error::Changed);
            }
        }
        for change in &operation.changes {
            if account_id(&change.action).is_some_and(|id| self.removed_accounts.contains(&id)) {
                return Err(Error::Removed);
            }
            let key = target(&change.action);
            let changed = self.targets.get(&key).map(|t| t.revision).unwrap_or(0);
            if changed > edit.expected_revision {
                return Err(Error::Changed);
            }
            let versions = self.version_ids(&key)?;
            if versions.len() > 1 && !reviewed.contains(&key) {
                return Err(Error::Conflict);
            }
            if !versions.is_empty()
                && !matches!(
                    change.action,
                    Action::SettingRemoved { .. }
                        | Action::AccountRemoved { .. }
                        | Action::ProfileRemoved
                )
            {
                let mut preserved = false;
                for version in versions {
                    let position = *self
                        .targets
                        .get(&key)
                        .and_then(|t| t.versions.get(&version))
                        .ok_or(Error::Storage)?;
                    let stored = self.operations.get(&version).ok_or(Error::Storage)?;
                    let prior = Operation::decode(&stored.raw)?
                        .changes
                        .into_iter()
                        .nth(position)
                        .ok_or(Error::Storage)?;
                    if preserves_extensions(&prior, change) {
                        preserved = true;
                        break;
                    }
                }
                if !preserved {
                    return Err(crate::Error::Upgrade.into());
                }
            }
        }
        self.atomic(move |journal| {
            journal.insert(&operation, &raw, Some(&request))?;
            journal.apply_ready()
        })
    }
    pub fn reserve(&mut self, operation: Uuid, file_id: &str) -> Result<State> {
        if !valid_file_id(file_id) {
            return Err(Error::Identity);
        }
        let stored = self
            .operations
            .get(&operation)
            .filter(|op| op.request.is_some())
            .ok_or(Error::Changed)?;
        if stored.file_id.as_deref().is_some_and(|old| old != file_id) {
            return Err(Error::Identity);
        }
        if stored.file_id.is_none() {
            if self.file_ids.contains_key(file_id) {
                return Err(Error::Storage);
            }
            self.file_ids.insert(file_id.to_owned(), operation);
            if let Some(op) = self.operations.get_mut(&operation) {
                op.file_id = Some(file_id.to_owned());
            }
            self.counters.revision += 1;
        }
        Ok(self.state())
    }
    /// Only call after transport verifies this exact immutable file and digest.
    pub fn confirm(&mut self, operation: Uuid, file_id: &str, sha256: &str) -> Result<State> {
        let stored = self
            .operations
            .get_mut(&operation)
            .filter(|op| {
                op.request.is_some()
                    && op.file_id.as_deref() == Some(file_id)
                    && op.sha256 == sha256
            })
            .ok_or(Error::Identity)?;
        if !stored.uploaded {
            stored.uploaded = true;
            self.counters.queued -= 1;
            self.counters.revision += 1;
        }
        Ok(self.state())
    }
    fn check_binding(&self, op: &Operation) -> Result<()> {
        if op.namespace != self.binding.namespace
            || op.profile != self.binding.profile
            || op.generation != self.binding.generation
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
    fn version_ids(&self, key: &str) -> Result<BTreeSet<Uuid>> {
        let ids: BTreeSet<_> = self
            .targets
            .get(key)
            .map(|t| t.versions.keys().copied().collect())
            .unwrap_or_default();
        if ids.len() > crate::MAX_PARENTS {
            return Err(Error::Heads);
        }
        Ok(ids)
    }
    fn ancestry(&self, operation: Uuid) -> BTreeSet<Uuid> {
        let mut seen = BTreeSet::new();
        let mut frontier: Vec<Uuid> = self
            .operations
            .get(&operation)
            .map(|op| op.parents.clone())
            .unwrap_or_default();
        while let Some(id) = frontier.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(op) = self.operations.get(&id) {
                frontier.extend(op.parents.iter().copied());
            }
        }
        seen
    }
    fn insert(&mut self, op: &Operation, raw: &[u8], request: Option<&[u8]>) -> Result<()> {
        if self.operations.contains_key(&op.operation) {
            return Err(Error::Storage);
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        self.operations.insert(
            op.operation,
            Stored {
                seq,
                device: op.device,
                raw: raw.to_vec(),
                sha256: format!("{:x}", Sha256::digest(raw)),
                request: request.map(<[u8]>::to_vec),
                parents: op.parents.clone(),
                applied: false,
                remaining: 0,
                uploaded: false,
                file_id: None,
            },
        );
        self.sequence.insert(seq, op.operation);
        for parent in &op.parents {
            self.children
                .entry(*parent)
                .or_default()
                .insert(op.operation);
        }
        if self.ancestry(op.operation).contains(&op.operation) {
            return Err(Error::Cycle);
        }
        let remaining = op
            .parents
            .iter()
            .filter(|p| !self.operations.get(p).is_some_and(|parent| parent.applied))
            .count();
        if let Some(stored) = self.operations.get_mut(&op.operation) {
            stored.remaining = remaining;
        }
        let c = &mut self.counters;
        c.revision += 1;
        c.operations += 1;
        c.waiting += 1;
        c.ready += u64::from(remaining == 0);
        c.queued += u64::from(request.is_some());
        Ok(())
    }
    fn apply_ready(&mut self) -> Result<()> {
        for _ in 0..APPLY_BATCH {
            let Some(id) = self
                .sequence
                .values()
                .find(|id| {
                    self.operations
                        .get(id)
                        .is_some_and(|op| !op.applied && op.remaining == 0)
                })
                .copied()
            else {
                break;
            };
            let stored = self.operations.get(&id).ok_or(Error::Storage)?;
            let op = Operation::decode(&stored.raw)?;
            let ancestors = self.ancestry(id);
            for (position, change) in op.changes.iter().enumerate() {
                self.apply_change(&op, position, &change.action, &ancestors)?;
            }
            for parent in &op.parents {
                self.heads.remove(parent);
            }
            self.heads.insert(id);
            if let Some(stored) = self.operations.get_mut(&id) {
                stored.applied = true;
            }
            let mut ready = 0;
            for child in self.children.get(&id).cloned().unwrap_or_default() {
                if let Some(child) = self.operations.get_mut(&child)
                    && !child.applied
                {
                    if child.remaining == 1 {
                        ready += 1;
                    }
                    child.remaining -= 1;
                }
            }
            let c = &mut self.counters;
            c.waiting -= 1;
            c.ready = c.ready - 1 + ready;
            c.revision += 1;
        }
        Ok(())
    }
    fn setup_state(&self) -> Option<(bool, Uuid, Uuid)> {
        let target = self.targets.get("profile:setup")?;
        let (operation, position) = target.versions.iter().next()?;
        let stored = self.operations.get(operation)?;
        let op = Operation::decode(&stored.raw).ok()?;
        match op.changes.get(*position)?.action {
            Action::ProfileSetup { complete } => Some((complete, op.device, *operation)),
            _ => None,
        }
    }
    fn apply_change(
        &mut self,
        op: &Operation,
        position: usize,
        action: &Action,
        ancestors: &BTreeSet<Uuid>,
    ) -> Result<()> {
        if self.counters.removed && !matches!(action, Action::ProfileRemoved) {
            return Ok(());
        }
        if let Action::ProfileRemoved = action {
            if !self.counters.removed {
                for t in self.targets.values_mut() {
                    t.visible = false;
                }
                self.counters.removed = true;
                self.counters.fields = 0;
                self.counters.conflicts = 0;
            }
        } else if let Some(account) = account_id(action) {
            if matches!(action, Action::AccountRemoved { .. }) {
                self.removed_accounts.insert(account);
                let own = target(action);
                let keys: Vec<String> = self
                    .targets
                    .iter()
                    .filter(|(k, t)| t.account == Some(account) && t.visible && **k != own)
                    .map(|(k, _)| k.clone())
                    .collect();
                for key in keys {
                    self.hide(&key);
                }
            } else if self.removed_accounts.contains(&account) {
                return Ok(());
            }
        }
        if let Action::ProfileSetup { complete } = action {
            match (complete, self.setup_state()) {
                (false, None) if op.parents.is_empty() => {
                    if self.operations.values().any(|o| o.applied) {
                        return Err(Error::Identity);
                    }
                }
                (true, Some((false, device, root))) if device == op.device => {
                    if !ancestors.contains(&root) {
                        return Err(Error::Identity);
                    }
                }
                _ => return Err(Error::Identity),
            }
        }
        let key = target(action);
        let next_revision = self.counters.revision + 1;
        let t = self.targets.entry(key.clone()).or_insert_with(|| Target {
            account: account_id(action),
            ..Target::default()
        });
        let old = t.versions.len() as u64;
        let visible = t.visible;
        t.versions.retain(|id, _| !ancestors.contains(id));
        t.versions.insert(op.operation, position);
        let count = t.versions.len() as u64;
        t.visible = true;
        t.revision = next_revision;
        let c = &mut self.counters;
        c.fields += u64::from(!visible);
        let after = u64::from(conflicting(&key, count));
        let before = u64::from(visible && conflicting(&key, old));
        c.conflicts = c.conflicts + after - before;
        Ok(())
    }
    fn hide(&mut self, key: &str) {
        let next_revision = self.counters.revision + 1;
        let Some(t) = self.targets.get_mut(key) else {
            return;
        };
        if !t.visible {
            return;
        }
        let count = t.versions.len() as u64;
        t.visible = false;
        t.revision = next_revision;
        self.counters.fields -= 1;
        self.counters.conflicts -= u64::from(conflicting(key, count));
    }
}
