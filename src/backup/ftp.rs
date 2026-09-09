//! FTP/FTPS transport; each libcurl request is owned by a background task.
use super::{
    BackupCopy, BackupProvider, MAGIC, MAX_DECODED, PreparedUpload, UploadCheckpoint, valid_name,
};
use anyhow::Context;
use async_trait::async_trait;
use curl::easy::{Easy2, Handler, ReadError, WriteError};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{ffi::CString, sync::Arc, time::Duration};
use tokio::sync::oneshot;

const LIST_LIMIT: usize = 2 * 1024 * 1024;
const ENTRY_LIMIT: usize = 10_000;
const MANIFEST_LIMIT: usize = 2048;
const ARCHIVE: &str = "mail.shepbackup";
const COMMIT: &str = "shep-commit.json";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Security {
    #[default]
    ExplicitTls,
    ImplicitTls,
    Plain,
}
impl std::fmt::Display for Security {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ExplicitTls => "FTPS · STARTTLS",
            Self::ImplicitTls => "FTPS · TLS",
            Self::Plain => "FTP · unencrypted connection",
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub directory: String,
    pub security: Security,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 21,
            username: String::new(),
            directory: String::new(),
            security: Security::default(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub directory: String,
    pub security: Security,
}
impl Settings {
    pub fn identity(&self) -> Identity {
        Identity {
            host: normalized_host(&self.host).unwrap_or_else(|_| self.host.clone()),
            port: self.port,
            username: self.username.clone(),
            directory: if self.directory == "/" {
                "/".into()
            } else {
                self.directory.trim_end_matches('/').into()
            },
            security: self.security,
        }
    }
    pub(crate) fn secret_id(&self) -> String {
        format!(
            "backup-ftp:{:x}",
            Sha256::digest(serde_json::to_vec(&self.identity()).expect("FTP identity serializes"))
        )
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.host.is_empty()
                && self.host.len() <= 253
                && !self
                    .host
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace())
                && !self.host.contains(['/', '@', '\\'])
                && normalized_host(&self.host).is_ok(),
            "Enter an FTP hostname or IP address without a URL or username."
        );
        anyhow::ensure!(self.port != 0, "Enter an FTP port between 1 and 65535.");
        anyhow::ensure!(
            !self.username.is_empty()
                && self.username.len() <= 256
                && !self.username.chars().any(char::is_control),
            "Enter an FTP username without control characters."
        );
        anyhow::ensure!(
            self.directory.starts_with('/')
                && self.directory.len() <= 4096
                && !self.directory.contains(['\\', '\r', '\n'])
                && !self.directory.chars().any(char::is_control)
                && !self.directory.contains("//")
                && !self.directory.split('/').any(|p| matches!(p, "." | "..")),
            "Use an existing absolute FTP folder without dot segments or repeated slashes."
        );
        Ok(())
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
        complete.validate()
    }
}
fn normalized_host(host: &str) -> anyhow::Result<String> {
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        return Ok(ip.to_string());
    }
    Ok(url::Host::parse(host)?
        .to_string()
        .trim_end_matches('.')
        .into())
}
#[derive(Clone)]
pub struct FtpBackup {
    settings: Settings,
    password: SecretString,
    #[cfg(test)]
    ca: Option<Arc<[u8]>>,
}
impl FtpBackup {
    pub fn new(settings: &Settings, password: SecretString) -> anyhow::Result<Self> {
        settings.validate()?;
        anyhow::ensure!(
            !password.expose_secret().is_empty()
                && !password.expose_secret().chars().any(char::is_control),
            "Enter an FTP password without control characters."
        );
        Ok(Self {
            settings: settings.clone(),
            password,
            #[cfg(test)]
            ca: None,
        })
    }
    pub async fn test_connection(&self) -> anyhow::Result<()> {
        tokio::time::timeout(Duration::from_secs(30), self.list())
            .await
            .context("The FTP connection test timed out. Check the server and retry.")?
            .map(|_| ())
    }
    fn path(&self, relative: &str) -> String {
        format!(
            "{}/{relative}",
            self.settings.identity().directory.trim_end_matches('/')
        )
    }
    async fn request(&self, relative: &str, action: Action) -> anyhow::Result<Reply> {
        let provider = self.clone();
        let relative = relative.to_owned();
        let (live, receiver) = oneshot::channel::<()>();
        let task = tokio::task::spawn_blocking(move || provider.perform(&relative, action, live));
        let reply = task
            .await
            .context("The FTP worker could not complete the request.")?;
        drop(receiver);
        reply
    }
    fn perform(
        &self,
        relative: &str,
        action: Action,
        live: oneshot::Sender<()>,
    ) -> anyhow::Result<Reply> {
        let mut url = url::Url::parse(if self.settings.security == Security::ImplicitTls {
            "ftps://placeholder/"
        } else {
            "ftp://placeholder/"
        })?;
        let host = normalized_host(&self.settings.host)?;
        let url_host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        url.set_host(Some(&url_host))
            .map_err(|_| anyhow::anyhow!("Invalid FTP host."))?;
        url.set_port(Some(self.settings.port))
            .map_err(|_| anyhow::anyhow!("Invalid FTP port."))?;
        // A doubled leading slash makes the path absolute in FTP, including a
        // server whose login directory differs from the configured backup root.
        url.set_path(&format!("/{}", self.path(relative)));
        let (limit, source) = match &action {
            Action::Read(limit) => (*limit, None),
            Action::List => (LIST_LIMIT, None),
            Action::Write(data, _) => (MANIFEST_LIMIT, Some(data.clone())),
            Action::Commands(_) => (MANIFEST_LIMIT, None),
        };
        let mut easy = Easy2::new(Transfer {
            bytes: Vec::new(),
            limit,
            source,
            offset: 0,
            headers: 0,
            live,
            expected_directory: self.settings.identity().directory,
            cwd_accepted: false,
            canonical: false,
        });
        easy.url(url.as_str())?;
        easy.username(&self.settings.username)?;
        easy.password(self.password.expose_secret())?;
        easy.verbose(false)?;
        easy.proxy("")?;
        easy.follow_location(false)?;
        easy.connect_timeout(Duration::from_secs(15))?;
        easy.timeout(Duration::from_secs(300))?;
        easy.low_speed_limit(1)?;
        easy.low_speed_time(Duration::from_secs(30))?;
        easy.progress(true)?;
        easy.ssl_verify_peer(true)?;
        easy.ssl_verify_host(true)?;
        easy.ssl_version(curl::easy::SslVersion::Tlsv12)?;
        #[cfg(test)]
        if let Some(ca) = &self.ca {
            easy.ssl_cainfo_blob(ca)?;
        }
        set_long(
            &mut easy,
            curl_sys::CURLOPT_USE_SSL,
            if self.settings.security == Security::Plain {
                0
            } else {
                3
            },
        )?;
        // FTPS requires encryption for both control and data. Never fall back
        // to cleartext, and ignore server-supplied PASV addresses.
        set_long(&mut easy, curl_sys::CURLOPT_FTPSSLAUTH, 2)?;
        set_long(&mut easy, curl_sys::CURLOPT_FTP_SKIP_PASV_IP, 1)?;
        set_long(&mut easy, curl_sys::CURLOPT_FTP_FILEMETHOD, 2)?;
        let mut commands = vec![
            format!("CWD {}", self.settings.identity().directory),
            "PWD".into(),
        ];
        match action {
            Action::List => easy.custom_request("MLSD")?,
            Action::Read(_) => {}
            Action::Write(data, append) => {
                easy.upload(true)?;
                easy.in_filesize(data.len() as u64)?;
                set_long(&mut easy, curl_sys::CURLOPT_APPEND, i64::from(append))?;
            }
            Action::Commands(extra) => {
                easy.nobody(true)?;
                commands.extend(extra);
            }
        }
        let quote = Quote::new(&commands)?;
        // SAFETY: CURLOPT_QUOTE takes curl_slist*. The list lives through
        // perform; every request verifies CWD/PWD before its data or mutations.
        check_option(unsafe {
            curl_sys::curl_easy_setopt(easy.raw(), curl_sys::CURLOPT_QUOTE, quote.0)
        })?;
        let result = easy.perform();
        let code = easy.response_code().unwrap_or(0);
        if let Err(error) = result {
            if error.code() == curl_sys::CURLE_REMOTE_FILE_NOT_FOUND {
                return Ok(Reply::Missing);
            }
            anyhow::bail!(
                "FTP request failed (code {}, server {}). Check the connection, certificate and folder permissions, then retry.",
                error.code(),
                code
            );
        }
        anyhow::ensure!(
            easy.get_ref().canonical,
            "The FTP server did not confirm the backup folder's canonical path. Use its absolute path."
        );
        Ok(Reply::Bytes(std::mem::take(&mut easy.get_mut().bytes)))
    }
}
#[derive(Clone)]
enum Action {
    Read(usize),
    List,
    Write(Arc<[u8]>, bool),
    Commands(Vec<String>),
}
enum Reply {
    Missing,
    Bytes(Vec<u8>),
}
struct Transfer {
    bytes: Vec<u8>,
    limit: usize,
    source: Option<Arc<[u8]>>,
    offset: usize,
    headers: usize,
    live: oneshot::Sender<()>,
    expected_directory: String,
    cwd_accepted: bool,
    canonical: bool,
}
impl Handler for Transfer {
    fn write(&mut self, bytes: &[u8]) -> Result<usize, WriteError> {
        if self.bytes.len().saturating_add(bytes.len()) > self.limit {
            return Ok(0);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, ReadError> {
        let Some(source) = &self.source else {
            return Ok(0);
        };
        let count = bytes.len().min(source.len() - self.offset);
        bytes[..count].copy_from_slice(&source[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
    fn header(&mut self, bytes: &[u8]) -> bool {
        self.headers = self.headers.saturating_add(bytes.len());
        if !self.canonical {
            if bytes.starts_with(b"250 ") {
                self.cwd_accepted = true;
            }
            if self.cwd_accepted && bytes.starts_with(b"257 ") {
                let path = std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|line| line.strip_prefix("257 \""))
                    .and_then(|line| line.split_once('"').map(|(path, _)| path));
                self.canonical = path == Some(self.expected_directory.as_str());
                if !self.canonical {
                    return false;
                }
            }
        }
        self.headers <= 128 * 1024
    }
    fn progress(&mut self, _: f64, _: f64, _: f64, _: f64) -> bool {
        !self.live.is_closed()
    }
}
fn set_long(
    easy: &mut Easy2<Transfer>,
    option: curl_sys::CURLoption,
    value: i64,
) -> anyhow::Result<()> {
    // SAFETY: callers pass only CURLOPT options documented as C long values.
    check_option(unsafe {
        curl_sys::curl_easy_setopt(easy.raw(), option, value as std::ffi::c_long)
    })
}
fn check_option(code: curl_sys::CURLcode) -> anyhow::Result<()> {
    anyhow::ensure!(
        code == curl_sys::CURLE_OK,
        "This libcurl build does not support the required FTP option (code {code})."
    );
    Ok(())
}
struct Quote(*mut curl_sys::curl_slist);
impl Quote {
    fn new(commands: &[String]) -> anyhow::Result<Self> {
        let mut list = Self(std::ptr::null_mut());
        for command in commands {
            anyhow::ensure!(!command.contains(['\r', '\n']), "Invalid FTP command path.");
            let command = CString::new(command.as_str())?;
            // SAFETY: libcurl copies this NUL-terminated string into the list.
            let next = unsafe { curl_sys::curl_slist_append(list.0, command.as_ptr()) };
            anyhow::ensure!(!next.is_null(), "Could not allocate the FTP command.");
            list.0 = next;
        }
        Ok(list)
    }
}
impl Drop for Quote {
    fn drop(&mut self) {
        unsafe {
            curl_sys::curl_slist_free_all(self.0);
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: String,
    name: String,
    size: u64,
    sha256: String,
}
impl Manifest {
    fn from_upload(upload: &PreparedUpload) -> Self {
        Self {
            format: "shep-ftp-1".into(),
            name: upload.name.clone(),
            size: upload.size,
            sha256: upload.sha256.clone(),
        }
    }
    fn parse(bytes: &[u8], name: &str) -> anyhow::Result<Self> {
        let manifest: Self = serde_json::from_slice(bytes).map_err(|_| {
            anyhow::anyhow!("The FTP copy has an invalid commit record. It was kept unchanged.")
        })?;
        anyhow::ensure!(
            manifest.format == "shep-ftp-1"
                && manifest.name == name
                && valid_name(name)
                && (MAGIC.len() as u64..=MAX_DECODED).contains(&manifest.size)
                && manifest.sha256.len() == 64
                && manifest
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "The FTP copy is not an owned Shep backup. It was kept unchanged."
        );
        Ok(manifest)
    }
    fn matches(&self, upload: &PreparedUpload) -> bool {
        self.name == upload.name && self.size == upload.size && self.sha256 == upload.sha256
    }
    fn verify(&self, bytes: &[u8]) -> anyhow::Result<()> {
        anyhow::ensure!(
            bytes.starts_with(MAGIC)
                && bytes.len() as u64 == self.size
                && format!("{:x}", Sha256::digest(bytes)) == self.sha256,
            "The FTP copy has unexpected contents. It was kept unchanged."
        );
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
struct Creation {
    directory: String,
    created: bool,
}
struct Entry {
    name: String,
    directory: bool,
}
fn entries(bytes: &[u8]) -> anyhow::Result<Vec<Entry>> {
    let text =
        std::str::from_utf8(bytes).context("The FTP directory listing is not valid UTF-8.")?;
    let mut entries = Vec::new();
    let mut names = std::collections::HashSet::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        anyhow::ensure!(
            entries.len() < ENTRY_LIMIT,
            "The FTP directory listing is too large. Existing copies were kept."
        );
        let (facts, name) = line.split_once(' ').context("The FTP server did not return a machine-readable directory listing. Enable MLSD on the server.")?;
        let kind = facts
            .split(';')
            .find_map(|fact| {
                fact.split_once('=')
                    .filter(|(key, _)| key.eq_ignore_ascii_case("type"))
                    .map(|(_, value)| value.to_ascii_lowercase())
            })
            .context("The FTP directory listing omitted the entry type.")?;
        if matches!(kind.as_str(), "cdir" | "pdir") {
            continue;
        }
        anyhow::ensure!(
            !name.is_empty()
                && !name.contains(['/', '\\'])
                && !name.chars().any(char::is_control)
                && names.insert(name.to_string()),
            "The FTP directory listing is incomplete or contains duplicate paths. Existing copies were kept."
        );
        if matches!(kind.as_str(), "dir" | "file") {
            entries.push(Entry {
                name: name.into(),
                directory: kind == "dir",
            });
        }
    }
    Ok(entries)
}
impl FtpBackup {
    async fn bytes(&self, path: &str, limit: usize) -> anyhow::Result<Option<Vec<u8>>> {
        match self.request(path, Action::Read(limit)).await? {
            Reply::Missing => Ok(None),
            Reply::Bytes(bytes) => Ok(Some(bytes)),
        }
    }
    async fn entries(&self, path: &str) -> anyhow::Result<Vec<Entry>> {
        match self.request(path, Action::List).await? {
            Reply::Missing => {
                anyhow::bail!("The FTP backup folder was not found. Create it on the server first.")
            }
            Reply::Bytes(bytes) => entries(&bytes),
        }
    }
    async fn commands(&self, commands: Vec<String>) -> anyhow::Result<()> {
        match self.request("", Action::Commands(commands)).await? {
            Reply::Missing => anyhow::bail!("The FTP backup folder was not found."),
            Reply::Bytes(_) => Ok(()),
        }
    }
    async fn write(&self, path: &str, data: &[u8], append: bool) -> anyhow::Result<()> {
        match self
            .request(path, Action::Write(data.into(), append))
            .await?
        {
            Reply::Missing => anyhow::bail!("The FTP upload folder disappeared."),
            Reply::Bytes(_) => Ok(()),
        }
    }
    async fn manifest(&self, name: &str) -> anyhow::Result<Option<Manifest>> {
        anyhow::ensure!(valid_name(name), "Invalid FTP backup name.");
        self.bytes(&format!("{name}/{COMMIT}"), MANIFEST_LIMIT)
            .await?
            .map(|bytes| Manifest::parse(&bytes, name))
            .transpose()
    }
    async fn archive(&self, name: &str) -> anyhow::Result<Vec<u8>> {
        self.bytes(&format!("{name}/{ARCHIVE}"), MAX_DECODED as usize)
            .await?
            .context("The FTP backup file was not found. Its commit record was kept.")
    }
}
#[async_trait]
impl BackupProvider for FtpBackup {
    async fn reserve(&self, name: &str, data: &[u8]) -> anyhow::Result<PreparedUpload> {
        anyhow::ensure!(
            valid_name(name) && data.starts_with(MAGIC),
            "Invalid FTP backup reservation."
        );
        Ok(PreparedUpload::new(name.into(), name.into(), data))
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
            upload.id == upload.name && data.starts_with(MAGIC),
            "Invalid FTP backup reservation."
        );
        let encoded = serde_json::to_vec(&Manifest::from_upload(upload))?;
        let commit_path = format!("{}/{COMMIT}", upload.id);
        let prior_commit = self.bytes(&commit_path, MANIFEST_LIMIT).await?;
        if let Some(bytes) = &prior_commit {
            match Manifest::parse(bytes, &upload.id) {
                Ok(manifest) => {
                    anyhow::ensure!(
                        manifest.matches(upload),
                        "The existing FTP copy does not match this reserved upload. It was kept unchanged."
                    );
                    manifest.verify(&self.archive(&upload.id).await?)?;
                    return Ok(());
                }
                Err(error) => {
                    let owned = upload
                        .session
                        .as_deref()
                        .and_then(|value| serde_json::from_str::<Creation>(value).ok())
                        .is_some_and(|receipt| receipt.created && receipt.directory == upload.id);
                    if !owned || bytes.len() >= encoded.len() || !encoded.starts_with(bytes) {
                        return Err(error);
                    }
                }
            }
        }
        let listed = self.entries("").await?;
        if let Some(existing) = listed.iter().find(|entry| entry.name == upload.id) {
            anyhow::ensure!(
                existing.directory,
                "The FTP backup path is not a directory. It was kept unchanged."
            );
            let creation: Creation = serde_json::from_str(upload.session.as_deref().context("An unconfirmed FTP upload folder already exists. Inspect it before retrying; no file was overwritten.")?)?;
            anyhow::ensure!(
                creation.created && creation.directory == upload.id,
                "The FTP creation receipt does not match the pending copy."
            );
            let contents = self.entries(&format!("{}/", upload.id)).await?;
            anyhow::ensure!(
                contents.iter().all(
                    |entry| !entry.directory && matches!(entry.name.as_str(), ARCHIVE | COMMIT)
                ),
                "The pending FTP folder contains unexpected files. It was kept unchanged."
            );
        } else {
            // FTP has no conditional STOR/rename. Reserve an exclusive directory,
            // then durably record ownership before creating any contents in it.
            self.commands(vec![format!("MKD {}", self.path(&upload.id))])
                .await?;
            upload.session = Some(serde_json::to_string(&Creation {
                directory: upload.id.clone(),
                created: true,
            })?);
            checkpoint.save(upload).await?;
        }
        let path = format!("{}/{ARCHIVE}", upload.id);
        let previous = self.bytes(&path, MAX_DECODED as usize).await?;
        let offset = previous.as_ref().map_or(0, Vec::len);
        anyhow::ensure!(
            data.get(..offset) == previous.as_deref().or(Some(&[])),
            "The pending FTP upload contains unexpected data. It was kept unchanged."
        );
        if offset < data.len() {
            self.write(&path, &data[offset..], previous.is_some())
                .await?;
        }
        let manifest = Manifest::from_upload(upload);
        manifest.verify(&self.archive(&upload.id).await?)?;
        let commit_offset = prior_commit.as_ref().map_or(0, Vec::len);
        let write = self
            .write(
                &commit_path,
                &encoded[commit_offset..],
                prior_commit.is_some(),
            )
            .await;
        if let Some(committed) = self.manifest(&upload.id).await? {
            anyhow::ensure!(
                committed.matches(upload),
                "The FTP commit record changed unexpectedly. Inspect this pending copy."
            );
            committed.verify(&self.archive(&upload.id).await?)?;
            return Ok(());
        }
        write?;
        anyhow::bail!("Could not confirm the FTP copy. Retry to recover the same reserved upload.")
    }
    async fn verify_upload(&self, upload: &PreparedUpload) -> anyhow::Result<bool> {
        let Some(manifest) = self.manifest(&upload.id).await? else {
            return Ok(false);
        };
        anyhow::ensure!(
            manifest.matches(upload),
            "The FTP copy differs from its upload receipt."
        );
        manifest.verify(&self.archive(&upload.id).await?)?;
        Ok(true)
    }
    async fn list(&self) -> anyhow::Result<Vec<BackupCopy>> {
        let mut copies = Vec::new();
        for entry in self.entries("").await? {
            if !entry.directory || !valid_name(&entry.name) {
                continue;
            }
            if self.manifest(&entry.name).await?.is_some() {
                copies.push(BackupCopy {
                    id: entry.name.clone(),
                    name: entry.name.clone(),
                    created_at: entry.name[5..21].into(),
                });
            }
        }
        copies.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(copies)
    }
    async fn download(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        let manifest = self
            .manifest(id)
            .await?
            .context("This FTP copy is not committed. Its pending data was kept.")?;
        let bytes = self.archive(id).await?;
        manifest.verify(&bytes)?;
        Ok(bytes)
    }
    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.download(id).await?;
        let contents = self.entries(&format!("{id}/")).await?;
        anyhow::ensure!(
            contents.len() == 2
                && contents.iter().all(
                    |entry| !entry.directory && matches!(entry.name.as_str(), ARCHIVE | COMMIT)
                ),
            "This FTP backup folder contains unrelated files. Nothing was deleted."
        );
        self.commands(vec![
            format!("DELE {}", self.path(&format!("{id}/{COMMIT}"))),
            format!("DELE {}", self.path(&format!("{id}/{ARCHIVE}"))),
            format!("RMD {}", self.path(id)),
        ])
        .await
    }
}

#[cfg(test)]
#[path = "ftp_tests.rs"]
pub(crate) mod tests;
