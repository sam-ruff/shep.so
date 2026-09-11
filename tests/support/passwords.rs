//! Owned password-sync fixture: a fictional keychain beside the persistent
//! fixture workspace and a connection tester that never contacts a server.
use crate::{
    credentials,
    model::{Account, ConnectionTarget},
    profile_sync::vault,
};
use anyhow::Context;
use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use std::{collections::BTreeMap, path::PathBuf};

const FIXTURES: &str = include_str!("../../shared/credential-vault-fixtures.json");

fn mode() -> Option<String> {
    std::env::args()
        .find_map(|a| a.strip_prefix("--profile-passwords=").map(str::to_owned))
        .filter(|value| matches!(value.as_str(), "ready" | "reject"))
}
pub fn active() -> bool {
    mode().is_some()
}

/// The fictional keychain models the OS keychain, so it lives outside the
/// workspace cache that the native scenarios scan for passwords.
struct Keychain {
    path: Option<PathBuf>,
    entries: BTreeMap<String, String>,
}
impl Keychain {
    fn save(&self) -> anyhow::Result<()> {
        if let Some(path) = &self.path {
            std::fs::write(path, serde_json::to_vec(&self.entries)?)?;
        }
        Ok(())
    }
}
impl credentials::Backend for Keychain {
    fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>> {
        Ok(self.entries.get(key).cloned().map(SecretString::from))
    }
    fn write(&mut self, key: &str, value: SecretString) -> anyhow::Result<()> {
        self.entries
            .insert(key.into(), value.expose_secret().to_owned());
        self.save()
    }
    fn delete(&mut self, key: &str) -> anyhow::Result<()> {
        self.entries.remove(key);
        self.save()
    }
}

pub fn credentials() -> anyhow::Result<credentials::Credentials> {
    let path = super::workspace::path_from_arguments()?
        .map(|workspace| workspace.with_file_name("fixture-keychain.json"));
    let entries = match path.as_ref().filter(|p| p.exists()) {
        Some(path) => serde_json::from_slice(&std::fs::read(path)?)
            .context("The fixture keychain is unreadable")?,
        None => BTreeMap::from([
            ("preview-work".into(), "fixture-studio-password".into()),
            (
                "preview-personal".into(),
                "fixture-personal-password".into(),
            ),
        ]),
    };
    let keychain = Keychain { path, entries };
    keychain.save()?;
    Ok(credentials::Credentials::with_backend(
        credentials::Scope::Legacy,
        keychain,
    ))
}

/// Accepts only the shared fixture's synced passwords for its Home account.
pub struct Tester;
#[async_trait]
impl vault::Tester for Tester {
    async fn test(
        &self,
        account: &Account,
        target: ConnectionTarget,
        secret: &SecretString,
    ) -> anyhow::Result<()> {
        let fixtures: serde_json::Value = serde_json::from_str(FIXTURES)?;
        let (host, expected) = match target {
            ConnectionTarget::Incoming => (&account.host, &fixtures["native_fixture"]["incoming"]),
            ConnectionTarget::Smtp => (&account.smtp_host, &fixtures["native_fixture"]["smtp"]),
        };
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        anyhow::ensure!(
            mode().as_deref() == Some("ready")
                && host.ends_with("example.test")
                && expected.as_str() == Some(secret.expose_secret()),
            "The fixture mail server rejected the synced password."
        );
        Ok(())
    }
}
