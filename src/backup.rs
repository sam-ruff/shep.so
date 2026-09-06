use crate::{model::*, providers::google::Google};
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use anyhow::Context;
use async_trait::async_trait;
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::PathBuf};
use zeroize::Zeroizing;

const MAGIC: &[u8; 8] = b"SHEPBK01";
const MAX_DECODED: u64 = 768 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub created_at: i64,
    pub messages: Vec<StoredMail>,
    pub accounts: Vec<Account>,
    pub calendars: Vec<CalendarSource>,
    pub preferences: Preferences,
    pub credentials: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupCopy {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

/// A configured destination, independent of settings such as theme or retention.
/// A new Google authorization has its own identity even when the client ID stays
/// the same, because the user may have selected a different Google account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupTarget {
    Local(String),
    GoogleDrive {
        client_id: String,
        connection_id: String,
    },
}
impl BackupTarget {
    pub fn from_preferences(prefs: &Preferences) -> Self {
        match prefs.backup_destination {
            BackupDestination::Local => Self::Local(prefs.backup_folder.clone()),
            BackupDestination::GoogleDrive => Self::GoogleDrive {
                client_id: prefs.google_client_id.clone(),
                connection_id: prefs.google_connection_id.clone(),
            },
        }
    }
    fn secret_id(&self) -> String {
        use sha2::{Digest, Sha256};
        // The hash avoids putting a folder path or Google identity in key names.
        format!(
            "backup-passphrase:{:x}",
            Sha256::digest(serde_json::to_vec(self).expect("backup target serializes"))
        )
    }
}

#[async_trait]
pub(crate) trait PassphraseStore: Send + Sync {
    async fn read(&self, target: &BackupTarget) -> anyhow::Result<SecretString>;
    async fn write(&self, target: &BackupTarget, secret: SecretString) -> anyhow::Result<()>;
}
pub(crate) struct OsPassphraseStore;
#[async_trait]
impl PassphraseStore for OsPassphraseStore {
    async fn read(&self, target: &BackupTarget) -> anyhow::Result<SecretString> {
        crate::providers::read_secret(&target.secret_id()).await
    }
    async fn write(&self, target: &BackupTarget, secret: SecretString) -> anyhow::Result<()> {
        crate::providers::write_secret(&target.secret_id(), secret).await
    }
}

#[async_trait]
pub trait BackupProvider: Send + Sync {
    async fn list(&self) -> anyhow::Result<Vec<BackupCopy>>;
    async fn upload(&self, name: &str, data: Vec<u8>) -> anyhow::Result<String>;
    async fn download(&self, id: &str) -> anyhow::Result<Vec<u8>>;
    async fn delete(&self, id: &str) -> anyhow::Result<()>;
}

pub struct LocalBackup {
    pub directory: PathBuf,
}
pub struct DriveBackup {
    pub google: Google,
    pub preferences: Preferences,
}

pub fn encrypt(snapshot: &Snapshot, passphrase: &SecretString) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        passphrase.expose_secret().chars().count() >= 12,
        "Use a backup passphrase of at least 12 characters."
    );
    let json = Zeroizing::new(serde_json::to_vec(snapshot)?);
    let compressed = Zeroizing::new(zstd::encode_all(json.as_slice(), 3)?);
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce);
    let mut key = Zeroizing::new([0u8; 32]);
    argon2::Argon2::default()
        .hash_password_into(passphrase.expose_secret().as_bytes(), &salt, key.as_mut())
        .map_err(|_| anyhow::anyhow!("Could not derive backup encryption key"))?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|_| anyhow::anyhow!("Could not initialize encryption"))?;
    let mut header = MAGIC.to_vec();
    header.extend(salt);
    header.extend(nonce);
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &compressed,
                aad: &header,
            },
        )
        .map_err(|_| anyhow::anyhow!("Backup encryption failed"))?;
    header.extend(ciphertext);
    Ok(header)
}
pub fn decrypt(bytes: &[u8], passphrase: &SecretString) -> anyhow::Result<Snapshot> {
    anyhow::ensure!(
        bytes.len() >= 52 && &bytes[..8] == MAGIC,
        "This is not a supported Shep backup."
    );
    let mut key = Zeroizing::new([0u8; 32]);
    argon2::Argon2::default()
        .hash_password_into(
            passphrase.expose_secret().as_bytes(),
            &bytes[8..24],
            key.as_mut(),
        )
        .map_err(|_| anyhow::anyhow!("Could not derive backup key"))?;
    let cipher = Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|_| anyhow::anyhow!("Could not initialize decryption"))?;
    let compressed = Zeroizing::new(
        cipher
            .decrypt(
                Nonce::from_slice(&bytes[24..36]),
                Payload {
                    msg: &bytes[36..],
                    aad: &bytes[..36],
                },
            )
            .map_err(|_| anyhow::anyhow!("Incorrect passphrase or damaged backup."))?,
    );
    let mut decoded = Zeroizing::new(Vec::new());
    zstd::Decoder::new(compressed.as_slice())?
        .take(MAX_DECODED + 1)
        .read_to_end(&mut decoded)?;
    anyhow::ensure!(
        decoded.len() as u64 <= MAX_DECODED,
        "The backup exceeds the restore size limit."
    );
    let snapshot: Snapshot = serde_json::from_slice(&decoded)?;
    anyhow::ensure!(
        snapshot.version == 1,
        "This backup was made by a newer version of Shep."
    );
    snapshot.preferences.validate()?;
    for account in &snapshot.accounts {
        account.validate()?;
    }
    Ok(snapshot)
}

pub async fn retain(
    provider: &dyn BackupProvider,
    keep: usize,
    committed: &str,
) -> anyhow::Result<usize> {
    anyhow::ensure!((1..=100).contains(&keep), "Keep between 1 and 100 copies.");
    let mut copies = provider.list().await?;
    anyhow::ensure!(
        copies.iter().any(|copy| copy.id == committed),
        "The new copy is not yet visible in the backup list. Older copies were kept."
    );
    let mut ids = std::collections::HashSet::new();
    anyhow::ensure!(
        copies.iter().all(|copy| ids.insert(&copy.id)),
        "The backup list contains duplicate file IDs. Older copies were kept."
    );
    // Protect the acknowledged copy even after a clock correction or when two
    // copies share a timestamp. Listing failure must never trigger deletion.
    copies.sort_by(|a, b| {
        (b.id == committed)
            .cmp(&(a.id == committed))
            .then_with(|| b.name.cmp(&a.name))
    });
    let mut deleted = 0;
    for copy in copies.into_iter().skip(keep) {
        provider.delete(&copy.id).await?;
        deleted += 1;
    }
    Ok(deleted)
}
fn valid_name(name: &str) -> bool {
    let Some((stamp, identity)) = name
        .strip_prefix("shep-")
        .and_then(|name| name.strip_suffix(".shepbackup"))
        .and_then(|name| name.split_once('-'))
    else {
        return false;
    };
    stamp.len() == 16
        && chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%dT%H%M%SZ").is_ok()
        && uuid::Uuid::parse_str(identity).is_ok_and(|id| id.to_string() == identity)
}

#[async_trait]
impl BackupProvider for LocalBackup {
    async fn list(&self) -> anyhow::Result<Vec<BackupCopy>> {
        tokio::fs::create_dir_all(&self.directory).await?;
        let mut entries = tokio::fs::read_dir(&self.directory).await?;
        let mut copies = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            if valid_name(&name) && entry.file_type().await?.is_file() {
                copies.push(BackupCopy {
                    id: name.clone(),
                    created_at: name
                        .trim_start_matches("shep-")
                        .trim_end_matches(".shepbackup")
                        .into(),
                    name,
                });
            }
        }
        copies.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(copies)
    }
    async fn upload(&self, name: &str, data: Vec<u8>) -> anyhow::Result<String> {
        anyhow::ensure!(valid_name(name), "Invalid backup filename.");
        let directory = self.directory.clone();
        let filename = name.to_owned();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            use std::io::Write;
            std::fs::create_dir_all(&directory)?;
            let mut temporary = tempfile::Builder::new()
                .prefix(".shep-")
                .suffix(".tmp")
                .tempfile_in(&directory)?;
            temporary.write_all(&data)?;
            temporary.as_file().sync_all()?;
            // No overwrite, and RAII removes partial temporary files on errors.
            temporary
                .persist_noclobber(directory.join(filename))
                .map_err(|error| {
                    anyhow::anyhow!(
                        "Could not commit the backup without replacing an existing file: {}",
                        error.error
                    )
                })?;
            Ok(())
        })
        .await??;
        Ok(name.into())
    }
    async fn download(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        anyhow::ensure!(valid_name(id), "Invalid backup filename.");
        let path = self.directory.join(id);
        anyhow::ensure!(
            tokio::fs::symlink_metadata(&path)
                .await?
                .file_type()
                .is_file(),
            "Choose a regular backup file."
        );
        let file = tokio::fs::File::open(path).await?;
        anyhow::ensure!(
            file.metadata().await?.len() <= MAX_DECODED,
            "The backup exceeds the restore size limit."
        );
        let mut bytes = Vec::new();
        file.take(MAX_DECODED + 1).read_to_end(&mut bytes).await?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_DECODED,
            "The backup exceeds the restore size limit."
        );
        Ok(bytes)
    }
    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(valid_name(id), "Invalid backup filename.");
        if let Err(error) = tokio::fs::remove_file(self.directory.join(id)).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(error.into());
        }
        Ok(())
    }
}

#[async_trait]
impl BackupProvider for DriveBackup {
    async fn list(&self) -> anyhow::Result<Vec<BackupCopy>> {
        let token = self.google.token(&self.preferences).await?;
        let mut next = String::new();
        let mut copies = Vec::new();
        loop {
            let data: serde_json::Value = self
                .google
                .http
                .get("https://www.googleapis.com/drive/v3/files")
                .bearer_auth(token.expose_secret())
                .query(&[
                    ("spaces", "appDataFolder"),
                    (
                        "q",
                        "trashed = false and appProperties has { key='shepBackup' and value='1' }",
                    ),
                    ("fields", "nextPageToken,files(id,name,createdTime)"),
                    ("pageSize", "100"),
                    ("pageToken", &next),
                ])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if let Some(files) = data["files"].as_array() {
                for file in files {
                    let name = file["name"].as_str().unwrap_or_default();
                    if valid_name(name) {
                        copies.push(BackupCopy {
                            id: file["id"].as_str().context("Missing Drive file ID")?.into(),
                            name: name.into(),
                            created_at: file["createdTime"].as_str().unwrap_or("").into(),
                        });
                    }
                }
            }
            match data["nextPageToken"].as_str() {
                Some(n) => next = n.into(),
                None => break,
            }
        }
        copies.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(copies)
    }
    async fn upload(&self, name: &str, data: Vec<u8>) -> anyhow::Result<String> {
        let token = self.google.token(&self.preferences).await?;
        let boundary = format!("shep-{}", uuid::Uuid::new_v4());
        let metadata = serde_json::json!({"name":name,"parents":["appDataFolder"],"appProperties":{"shepBackup":"1"}});
        let mut body=format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
        body.extend(data);
        body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
        let response: serde_json::Value = self
            .google
            .http
            .post("https://www.googleapis.com/upload/drive/v3/files")
            .query(&[("uploadType", "multipart"), ("fields", "id")])
            .bearer_auth(token.expose_secret())
            .header(
                "Content-Type",
                format!("multipart/related; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(response["id"]
            .as_str()
            .context("Google Drive did not confirm the upload")?
            .into())
    }
    async fn download(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        let token = self.google.token(&self.preferences).await?;
        let mut response = self
            .google
            .http
            .get(drive_url(id)?)
            .query(&[("alt", "media")])
            .bearer_auth(token.expose_secret())
            .send()
            .await?
            .error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                (bytes.len() + chunk.len()) as u64 <= MAX_DECODED,
                "The backup exceeds the restore size limit."
            );
            bytes.extend(chunk);
        }
        Ok(bytes)
    }
    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        let token = self.google.token(&self.preferences).await?;
        self.google
            .http
            .delete(drive_url(id)?)
            .bearer_auth(token.expose_secret())
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}
fn drive_url(id: &str) -> anyhow::Result<url::Url> {
    let mut url = url::Url::parse("https://www.googleapis.com/drive/v3/files/")?;
    url.path_segments_mut().unwrap().pop_if_empty().push(id);
    Ok(url)
}
