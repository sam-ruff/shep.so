//! Owned native-fixture targets; production never enables these providers or secrets.
use crate::{
    backup::{self, BackupProvider, PassphraseStore},
    credentials,
    model::Preferences,
    store::Store,
};
use anyhow::Context;
use async_trait::async_trait;
use secrecy::SecretString;
use std::{collections::HashMap, path::PathBuf};

pub fn active() -> bool {
    mode().is_some()
}
fn mode() -> Option<String> {
    std::env::args()
        .find_map(|a| a.strip_prefix("--backup-run=").map(str::to_owned))
        .filter(|value| matches!(value.as_str(), "ready" | "recover"))
}
fn root() -> anyhow::Result<PathBuf> {
    Ok(super::workspace::path_from_arguments()?
        .context("Backup fixtures require an owned persistent workspace")?
        .with_file_name("backup-targets"))
}

pub async fn seed(store: &Store) -> anyhow::Result<()> {
    if !active() {
        return Ok(());
    }
    let directory = root()?;
    tokio::fs::create_dir_all(&directory).await?;
    let mut prefs: Preferences = store.get("preferences").await?;
    prefs.backup_folder = directory.join("first").to_string_lossy().into_owned();
    prefs.backup_accounts = false;
    backup::config::add(&mut prefs)?;
    prefs.backup_folder = directory.join("second").to_string_lossy().into_owned();
    prefs.backup_accounts = false;
    backup::config::capture_editor(&mut prefs);
    for (index, destination) in prefs.backup_destinations.iter_mut().enumerate() {
        destination.name = ["Home archive", "Second copy"][index].into();
        destination.ready = true;
        destination.accounts = false;
        destination.copies = 2;
    }
    let first = prefs.backup_destinations[0].clone();
    first.apply(&mut prefs);
    prefs.backup_selected = Some(first.id);
    store.put("preferences", prefs).await
}

#[derive(Default)]
struct Vault(HashMap<String, SecretString>);
impl credentials::Backend for Vault {
    fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>> {
        Ok(self.0.get(key).cloned())
    }
    fn write(&mut self, key: &str, value: SecretString) -> anyhow::Result<()> {
        self.0.insert(key.into(), value);
        Ok(())
    }
    fn delete(&mut self, key: &str) -> anyhow::Result<()> {
        self.0.remove(key);
        Ok(())
    }
}
pub async fn credentials(store: &Store) -> anyhow::Result<credentials::Credentials> {
    let vault =
        credentials::Credentials::with_backend(credentials::Scope::Legacy, Vault::default());
    let secrets = backup::OsPassphraseStore(vault.clone());
    let prefs: Preferences = store.get("preferences").await?;
    for d in &prefs.backup_destinations {
        if d.ready {
            // Explicit, distinct fictional secrets are seeded anew after restart.
            secrets
                .write(
                    &d.target(&prefs),
                    format!("fixture passphrase for {}", d.folder).into(),
                )
                .await?;
        }
    }
    Ok(vault)
}

pub fn provider(store: &Store, prefs: &Preferences) -> anyhow::Result<Box<dyn BackupProvider>> {
    let path = PathBuf::from(&prefs.backup_folder);
    anyhow::ensure!(
        active()
            && prefs.backup_destination == crate::model::BackupDestination::Local
            && [root()?.join("first"), root()?.join("second")].contains(&path),
        "Only owned local backup fixture targets are allowed in preview."
    );
    anyhow::ensure!(
        !prefs.backup_accounts,
        "Fixture backups never read real account credentials."
    );
    Ok(Box::new(FixtureProvider {
        local: backup::LocalBackup { directory: path },
        store: store.clone(),
        fail_once: mode().as_deref() == Some("recover"),
    }))
}
struct FixtureProvider {
    local: backup::LocalBackup,
    store: Store,
    fail_once: bool,
}
#[async_trait]
impl BackupProvider for FixtureProvider {
    async fn list(&self) -> anyhow::Result<Vec<backup::BackupCopy>> {
        self.local.list().await
    }
    async fn upload(&self, name: &str, data: Vec<u8>) -> anyhow::Result<String> {
        self.local.upload(name, data).await
    }
    async fn download(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        self.local.download(id).await
    }
    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.local.delete(id).await
    }
    async fn reserve(&self, name: &str, data: &[u8]) -> anyhow::Result<backup::PreparedUpload> {
        self.local.reserve(name, data).await
    }
    async fn verify_upload(&self, upload: &backup::PreparedUpload) -> anyhow::Result<bool> {
        self.local.verify_upload(upload).await
    }
    async fn upload_prepared(
        &self,
        upload: &mut backup::PreparedUpload,
        data: &[u8],
        checkpoint: &dyn backup::UploadCheckpoint,
    ) -> anyhow::Result<()> {
        // Keep the second destination visibly pending while the first can finish.
        if self
            .local
            .directory
            .file_name()
            .is_some_and(|name| name == "second")
        {
            tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
        }
        self.local.upload_prepared(upload, data, checkpoint).await?;
        if self.fail_once
            && self
                .local
                .directory
                .file_name()
                .is_some_and(|name| name == "second")
            && !self.store.get::<bool>("backup-fixture-lost-ack").await?
        {
            self.store.put("backup-fixture-lost-ack", true).await?;
            anyhow::bail!(
                "Fixture upload acknowledgment was lost. Retry to verify the same encrypted copy."
            );
        }
        Ok(())
    }
}
