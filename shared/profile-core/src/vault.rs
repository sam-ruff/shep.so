//! Credential vault codec for Google-only password protection.
//!
//! The vault key is stored in a separate app-data file beside the vault. This
//! keeps passwords out of causal history, caches, logs and exports, but anyone
//! who can read the Drive app data can decrypt them. Callers supply random key
//! material and a fresh nonce for every seal; this module performs no I/O.
use crate::account::{Connection, Protocol};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt};
use uuid::Uuid;
use zeroize::Zeroizing;

pub const KEY_FORMAT: &str = "so.shep.credential-key";
pub const VAULT_FORMAT: &str = "so.shep.credential-vault";
pub const ALGORITHM: &str = "A256GCM";
pub const ENVELOPE_VERSION: u8 = 1;
pub const KEY_BYTES: usize = 32;
pub const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
pub const MAX_ENTRIES: usize = 512;
pub const MAX_SECRET_BYTES: usize = 4096;
pub const MAX_FILE_BYTES: usize = 1024 * 1024;
/// Revisions stay within the integer range every client can represent exactly.
pub const MAX_REVISION: u64 = (1 << 53) - 1;
const MAX_SEALED_CHARS: usize = (1 + NONCE_BYTES + MAX_SECRET_BYTES + TAG_BYTES).div_ceil(3) * 4;
const ENDPOINT_CONTEXT: &str = "so.shep.credential-endpoint/1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("The synced password vault is invalid. This device's passwords were kept.")]
    Invalid,
    #[error("The synced password vault uses a newer format. Update Shep before syncing passwords.")]
    Upgrade,
    #[error("The synced password vault exceeds the supported size.")]
    TooLarge,
    #[error("The synced password vault belongs to another profile or key.")]
    Binding,
    #[error("A synced password did not match its key and context. It was not used.")]
    Authentication,
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Incoming,
    Smtp,
}
impl Field {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Smtp => "smtp",
        }
    }
    fn parse(value: &Json) -> Result<Self> {
        match value.as_str() {
            Some("incoming") => Ok(Self::Incoming),
            Some("smtp") => Ok(Self::Smtp),
            Some(_) => Err(Error::Upgrade),
            None => Err(Error::Invalid),
        }
    }
}

/// Lowercase hex SHA-256 identifying the server and login a password belongs
/// to. A password is only ever offered to the endpoint it was published for.
pub fn endpoint(connection: &Connection, field: Field) -> String {
    let (kind, host, port, username) = match field {
        Field::Incoming => (
            match connection.protocol {
                Protocol::Imap => "imap",
                Protocol::Pop3 => "pop3",
            },
            &connection.host,
            connection.port,
            &connection.username,
        ),
        Field::Smtp => (
            "smtp",
            &connection.smtp_host,
            connection.smtp_port,
            &connection.smtp_username,
        ),
    };
    let text = format!(
        "{ENDPOINT_CONTEXT}\n{kind}\n{}\n{port}\n{username}",
        host.to_ascii_lowercase()
    );
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

/// Vault key material. It never implements Serialize or prints its bytes.
#[derive(Clone)]
pub struct Key {
    profile: Uuid,
    generation: Uuid,
    id: Uuid,
    sequence: u32,
    material: Zeroizing<[u8; KEY_BYTES]>,
}
impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Key")
            .field("id", &self.id)
            .field("sequence", &self.sequence)
            .finish_non_exhaustive()
    }
}

#[derive(Serialize)]
struct KeyFile<'a> {
    format: &'a str,
    major: u16,
    minor: u16,
    algorithm: &'a str,
    profile: Uuid,
    generation: Uuid,
    key: Uuid,
    sequence: u32,
    material: &'a str,
}

impl Key {
    pub fn new(
        profile: Uuid,
        generation: Uuid,
        id: Uuid,
        sequence: u32,
        material: [u8; KEY_BYTES],
    ) -> Result<Self> {
        let material = Zeroizing::new(material);
        if [profile, generation, id].iter().any(Uuid::is_nil) || sequence == 0 {
            return Err(Error::Invalid);
        }
        Ok(Self {
            profile,
            generation,
            id,
            sequence,
            material,
        })
    }
    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn sequence(&self) -> u32 {
        self.sequence
    }
    pub fn profile(&self) -> Uuid {
        self.profile
    }
    pub fn generation(&self) -> Uuid {
        self.generation
    }

    /// Exact key-file bytes. Readers must reject any other profile/generation.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let material = Zeroizing::new(STANDARD.encode(&self.material[..]));
        let bytes = serde_json::to_vec(&KeyFile {
            format: KEY_FORMAT,
            major: 1,
            minor: 0,
            algorithm: ALGORITHM,
            profile: self.profile,
            generation: self.generation,
            key: self.id,
            sequence: self.sequence,
            material: &material,
        })
        .map_err(|_| Error::Invalid)?;
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8], profile: Uuid, generation: Uuid) -> Result<Self> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(Error::TooLarge);
        }
        let raw = crate::json::decode(bytes).map_err(|_| Error::Invalid)?;
        if raw["format"] != KEY_FORMAT || raw["major"] != 1 {
            return Err(Error::Upgrade);
        }
        minor(&raw)?;
        if raw["algorithm"] != ALGORITHM {
            return Err(Error::Upgrade);
        }
        if uuid(&raw["profile"])? != profile || uuid(&raw["generation"])? != generation {
            return Err(Error::Binding);
        }
        let sequence = raw["sequence"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or(Error::Invalid)?;
        let text = raw["material"].as_str().ok_or(Error::Invalid)?;
        let decoded = Zeroizing::new(STANDARD.decode(text).map_err(|_| Error::Invalid)?);
        let material: [u8; KEY_BYTES] = decoded[..].try_into().map_err(|_| Error::Invalid)?;
        Self::new(profile, generation, uuid(&raw["key"])?, sequence, material)
    }
}

/// The authenticated context of one sealed password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot<'a> {
    pub account: Uuid,
    pub field: Field,
    pub revision: u64,
    pub endpoint: &'a str,
}
impl Slot<'_> {
    fn validate(&self) -> Result<()> {
        if self.account.is_nil() || !revision(self.revision) || !digest(self.endpoint) {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

/// Exact authenticated data: newline-separated format, envelope version,
/// profile, generation, key, account, field, revision and endpoint digest.
pub fn associated_data(key: &Key, slot: &Slot<'_>) -> Vec<u8> {
    format!(
        "{VAULT_FORMAT}\n{ENVELOPE_VERSION}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        key.profile,
        key.generation,
        key.id,
        slot.account,
        slot.field.as_str(),
        slot.revision,
        slot.endpoint
    )
    .into_bytes()
}

/// Base64 of version byte, 12-byte nonce, then AES-256-GCM ciphertext and tag.
pub fn seal(key: &Key, slot: &Slot<'_>, secret: &[u8], nonce: [u8; NONCE_BYTES]) -> Result<String> {
    slot.validate()?;
    if secret.is_empty() {
        return Err(Error::Invalid);
    }
    if secret.len() > MAX_SECRET_BYTES {
        return Err(Error::TooLarge);
    }
    let aad = associated_data(key, slot);
    let ciphertext = cipher(key)?
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: secret,
                aad: &aad,
            },
        )
        .map_err(|_| Error::Invalid)?;
    let mut envelope = Vec::with_capacity(1 + NONCE_BYTES + ciphertext.len());
    envelope.push(ENVELOPE_VERSION);
    envelope.extend_from_slice(&nonce);
    envelope.extend_from_slice(&ciphertext);
    Ok(STANDARD.encode(envelope))
}

/// Tampering, another key and any changed context all fail authentication.
pub fn open(key: &Key, slot: &Slot<'_>, sealed: &str) -> Result<Zeroizing<Vec<u8>>> {
    slot.validate()?;
    if sealed.len() > MAX_SEALED_CHARS {
        return Err(Error::TooLarge);
    }
    let envelope = STANDARD.decode(sealed).map_err(|_| Error::Invalid)?;
    match envelope.first() {
        Some(&ENVELOPE_VERSION) => {}
        Some(_) => return Err(Error::Upgrade),
        None => return Err(Error::Invalid),
    }
    if envelope.len() <= 1 + NONCE_BYTES + TAG_BYTES {
        return Err(Error::Invalid);
    }
    let (nonce, ciphertext) = envelope[1..].split_at(NONCE_BYTES);
    let aad = associated_data(key, slot);
    let plaintext = cipher(key)?
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| Error::Authentication)?;
    Ok(Zeroizing::new(plaintext))
}

fn cipher(key: &Key) -> Result<Aes256Gcm> {
    Aes256Gcm::new_from_slice(&key.material[..]).map_err(|_| Error::Invalid)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Sealed { endpoint: String, sealed: String },
    Removed,
}
/// One slot per shared account and field. Removal markers carry no secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub account: Uuid,
    pub field: Field,
    pub revision: u64,
    pub device: Uuid,
    pub value: Value,
}
impl Entry {
    fn validate(&self) -> Result<()> {
        if self.account.is_nil() || self.device.is_nil() || !revision(self.revision) {
            return Err(Error::Invalid);
        }
        if let Value::Sealed { endpoint, sealed } = &self.value {
            if !digest(endpoint) || sealed.is_empty() {
                return Err(Error::Invalid);
            }
            if sealed.len() > MAX_SEALED_CHARS {
                return Err(Error::TooLarge);
            }
        }
        Ok(())
    }
    fn sealed(&self) -> &str {
        match &self.value {
            Value::Sealed { sealed, .. } => sealed,
            Value::Removed => "",
        }
    }
    pub fn slot(&self) -> Option<Slot<'_>> {
        match &self.value {
            Value::Sealed { endpoint, .. } => Some(Slot {
                account: self.account,
                field: self.field,
                revision: self.revision,
                endpoint,
            }),
            Value::Removed => None,
        }
    }
}

#[derive(Serialize)]
struct EntryFile<'a> {
    account: Uuid,
    field: Field,
    revision: u64,
    device: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    endpoint: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sealed: Option<&'a str>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    removed: bool,
}
#[derive(Serialize)]
struct VaultFile<'a> {
    format: &'a str,
    major: u16,
    minor: u16,
    profile: Uuid,
    generation: Uuid,
    key: Uuid,
    revision: u64,
    entries: Vec<EntryFile<'a>>,
}

/// A replaceable vault file. Every sealed entry uses this file's key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vault {
    pub profile: Uuid,
    pub generation: Uuid,
    pub key: Uuid,
    pub revision: u64,
    /// A newer minor version may carry data this client cannot preserve, so
    /// such a vault is read-only here.
    pub minor: u16,
    pub entries: Vec<Entry>,
}
impl Vault {
    fn validate(&self) -> Result<()> {
        if [self.profile, self.generation, self.key]
            .iter()
            .any(Uuid::is_nil)
            || !revision(self.revision)
        {
            return Err(Error::Invalid);
        }
        if self.entries.len() > MAX_ENTRIES {
            return Err(Error::TooLarge);
        }
        let mut slots = std::collections::BTreeSet::new();
        for entry in &self.entries {
            entry.validate()?;
            if !slots.insert((entry.account, entry.field)) {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }

    /// Exact bytes with entries sorted by account then field (incoming first).
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        if self.minor != 0 {
            return Err(Error::Upgrade);
        }
        let mut entries: Vec<_> = self.entries.iter().collect();
        entries.sort_by_key(|e| (e.account, e.field));
        let bytes = serde_json::to_vec(&VaultFile {
            format: VAULT_FORMAT,
            major: 1,
            minor: 0,
            profile: self.profile,
            generation: self.generation,
            key: self.key,
            revision: self.revision,
            entries: entries
                .into_iter()
                .map(|e| {
                    let (endpoint, sealed) = match &e.value {
                        Value::Sealed { endpoint, sealed } => {
                            (Some(endpoint.as_str()), Some(sealed.as_str()))
                        }
                        Value::Removed => (None, None),
                    };
                    EntryFile {
                        account: e.account,
                        field: e.field,
                        revision: e.revision,
                        device: e.device,
                        endpoint,
                        sealed,
                        removed: matches!(e.value, Value::Removed),
                    }
                })
                .collect(),
        })
        .map_err(|_| Error::Invalid)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(Error::TooLarge);
        }
        Ok(bytes)
    }

    /// Unknown optional fields at major 1 are ignored, not preserved.
    pub fn decode(bytes: &[u8], profile: Uuid, generation: Uuid) -> Result<Self> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err(Error::TooLarge);
        }
        let raw = crate::json::decode(bytes).map_err(|_| Error::Invalid)?;
        if raw["format"] != VAULT_FORMAT || raw["major"] != 1 {
            return Err(Error::Upgrade);
        }
        let minor = minor(&raw)?;
        if uuid(&raw["profile"])? != profile || uuid(&raw["generation"])? != generation {
            return Err(Error::Binding);
        }
        let list = raw["entries"].as_array().ok_or(Error::Invalid)?;
        if list.len() > MAX_ENTRIES {
            return Err(Error::TooLarge);
        }
        let entries = list.iter().map(entry).collect::<Result<Vec<_>>>()?;
        let vault = Self {
            profile,
            generation,
            key: uuid(&raw["key"])?,
            revision: raw["revision"]
                .as_u64()
                .filter(|n| revision(*n))
                .ok_or(Error::Invalid)?,
            minor,
            entries,
        };
        vault.validate()?;
        Ok(vault)
    }
}

fn entry(raw: &Json) -> Result<Entry> {
    if !raw.is_object() {
        return Err(Error::Invalid);
    }
    let removed = match raw.get("removed") {
        None => false,
        Some(Json::Bool(value)) => *value,
        Some(_) => return Err(Error::Invalid),
    };
    let value = match (removed, raw.get("endpoint"), raw.get("sealed")) {
        (true, None, None) => Value::Removed,
        (false, Some(Json::String(endpoint)), Some(Json::String(sealed))) => Value::Sealed {
            endpoint: endpoint.clone(),
            sealed: sealed.clone(),
        },
        _ => return Err(Error::Invalid),
    };
    let entry = Entry {
        account: uuid(&raw["account"])?,
        field: Field::parse(&raw["field"])?,
        revision: raw["revision"].as_u64().ok_or(Error::Invalid)?,
        device: uuid(&raw["device"])?,
        value,
    };
    entry.validate()?;
    Ok(entry)
}

/// Deterministic slot winner: the higher revision; on a tie a removal, then
/// the greater device UUID, then the greater sealed text.
pub fn supersedes(candidate: &Entry, current: &Entry) -> bool {
    use std::cmp::Ordering;
    match candidate.revision.cmp(&current.revision) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => match (&candidate.value, &current.value) {
            (Value::Removed, Value::Sealed { .. }) => true,
            (Value::Sealed { .. }, Value::Removed) => false,
            _ => (candidate.device, candidate.sealed()) > (current.device, current.sealed()),
        },
    }
}

/// Merge every listed vault file; each winning entry keeps its file's key.
pub fn merge<'a>(
    vaults: impl IntoIterator<Item = &'a Vault>,
) -> BTreeMap<(Uuid, Field), (Entry, Uuid)> {
    let mut merged: BTreeMap<(Uuid, Field), (Entry, Uuid)> = BTreeMap::new();
    for vault in vaults {
        for entry in &vault.entries {
            let slot = (entry.account, entry.field);
            if merged
                .get(&slot)
                .is_none_or(|(current, _)| supersedes(entry, current))
            {
                merged.insert(slot, (entry.clone(), vault.key));
            }
        }
    }
    merged
}

/// Highest sequence, then the smallest key UUID. Every device picks the same key.
pub fn canonical(keys: impl IntoIterator<Item = (Uuid, u32)>) -> Option<(Uuid, u32)> {
    keys.into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
}

fn minor(raw: &Json) -> Result<u16> {
    raw["minor"]
        .as_u64()
        .and_then(|n| u16::try_from(n).ok())
        .ok_or(Error::Invalid)
}
fn revision(value: u64) -> bool {
    (1..=MAX_REVISION).contains(&value)
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn uuid(value: &Json) -> Result<Uuid> {
    let text = value.as_str().ok_or(Error::Invalid)?;
    let id = Uuid::parse_str(text).map_err(|_| Error::Invalid)?;
    if id.is_nil() || id.to_string() != text {
        return Err(Error::Invalid);
    }
    Ok(id)
}
