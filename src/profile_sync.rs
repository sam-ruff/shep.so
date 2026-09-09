//! Continuous-profile transport primitives. Operations use the same pinned codec
//! as Flutter. Enrollment/merge owns application decisions; transport never
//! applies account settings, credentials or mail actions by itself.
pub mod drive;
pub mod journal;
pub mod replica;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub use shep_profile_core::{MAX_RECORD_BYTES, Operation};
use std::sync::Arc;
use uuid::Uuid;

/// Verified Drive account plus configured cross-client application namespace.
/// The OAuth client and local grant stay in the local connection lifecycle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    identity: String,
    namespace: String,
}
impl Binding {
    pub fn new(identity: String, namespace: String) -> anyhow::Result<Self> {
        anyhow::ensure!(
            identity
                .strip_prefix("drive:")
                .is_some_and(crate::providers::drive_http::valid_id),
            "Reconnect Google to verify the profile account."
        );
        anyhow::ensure!(
            namespace.len() <= 128
                && namespace.contains('.')
                && namespace.split('.').all(|part| !part.is_empty()
                    && part.len() <= 63
                    && !part.starts_with('-')
                    && !part.ends_with('-')
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')),
            "Use a valid configured Shep application namespace."
        );
        Ok(Self {
            identity,
            namespace,
        })
    }
    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    fn namespace_hash(&self) -> String {
        digest(self.namespace.as_bytes())
    }
    fn validate(&self) -> anyhow::Result<()> {
        Self::new(self.identity.clone(), self.namespace.clone()).map(|_| ())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Key {
    pub profile: Uuid,
    pub generation: Uuid,
    pub operation: Uuid,
}
impl Key {
    fn validate(self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.profile.is_nil() && !self.generation.is_nil() && !self.operation.is_nil(),
            "The profile record has an invalid identity."
        );
        Ok(())
    }
    fn filename(self) -> String {
        format!("shep-profile-{}.json", self.operation)
    }
}

/// Original bytes stay unchanged through retries, including unknown optional
/// fields/whitespace. Normalizing JSON would change the committed checksum.
#[derive(Clone, Debug)]
pub struct Record {
    operation: Arc<Operation>,
    bytes: Arc<[u8]>,
    sha256: String,
}
impl Record {
    pub fn decode(namespace: &str, bytes: Vec<u8>) -> anyhow::Result<Self> {
        let operation = Operation::decode(&bytes)?;
        anyhow::ensure!(
            operation.namespace == namespace,
            "This profile belongs to another application namespace. Keep the current setup and review the Google application configuration."
        );
        Ok(Self {
            operation: Arc::new(operation),
            sha256: digest(&bytes),
            bytes: bytes.into(),
        })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn operation(&self) -> &Operation {
        &self.operation
    }
    pub fn key(&self) -> Key {
        Key {
            profile: self.operation.profile,
            generation: self.operation.generation,
            operation: self.operation.operation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteRecord {
    id: String,
    key: Key,
    size: u64,
    sha256: String,
}
impl RemoteRecord {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn key(&self) -> Key {
        self.key
    }
    pub fn size(&self) -> u64 {
        self.size
    }
    fn validate(&self) -> anyhow::Result<()> {
        self.key.validate()?;
        anyhow::ensure!(
            crate::providers::drive_http::valid_id(&self.id)
                && self.id.len() <= 200
                && self.size > 0
                && self.size <= MAX_RECORD_BYTES as u64
                && valid_digest(&self.sha256),
            "The Drive profile record has invalid metadata. Keep the current setup and retry discovery."
        );
        Ok(())
    }
    fn verify(&self, record: &Record) -> anyhow::Result<()> {
        self.validate()?;
        anyhow::ensure!(
            self.key == record.key()
                && self.size == record.bytes.len() as u64
                && self.sha256 == record.sha256,
            "The profile content does not match its Drive identity/checksum. The local setup was kept."
        );
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ReservedUpload {
    binding: Binding,
    remote: RemoteRecord,
    record: Record,
}
impl ReservedUpload {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn remote(&self) -> &RemoteRecord {
        &self.remote
    }
    pub fn record(&self) -> &Record {
        &self.record
    }
    fn validate(&self) -> anyhow::Result<()> {
        self.binding.validate()?;
        anyhow::ensure!(
            self.binding.namespace == self.record.operation.namespace,
            "The pending profile upload belongs to another namespace."
        );
        self.remote.verify(&self.record)
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn canonical_uuid(value: Option<&str>) -> anyhow::Result<Uuid> {
    let value = value.context("Drive omitted a profile identity")?;
    let id = Uuid::parse_str(value)?;
    anyhow::ensure!(
        !id.is_nil() && id.to_string() == value,
        "Drive returned an invalid profile identity."
    );
    Ok(id)
}
