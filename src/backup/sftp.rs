//! Password-authenticated SFTP with pinned host keys and resumable immutable copies.
use super::{
    BackupCopy, BackupProvider, MAGIC, MAX_DECODED, PreparedUpload, UploadCheckpoint, valid_name,
};
use anyhow::Context;
use async_trait::async_trait;
use base64::Engine as _;
use russh::{
    client,
    keys::{HashAlg, PublicKeyOrCertificate},
};
use russh_sftp::{
    client::{RawSftpSession, error::Error as SftpError},
    protocol::{FileAttributes, OpenFlags, StatusCode},
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, sync::Arc, time::Duration};
use tokio::sync::oneshot;

const CHUNK: u32 = 32 * 1024;
const MAX_ENTRIES: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub directory: String,
    pub fingerprint: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            username: String::new(),
            directory: String::new(),
            fingerprint: String::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub directory: String,
    pub fingerprint: String,
}
impl Settings {
    pub fn identity(&self) -> Identity {
        Identity {
            host: normalized_host(&self.host).unwrap_or_else(|_| self.host.clone()),
            port: self.port,
            username: self.username.clone(),
            fingerprint: self.fingerprint.clone(),
            directory: if self.directory == "/" {
                "/".into()
            } else {
                self.directory.trim_end_matches('/').into()
            },
        }
    }
    pub(crate) fn secret_id(&self) -> String {
        format!(
            "backup-sftp:{:x}",
            Sha256::digest(serde_json::to_vec(&self.identity()).expect("SFTP identity serializes"))
        )
    }
    pub(crate) fn server(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.host.is_empty()
                && self.host.len() <= 253
                && !self.host.contains(['/', '@', '\\'])
                && !self.host.chars().any(char::is_whitespace)
                && normalized_host(&self.host).is_ok(),
            "Enter an SFTP hostname or IP address without a URL or username."
        );
        anyhow::ensure!(self.port != 0, "Enter an SFTP port between 1 and 65535.");
        Ok(())
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        self.server()?;
        anyhow::ensure!(
            !self.username.is_empty()
                && self.username.len() <= 256
                && !self.username.chars().any(char::is_control),
            "Enter the SFTP username."
        );
        anyhow::ensure!(
            self.directory.starts_with('/')
                && self.directory.len() <= 4096
                && !self.directory.contains('\\')
                && !self.directory.chars().any(char::is_control)
                && !self.directory.contains("//")
                && !self.directory.split('/').any(|p| p == "." || p == ".."),
            "Use an absolute SFTP folder without dot segments or repeated slashes."
        );
        validate_fingerprint(&self.fingerprint)
    }
    pub(crate) fn validate_draft(&self) -> anyhow::Result<()> {
        let mut complete = self.clone();
        if complete.host.is_empty() {
            complete.host = "unconfigured.invalid".into();
        }
        if complete.username.is_empty() {
            complete.username = "unconfigured".into();
        }
        if complete.directory.is_empty() {
            complete.directory = "/".into();
        }
        if complete.fingerprint.is_empty() {
            complete.fingerprint = format!(
                "SHA256:{}",
                base64::engine::general_purpose::STANDARD_NO_PAD.encode([0; 32])
            );
        }
        complete.validate()
    }
}
fn normalized_host(host: &str) -> anyhow::Result<String> {
    let address = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(address) = address.parse::<std::net::IpAddr>() {
        return Ok(address.to_string());
    }
    Ok(url::Host::parse(host)?.to_string())
}
fn validate_fingerprint(value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        value.strip_prefix("SHA256:").is_some_and(|v| {
            base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(v)
                .is_ok_and(|b| b.len() == 32)
        }),
        "Verify the server's SHA256 host-key fingerprint before connecting."
    );
    Ok(())
}
struct KeyCheck {
    expected: Option<String>,
    offered: Option<oneshot::Sender<String>>,
}
impl client::Handler for KeyCheck {
    type Error = anyhow::Error;
    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        if let Some(offered) = self.offered.take() {
            let _ = offered.send(fingerprint.clone());
        }
        Ok(self.expected.as_ref() == Some(&fingerprint))
    }
}
fn ssh_config() -> Arc<client::Config> {
    Arc::new(client::Config {
        channel_buffer_size: 8,
        inactivity_timeout: Some(Duration::from_secs(60)),
        keepalive_interval: Some(Duration::from_secs(15)),
        keepalive_max: 2,
        nodelay: true,
        ..Default::default()
    })
}
/// Read the offered key without accepting it or sending any authentication data.
pub async fn probe_fingerprint(settings: &Settings) -> anyhow::Result<String> {
    settings.server()?;
    let (offered, mut observation) = oneshot::channel();
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        client::connect(
            ssh_config(),
            (normalized_host(&settings.host)?.as_str(), settings.port),
            KeyCheck {
                expected: None,
                offered: Some(offered),
            },
        ),
    )
    .await;
    drop(result);
    observation
        .try_recv()
        .context("Could not read the SFTP server fingerprint. Check the host and port, then retry.")
}

pub struct SftpBackup {
    settings: Settings,
    password: SecretString,
}
impl SftpBackup {
    pub fn new(settings: &Settings, password: SecretString) -> anyhow::Result<Self> {
        settings.validate()?;
        anyhow::ensure!(
            !password.expose_secret().is_empty(),
            "Enter the SFTP password."
        );
        Ok(Self {
            settings: settings.clone(),
            password,
        })
    }
    async fn connect(&self) -> anyhow::Result<Remote> {
        let (offered, mut observation) = oneshot::channel();
        let connection = tokio::time::timeout(
            Duration::from_secs(15),
            client::connect(
                ssh_config(),
                (
                    normalized_host(&self.settings.host)?.as_str(),
                    self.settings.port,
                ),
                KeyCheck {
                    expected: Some(self.settings.fingerprint.clone()),
                    offered: Some(offered),
                },
            ),
        )
        .await;
        let mut ssh = match connection {
            Ok(Ok(ssh)) => ssh,
            _ => {
                if let Ok(actual) = observation.try_recv()
                    && actual != self.settings.fingerprint
                {
                    anyhow::bail!(
                        "The SFTP host key changed. Expected {}; received {}. Verify the server's identity before changing its saved fingerprint. No password was sent.",
                        self.settings.fingerprint,
                        actual
                    );
                }
                anyhow::bail!(
                    "Could not connect to the verified SFTP server. Check the host, port and fingerprint, then retry."
                );
            }
        };
        let authenticated = tokio::time::timeout(
            Duration::from_secs(30),
            ssh.authenticate_password(&self.settings.username, self.password.expose_secret()),
        )
        .await
        .context("The SFTP login timed out.")?
        .map_err(|_| anyhow::anyhow!("The SFTP login could not complete."))?;
        anyhow::ensure!(
            authenticated.success(),
            "SFTP rejected the username or password. Check the connection settings."
        );
        // A peer can answer keepalives while never confirming a session.
        // Bound setup itself; transport inactivity alone is not a deadline.
        let channel = tokio::time::timeout(Duration::from_secs(15), ssh.channel_open_session())
            .await
            .context("The SFTP server did not open a session in time. Retry the connection.")?
            .map_err(|_| anyhow::anyhow!("Could not open the SFTP channel."))?;
        tokio::time::timeout(
            Duration::from_secs(15),
            channel.request_subsystem(true, "sftp"),
        )
        .await
        .context("The SFTP subsystem request timed out. Retry the connection.")?
        .map_err(|_| anyhow::anyhow!("The server did not accept SFTP."))?;
        let sftp = RawSftpSession::new_with_config(
            bounded_stream(channel.into_stream()),
            russh_sftp::client::Config {
                max_concurrent_reads: 4,
                max_concurrent_writes: 4,
                request_timeout_secs: 30,
                ..Default::default()
            },
        );
        let version = sftp
            .init()
            .await
            .map_err(wire)
            .context("Could not start the SFTP protocol.")?;
        anyhow::ensure!(
            version.version == 3,
            "This server does not support SFTP version 3."
        );
        let remote = Remote {
            sftp,
            _ssh: ssh,
            directory: self.settings.identity().directory,
            fsync: version
                .extensions
                .get("fsync@openssh.com")
                .is_some_and(|v| v == "1"),
        };
        let canonical = remote
            .sftp
            .realpath(&remote.directory)
            .await
            .map_err(wire)
            .context("The SFTP backup folder was not found. Create it on the server first.")?;
        anyhow::ensure!(
            canonical.files.len() == 1 && canonical.files[0].filename == remote.directory,
            "Use the SFTP folder's absolute canonical path so aliases cannot share retention."
        );
        let attributes = remote
            .sftp
            .lstat(&remote.directory)
            .await
            .map_err(wire)?
            .attrs;
        anyhow::ensure!(
            attributes.is_dir() && !attributes.is_symlink(),
            "Choose a regular SFTP directory, not a symbolic link."
        );
        Ok(remote)
    }
    pub async fn test_connection(&self) -> anyhow::Result<()> {
        self.list().await.map(|_| ())
    }
}
// The dependency's raw session currently ignores Config::max_packet_len.
// Validate framing before passing bytes to its parser; never allocate a
// server-advertised multi-gigabyte packet from a four-byte length prefix.
fn bounded_stream<S>(stream: S) -> impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send,
{
    let (read, write) = tokio::io::split(stream);
    let framed = tokio_util::codec::LengthDelimitedCodec::builder()
        .max_frame_length(256 * 1024)
        .length_adjustment(4)
        .num_skip(0)
        .new_read(read);
    tokio::io::join(tokio_util::io::StreamReader::new(framed), write)
}
fn wire(error: SftpError) -> anyhow::Error {
    match error {
        SftpError::Status(status) => anyhow::anyhow!(
            "SFTP rejected the request ({:?}). Check access to this backup folder.",
            status.status_code
        ),
        _ => anyhow::anyhow!("The SFTP request did not complete. Check the connection and retry."),
    }
}
struct Remote {
    sftp: RawSftpSession,
    _ssh: client::Handle<KeyCheck>,
    directory: String,
    fsync: bool,
}
fn missing(error: &SftpError) -> bool {
    matches!(error, SftpError::Status(s) if s.status_code == StatusCode::NoSuchFile)
}
impl Remote {
    fn path(&self, name: &str) -> String {
        format!("{}/{name}", self.directory.trim_end_matches('/'))
    }
    async fn attributes(&self, name: &str) -> anyhow::Result<Option<FileAttributes>> {
        match self.sftp.lstat(self.path(name)).await {
            Ok(attrs) => {
                anyhow::ensure!(
                    attrs.attrs.is_regular() && !attrs.attrs.is_symlink(),
                    "An SFTP backup path is not a regular file. It was kept unchanged."
                );
                Ok(Some(attrs.attrs))
            }
            Err(error) if missing(&error) => Ok(None),
            Err(error) => Err(wire(error)).context("Could not check the SFTP backup file."),
        }
    }
    async fn owned(&self, name: &str) -> anyhow::Result<bool> {
        let Some(attrs) = self.attributes(name).await? else {
            return Ok(false);
        };
        anyhow::ensure!(
            attrs
                .size
                .is_some_and(|size| size >= MAGIC.len() as u64 && size <= MAX_DECODED),
            "The SFTP backup has an invalid size."
        );
        let handle = self
            .sftp
            .open(self.path(name), OpenFlags::READ, FileAttributes::empty())
            .await
            .map_err(wire)?
            .handle;
        let mut prefix = Vec::with_capacity(MAGIC.len());
        while prefix.len() < MAGIC.len() {
            let bytes = self
                .sftp
                .read(
                    &handle,
                    prefix.len() as u64,
                    (MAGIC.len() - prefix.len()) as u32,
                )
                .await
                .map_err(wire)?
                .data;
            anyhow::ensure!(
                !bytes.is_empty() && bytes.len() <= MAGIC.len() - prefix.len(),
                "SFTP returned an invalid backup header."
            );
            prefix.extend(bytes);
        }
        self.sftp.close(handle).await.map_err(wire)?;
        anyhow::ensure!(
            super::format::recognized_prefix(&prefix),
            "This SFTP file is not a Shep backup. It was kept unchanged."
        );
        Ok(true)
    }
    async fn read(
        &self,
        name: &str,
        expected: Option<&PreparedUpload>,
        collect: bool,
    ) -> anyhow::Result<Option<Vec<u8>>> {
        let Some(attrs) = self.attributes(name).await? else {
            return Ok(None);
        };
        let size = attrs.size.context("SFTP omitted the backup size.")?;
        anyhow::ensure!(
            size <= MAX_DECODED && size >= MAGIC.len() as u64,
            "The SFTP backup has an invalid size."
        );
        if let Some(expected) = expected {
            anyhow::ensure!(
                size == expected.size,
                "The reserved SFTP file contains different data. It was kept unchanged."
            );
        }
        let handle = self
            .sftp
            .open(self.path(name), OpenFlags::READ, FileAttributes::empty())
            .await
            .map_err(wire)?
            .handle;
        let mut bytes = Vec::new();
        let mut hash = Sha256::new();
        let mut prefix = Vec::with_capacity(MAGIC.len());
        let mut offset = 0;
        while offset < size {
            let chunk = self
                .sftp
                .read(&handle, offset, CHUNK.min((size - offset) as u32))
                .await
                .map_err(wire)?
                .data;
            anyhow::ensure!(
                !chunk.is_empty() && chunk.len() <= (size - offset).min(CHUNK as u64) as usize,
                "SFTP returned an incomplete or oversized backup chunk."
            );
            prefix.extend(chunk.iter().copied().take(MAGIC.len() - prefix.len()));
            hash.update(&chunk);
            offset += chunk.len() as u64;
            if collect {
                bytes.extend(chunk);
            }
        }
        anyhow::ensure!(
            super::format::recognized_prefix(&prefix),
            "This SFTP file is not a Shep backup. It was kept unchanged."
        );
        anyhow::ensure!(
            self.sftp.fstat(&handle).await.map_err(wire)?.attrs.size == Some(size),
            "The SFTP backup changed while reading. Retry after other writes finish."
        );
        self.sftp.close(handle).await.map_err(wire)?;
        if let Some(expected) = expected {
            anyhow::ensure!(
                format!("{:x}", hash.finalize()) == expected.sha256,
                "The reserved SFTP file contains different data. It was kept unchanged."
            );
        }
        Ok(Some(bytes))
    }
}
#[derive(Serialize, Deserialize)]
struct StagingReceipt {
    path: String,
    created: bool,
}
#[async_trait]
impl BackupProvider for SftpBackup {
    async fn reserve(&self, name: &str, data: &[u8]) -> anyhow::Result<PreparedUpload> {
        anyhow::ensure!(valid_name(name), "Invalid SFTP backup filename.");
        let upload = PreparedUpload::new(name.into(), name.into(), data);
        upload.verify(data)?;
        Ok(upload)
    }
    async fn upload(&self, name: &str, data: Vec<u8>) -> anyhow::Result<String> {
        let mut upload = self.reserve(name, &data).await?;
        self.upload_prepared(&mut upload, &data, &super::NoCheckpoint)
            .await?;
        Ok(upload.id)
    }
    async fn upload_prepared(
        &self,
        upload: &mut PreparedUpload,
        data: &[u8],
        checkpoint: &dyn UploadCheckpoint,
    ) -> anyhow::Result<()> {
        upload.verify(data)?;
        anyhow::ensure!(
            upload.id == upload.name
                && valid_name(&upload.id)
                && super::format::recognized_prefix(data),
            "Invalid SFTP backup reservation."
        );
        let remote = self.connect().await?;
        if remote
            .read(&upload.id, Some(upload), false)
            .await?
            .is_some()
        {
            return Ok(());
        }
        let staging = format!(".shep-upload-{}.part", upload.id);
        let mut offset = 0;
        let handle = if let Some(attrs) = remote.attributes(&staging).await? {
            let receipt: StagingReceipt = serde_json::from_str(upload.session.as_deref().context("An unconfirmed SFTP staging file already exists. Inspect it before retrying; no file was overwritten.")?)?;
            anyhow::ensure!(
                receipt.created && receipt.path == staging,
                "The SFTP staging receipt does not match this pending backup."
            );
            let size = attrs.size.context("SFTP omitted the staged file size.")?;
            anyhow::ensure!(
                size <= upload.size,
                "The SFTP staging file contains unexpected data. It was kept unchanged."
            );
            let handle = remote
                .sftp
                .open(
                    remote.path(&staging),
                    OpenFlags::READ | OpenFlags::WRITE,
                    FileAttributes::empty(),
                )
                .await
                .map_err(wire)?
                .handle;
            while offset < size {
                let chunk = remote
                    .sftp
                    .read(&handle, offset, CHUNK.min((size - offset) as u32))
                    .await
                    .map_err(wire)?
                    .data;
                anyhow::ensure!(
                    !chunk.is_empty()
                        && chunk.len() <= (size - offset).min(CHUNK as u64) as usize
                        && data.get(offset as usize..offset as usize + chunk.len())
                            == Some(chunk.as_slice()),
                    "The SFTP staging file contains unexpected data. It was kept unchanged."
                );
                offset += chunk.len() as u64;
            }
            handle
        } else {
            let attrs = FileAttributes {
                permissions: Some(0o600),
                ..FileAttributes::empty()
            };
            let handle = remote
                .sftp
                .open(
                    remote.path(&staging),
                    OpenFlags::READ | OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::EXCLUDE,
                    attrs,
                )
                .await
                .map_err(wire)
                .context("Could not reserve the SFTP staging file without replacing another file.")?
                .handle;
            upload.session = Some(serde_json::to_string(&StagingReceipt {
                path: staging.clone(),
                created: true,
            })?);
            checkpoint.save(upload).await?;
            handle
        };
        while offset < upload.size {
            let end = (offset + CHUNK as u64).min(upload.size);
            remote.sftp.write(&handle, offset, data[offset as usize..end as usize].to_vec()).await.map_err(wire).context("The SFTP upload was interrupted. Its staged data and local receipt are kept for retry.")?;
            offset = end;
        }
        if remote.fsync {
            remote.sftp.fsync(&handle).await.map_err(wire)?;
        }
        remote.sftp.close(handle).await.map_err(wire)?;
        remote
            .read(&staging, Some(upload), false)
            .await?
            .context("The staged SFTP copy disappeared before committing.")?;
        // SFTP v3 RENAME must reject an existing destination. Do not use the
        // OpenSSH POSIX rename extension, which permits replacement.
        let renamed = remote
            .sftp
            .rename(remote.path(&staging), remote.path(&upload.id))
            .await;
        if remote
            .read(&upload.id, Some(upload), false)
            .await?
            .is_some()
        {
            return Ok(());
        }
        renamed.map_err(wire).context(
            "Could not confirm the SFTP backup commit. Retry to recover the same reserved copy.",
        )?;
        anyhow::bail!("The committed SFTP copy was not found. Its local upload receipt was kept.")
    }
    async fn verify_upload(&self, upload: &PreparedUpload) -> anyhow::Result<bool> {
        anyhow::ensure!(
            upload.id == upload.name && valid_name(&upload.id),
            "Invalid SFTP backup reservation."
        );
        Ok(self
            .connect()
            .await?
            .read(&upload.id, Some(upload), false)
            .await?
            .is_some())
    }
    async fn download(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        anyhow::ensure!(valid_name(id), "Invalid SFTP backup filename.");
        self.connect()
            .await?
            .read(id, None, true)
            .await?
            .context("This SFTP copy no longer exists. Refresh saved copies.")
    }
    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        anyhow::ensure!(valid_name(id), "Invalid SFTP backup filename.");
        let remote = self.connect().await?;
        if remote.read(id, None, false).await?.is_some() {
            remote
                .sftp
                .remove(remote.path(id))
                .await
                .map_err(wire)
                .context("Could not remove the old SFTP backup. Other copies were kept.")?;
        }
        Ok(())
    }
    async fn list(&self) -> anyhow::Result<Vec<BackupCopy>> {
        let remote = self.connect().await?;
        let handle = remote
            .sftp
            .opendir(&remote.directory)
            .await
            .map_err(wire)?
            .handle;
        let mut names = HashSet::new();
        let mut copies = Vec::new();
        for page in 0..1000 {
            let page_data = match remote.sftp.readdir(&handle).await {
                Ok(page) => page,
                Err(SftpError::Status(status)) if status.status_code == StatusCode::Eof => {
                    remote.sftp.close(handle).await.map_err(wire)?;
                    copies.sort_by(|a: &BackupCopy, b| b.name.cmp(&a.name));
                    return Ok(copies);
                }
                Err(error) => {
                    return Err(wire(error))
                        .context("The SFTP listing did not finish. Existing copies were kept.");
                }
            };
            anyhow::ensure!(
                !page_data.files.is_empty() && page < 999,
                "The SFTP server returned an incomplete directory listing. Existing copies were kept."
            );
            for file in page_data.files {
                anyhow::ensure!(
                    names.len() < MAX_ENTRIES && names.insert(file.filename.clone()),
                    "The SFTP server returned too many or repeated directory entries. Existing copies were kept."
                );
                if valid_name(&file.filename) {
                    let Some(attrs) = remote.attributes(&file.filename).await? else {
                        continue;
                    };
                    if !remote.owned(&file.filename).await? {
                        continue;
                    }
                    copies.push(BackupCopy {
                        id: file.filename.clone(),
                        name: file.filename,
                        created_at: attrs
                            .mtime
                            .and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0))
                            .map(|t| t.to_rfc3339())
                            .unwrap_or_default(),
                    });
                }
            }
        }
        anyhow::bail!("The SFTP directory listing was incomplete. Existing copies were kept.")
    }
}

#[cfg(test)]
#[path = "sftp_tests.rs"]
pub(crate) mod tests;
