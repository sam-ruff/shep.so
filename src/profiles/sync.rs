//! Persistent device synchronization state. Local edits and remote application
//! receipts stay in the mail database; immutable operations stay in history.
pub mod control;
pub mod resolution;
pub mod runner;
use serde::{Deserialize, Serialize};
use shep_profile_core::{Change, SettingKey, history::Binding};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subscription {
    pub binding: Binding,
    pub device: Uuid,
    pub name: String,
    pub enabled: bool,
    pub revision: u64,
    pub history_revision: u64,
    pub remote_cursor: u64,
    pub remote_device: Option<Uuid>,
    pub last_synced: Option<i64>,
    pub pending: u64,
    pub conflicts: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Seed {
    pub binding: Binding,
    pub device: Uuid,
    pub name: String,
    pub history_revision: u64,
    /// Revisions captured by the completed review; checked atomically at setup.
    #[serde(default)]
    pub baseline: Option<BTreeMap<SettingKey, u64>>,
    /// Explicit newer/kept local intent, including a reverted value.
    #[serde(default)]
    pub local_intent: BTreeSet<SettingKey>,
    /// Only preferences accepted in the completed publication/enrollment review.
    /// None represents a field absent from that known history.
    pub fields: BTreeMap<SettingKey, Option<Change>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingEdit {
    pub binding: Binding,
    pub key: SettingKey,
    pub operation: Uuid,
    pub local_revision: u64,
    pub request: shep_profile_core::history::LocalEdit,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Field {
    pub key: SettingKey,
    pub enabled: bool,
    pub shared: Option<Change>,
    pub shared_revision: u64,
    pub local_revision: u64,
    pub pending: bool,
    pub local: serde_json::Value,
    pub incoming: Option<Change>,
    pub incoming_revision: Option<u64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ApplyResult {
    Applied(Box<crate::store::PreferenceSnapshot>),
    Unchanged,
    ReviewRequired,
}
