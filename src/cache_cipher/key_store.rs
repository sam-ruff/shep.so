//! A bounded OS-credential worker owns key admission; callers own the data-root
//! lease. Losing a required key never silently generates a replacement.
use super::Key;
use crate::credentials::{Credentials, Scope};
use anyhow::Context;
use secrecy::{ExposeSecret, SecretString};

const ENTRY: &str = "encryption-key-v1";

#[derive(Clone)]
pub struct Keys(Credentials);

impl Keys {
    /// The root ID comes from device-local metadata, never an imported database.
    pub fn new(root: uuid::Uuid) -> Self {
        Self(Credentials::new(Scope::CacheRoot(root)))
    }

    pub async fn load(&self) -> anyhow::Result<Key> {
        let saved = self.0.read_optional(ENTRY).await?.context("This device's cache key is missing. Keep your data and restore its original recovery key; a new key cannot unlock existing mail.")?;
        Key::decode(saved.expose_secret())
    }

    /// Only a new, exclusively owned data root may call this. An existing root
    /// uses `load`, including after a locked keychain or a failed migration.
    pub async fn create(&self) -> anyhow::Result<Key> {
        let candidate = Key::generate()?.encode();
        let saved = self
            .0
            .read_or_create(ENTRY, SecretString::from(candidate.to_string()))
            .await?;
        Key::decode(saved.expose_secret())
    }

    #[cfg(test)]
    pub(crate) fn with_credentials(credentials: Credentials) -> Self {
        Self(credentials)
    }
}
