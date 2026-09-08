//! Portable profile metadata. No network, storage, credential or mail-action API.
//! Decoding is only structural validation: enrollment must separately verify the
//! Google identity/namespace, causal ancestry, immutable IDs and local revisions.
pub mod account;
mod json;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{collections::BTreeSet, fmt};
use uuid::Uuid;

pub const FORMAT: &str = "so.shep.profile-operation";
/// Per-record transport/parse bound, not a limit on the paged profile history.
pub const MAX_RECORD_BYTES: usize = 1024 * 1024;
pub const MAX_CHANGES: usize = 64;
pub const MAX_PARENTS: usize = 256;
const CAPABILITIES: &[&str] = &["causal-v1", "accounts-v1", "settings-v1"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    TooLarge,
    Upgrade,
    LocalData,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "This profile record is invalid. Keep the local setup and retry discovery.",
            Self::TooLarge => "This profile record exceeds the supported record size. Update Shep before syncing it.",
            Self::Upgrade => "This profile uses an unsupported version or capability. Update Shep before syncing it.",
            Self::LocalData => "Device state or credentials cannot be included in this profile metadata record.",
        })
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

/// Unknown optional fields remain in the returned JSON. They must not be
/// converted through local Preferences/Account serialization and discarded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub format: String,
    pub major: u16,
    pub minor: u16,
    pub requires: Vec<String>,
    pub namespace: String,
    pub profile: Uuid,
    pub generation: Uuid,
    pub device: Uuid,
    pub operation: Uuid,
    pub parents: Vec<Uuid>,
    pub changes: Vec<Change>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Change {
    #[serde(flatten)]
    pub action: Action,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl<'de> Deserialize<'de> for Change {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let mut extra = Map::<String, Value>::deserialize(deserializer)?;
        let action: Action = serde_json::from_value(Value::Object(extra.clone()))
            .map_err(serde::de::Error::custom)?;
        // Two derived flatten deserializers cannot distinguish enum fields from
        // extensions: explicitly consume known fields once, retaining only extras.
        extra.remove("kind");
        let fields: &[&str] = match &action {
            Action::AccountConnection { .. } => &["account"],
            Action::AccountName { .. } => &["id", "name"],
            Action::AccountRemoved { .. } => &["id"],
            Action::Setting { .. } => &["key", "value"],
            Action::SettingRemoved { .. } => &["key"],
            Action::ProfileName { .. } => &["name"],
            Action::ProfileRemoved => &[],
        };
        for field in fields {
            extra.remove(*field);
        }
        Ok(Self { action, extra })
    }
}

/// One operation may change distinct fields atomically. Connection settings stay
/// together; name/settings edits have independent targets for causal merging.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    AccountConnection { account: account::Connection },
    AccountName { id: Uuid, name: String },
    AccountRemoved { id: Uuid },
    Setting { key: SettingKey, value: Value },
    SettingRemoved { key: SettingKey },
    ProfileName { name: String },
    ProfileRemoved,
}

/// Freeze supported portable fields explicitly. New fields require a codec
/// update/capability; never accept an arbitrary local preference path as a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingKey {
    Appearance,
    ReplyDisplay,
    ImagePolicy,
    UnifiedInbox,
    CrossAccountMoves,
    GroupConversations,
    DesktopBadges,
    PreviewLines,
}

impl SettingKey {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Appearance => value
                .as_str()
                .is_some_and(|s| ["Light", "Dark", "System"].contains(&s)),
            Self::ReplyDisplay => value
                .as_str()
                .is_some_and(|s| ["Collapsed", "Expanded", "LatestOnly"].contains(&s)),
            Self::ImagePolicy => value
                .as_str()
                .is_some_and(|s| ["BlockAll", "Contacts", "AllowAll"].contains(&s)),
            Self::PreviewLines => value.as_u64().is_some_and(|n| n <= 4),
            _ => value.is_boolean(),
        }
    }
}

impl Operation {
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(Error::TooLarge);
        }
        let raw = json::decode(bytes)?;
        // Version/capability errors must not fall back to a default empty record.
        if raw.get("format").and_then(Value::as_str) != Some(FORMAT)
            || raw.get("major").and_then(Value::as_u64) != Some(1)
        {
            return Err(Error::Upgrade);
        }
        let required = raw
            .get("requires")
            .and_then(Value::as_array)
            .ok_or(Error::Invalid)?;
        if required
            .iter()
            .any(|v| !v.as_str().is_some_and(|s| CAPABILITIES.contains(&s)))
        {
            return Err(Error::Upgrade);
        }
        json::portable(&raw)?;
        let result: Self = serde_json::from_value(raw).map_err(|_| Error::Upgrade)?;
        result.validate()?;
        Ok(result)
    }

    /// Revalidation also rejects duplicate field names introduced by a caller's
    /// extension map. Do not emit a half-valid record or drop an optional field.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = json::encode(self)?;
        Self::decode(&bytes)?;
        Ok(bytes)
    }

    fn validate(&self) -> Result<()> {
        json::portable_map(&self.extra)?;
        if self.format != FORMAT || self.major != 1 {
            return Err(Error::Upgrade);
        }
        if self
            .requires
            .iter()
            .any(|s| !CAPABILITIES.contains(&s.as_str()))
        {
            return Err(Error::Upgrade);
        }
        let required: BTreeSet<_> = self.requires.iter().map(String::as_str).collect();
        if required.len() != self.requires.len() || !required.contains("causal-v1") {
            return Err(Error::Invalid);
        }
        if !namespace(&self.namespace)
            || [self.profile, self.generation, self.device, self.operation]
                .iter()
                .any(Uuid::is_nil)
            || self.parents.len() > MAX_PARENTS
            || self.changes.is_empty()
            || self.changes.len() > MAX_CHANGES
        {
            return Err(Error::Invalid);
        }
        let parents: BTreeSet<_> = self.parents.iter().collect();
        if parents.len() != self.parents.len()
            || self
                .parents
                .iter()
                .any(|id| id.is_nil() || *id == self.operation)
        {
            return Err(Error::Invalid);
        }
        let mut targets = BTreeSet::new();
        let mut accounts = BTreeSet::new();
        let mut removed = BTreeSet::new();
        for change in &self.changes {
            json::portable_map(&change.extra)?;
            let (target, capability) = match &change.action {
                Action::AccountConnection { account } => {
                    account.validate()?;
                    accounts.insert(account.id);
                    (format!("{}:connection", account.id), "accounts-v1")
                }
                Action::AccountName { id, name } => {
                    if id.is_nil() || !text(name, 256, false) {
                        return Err(Error::Invalid);
                    }
                    accounts.insert(*id);
                    (format!("{id}:name"), "accounts-v1")
                }
                Action::AccountRemoved { id } => {
                    if id.is_nil() {
                        return Err(Error::Invalid);
                    }
                    removed.insert(*id);
                    (format!("{id}:removed"), "accounts-v1")
                }
                Action::Setting { key, value } => {
                    if !key.accepts(value) {
                        return Err(Error::Upgrade);
                    }
                    (format!("setting:{key:?}"), "settings-v1")
                }
                Action::SettingRemoved { key } => (format!("setting:{key:?}"), "settings-v1"),
                Action::ProfileName { name } => {
                    if !text(name, 256, false) {
                        return Err(Error::Invalid);
                    }
                    ("profile:name".into(), "causal-v1")
                }
                Action::ProfileRemoved => {
                    if self.changes.len() != 1 {
                        return Err(Error::Invalid);
                    }
                    ("profile:removed".into(), "causal-v1")
                }
            };
            if !required.contains(capability) || !targets.insert(target) {
                return Err(Error::Invalid);
            }
        }
        if !accounts.is_disjoint(&removed) {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

pub(crate) fn text(value: &str, max: usize, empty: bool) -> bool {
    value.len() <= max
        && (empty || !value.trim().is_empty())
        && !value.chars().any(char::is_control)
}
fn namespace(value: &str) -> bool {
    value.len() <= 128
        && value.contains('.')
        && value.split('.').all(|s| {
            !s.is_empty()
                && s.len() <= 63
                && !s.starts_with('-')
                && !s.ends_with('-')
                && s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use wasm_bindgen::prelude::*;
    /// Structural metadata validation only; it cannot enroll or apply a profile.
    #[wasm_bindgen]
    pub fn validate_profile_operation(bytes: &[u8]) -> std::result::Result<Vec<u8>, JsError> {
        super::Operation::decode(bytes)
            .and_then(|v| v.encode())
            .map_err(|e| JsError::new(&e.to_string()))
    }
}
