//! S3 transport. The caller owns credentials and the durable upload journal.
use super::{
    BackupCopy, BackupProvider, MAX_DECODED, PreparedUpload, UploadCheckpoint, valid_name,
};
use anyhow::Context;
use async_trait::async_trait;
use base64::Engine as _;
use reqwest::{Response, StatusCode};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, time::Duration};

const XML_NAMESPACE: &str = "http://s3.amazonaws.com/doc/2006-03-01/";
const LIST_LIMIT: usize = 2 * 1024 * 1024;
const SIGN_DURATION: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub prefix: String,
    pub path_style: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            endpoint: "https://s3.eu-west-1.amazonaws.com".into(),
            region: "eu-west-1".into(),
            bucket: String::new(),
            prefix: "shep".into(),
            path_style: true,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
}
impl Identity {
    pub(crate) fn secret_id(&self) -> String {
        format!(
            "backup-s3:{:x}",
            sha2::Sha256::digest(serde_json::to_vec(self).expect("S3 identity serializes"))
        )
    }
}

#[derive(Serialize, Deserialize, zeroize::ZeroizeOnDrop)]
struct Access {
    key: String,
    secret: String,
}
pub fn access_secret(key: String, secret: String) -> anyhow::Result<secrecy::SecretString> {
    anyhow::ensure!(
        !key.trim().is_empty() && !secret.is_empty(),
        "Enter both the S3 access key and secret key."
    );
    Ok(secrecy::SecretString::from(serde_json::to_string(
        &Access { key, secret },
    )?))
}

impl Settings {
    pub fn identity(&self) -> Identity {
        Identity {
            endpoint: url::Url::parse(&self.endpoint)
                .map(|u| u.to_string())
                .unwrap_or_else(|_| self.endpoint.clone()),
            bucket: self.bucket.clone(),
            prefix: self.prefix.trim_end_matches('/').into(),
        }
    }
    pub(crate) fn validate_draft(&self) -> anyhow::Result<()> {
        let mut complete = self.clone();
        if complete.bucket.is_empty() {
            complete.bucket = "unconfigured".into();
        }
        complete.validate()
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        self.checked_endpoint(false).map(|_| ())
    }
    fn checked_endpoint(&self, fixture: bool) -> anyhow::Result<url::Url> {
        let endpoint = url::Url::parse(&self.endpoint).context("Enter a valid S3 endpoint URL.")?;
        anyhow::ensure!(
            endpoint.scheme() == "https"
                || (fixture
                    && endpoint.scheme() == "http"
                    && endpoint.host_str() == Some("127.0.0.1")),
            "S3 endpoints must use HTTPS."
        );
        anyhow::ensure!(
            endpoint.host_str().is_some()
                && endpoint.username().is_empty()
                && endpoint.password().is_none()
                && endpoint.query().is_none()
                && endpoint.fragment().is_none()
                && endpoint.path() == "/",
            "Use an S3 server URL without credentials, query or path."
        );
        anyhow::ensure!(
            (3..=63).contains(&self.bucket.len())
                && self
                    .bucket
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-.".contains(&b))
                && !self.bucket.starts_with(['-', '.'])
                && !self.bucket.ends_with(['-', '.'])
                && !self.bucket.contains(".."),
            "Enter a valid S3 bucket name."
        );
        anyhow::ensure!(
            !self.region.is_empty()
                && self.region.len() <= 64
                && self
                    .region
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "Enter the S3 signing region."
        );
        anyhow::ensure!(
            self.prefix.len() <= 512
                && !self.prefix.starts_with('/')
                && self
                    .prefix
                    .split('/')
                    .all(|part| part != "." && part != "..")
                && !self.prefix.chars().any(|c| c.is_control() || c == '\\'),
            "Use a relative S3 folder prefix without dot segments or control characters."
        );
        Ok(endpoint)
    }
    fn prefix(&self) -> String {
        if self.prefix.is_empty() {
            String::new()
        } else {
            format!("{}/", self.prefix.trim_end_matches('/'))
        }
    }
}

pub struct S3Backup {
    http: reqwest::Client,
    bucket: Bucket,
    credentials: Credentials,
    prefix: String,
}
impl S3Backup {
    #[cfg(test)]
    pub(crate) fn fixture(
        settings: &Settings,
        secret: &secrecy::SecretString,
    ) -> anyhow::Result<Self> {
        use secrecy::ExposeSecret;
        let access: Access = serde_json::from_str(secret.expose_secret())?;
        Self::configured(settings, access.key.clone(), access.secret.clone(), true)
    }
    pub fn from_secret(
        settings: &Settings,
        secret: &secrecy::SecretString,
    ) -> anyhow::Result<Self> {
        use secrecy::ExposeSecret;
        let access: Access = serde_json::from_str(secret.expose_secret()).map_err(|_| {
            anyhow::anyhow!(
                "Saved S3 credentials could not be read. Test and save this connection again."
            )
        })?;
        Self::new(settings, access.key.clone(), access.secret.clone())
    }
    pub fn new(settings: &Settings, key: String, secret: String) -> anyhow::Result<Self> {
        Self::configured(settings, key, secret, false)
    }
    fn configured(
        settings: &Settings,
        key: String,
        secret: String,
        fixture: bool,
    ) -> anyhow::Result<Self> {
        let endpoint = settings.checked_endpoint(fixture)?;
        anyhow::ensure!(
            !key.trim().is_empty()
                && !secret.is_empty()
                && key.len() <= 512
                && secret.len() <= 4096,
            "Enter the S3 access key and secret key."
        );
        Ok(Self {
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(120))
                .build()?,
            bucket: Bucket::new(
                endpoint,
                if settings.path_style {
                    UrlStyle::Path
                } else {
                    UrlStyle::VirtualHost
                },
                settings.bucket.clone(),
                settings.region.clone(),
            )?,
            credentials: Credentials::new(key, secret),
            prefix: settings.prefix(),
        })
    }
    fn name<'a>(&self, id: &'a str) -> anyhow::Result<&'a str> {
        let name = id
            .strip_prefix(&self.prefix)
            .context("This object is outside the configured backup folder.")?;
        anyhow::ensure!(
            valid_name(name),
            "Choose a Shep backup object in this folder."
        );
        Ok(name)
    }
    async fn send(&self, request: reqwest::RequestBuilder) -> anyhow::Result<Response> {
        request.send().await.map_err(|_| anyhow::anyhow!("The S3 request could not complete. Check the connection and retry; its pending copy is kept."))
    }
    async fn head(&self, id: &str) -> anyhow::Result<Option<Metadata>> {
        self.name(id)?;
        let action = self.bucket.head_object(Some(&self.credentials), id);
        let response = self
            .send(self.http.head(action.sign(SIGN_DURATION)))
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        ensure_success(&response)?;
        let headers = response.headers();
        anyhow::ensure!(
            headers
                .get("x-amz-meta-shep-backup")
                .is_some_and(|v| v == "1"),
            "This S3 object is not owned by Shep. It was not changed."
        );
        let size = headers
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .context("S3 returned invalid backup size metadata.")?;
        anyhow::ensure!(
            size > 0 && size <= MAX_DECODED,
            "The S3 backup exceeds the restore size limit."
        );
        let sha256 = headers
            .get("x-amz-meta-shep-sha256")
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
            .context("S3 returned invalid backup checksum metadata.")?
            .to_owned();
        let etag = headers
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.is_empty())
            .context("S3 omitted the object identity needed for safe access.")?
            .to_owned();
        Ok(Some(Metadata { size, sha256, etag }))
    }
    async fn bytes(&self, id: &str, metadata: &Metadata) -> anyhow::Result<Vec<u8>> {
        let mut action = self.bucket.get_object(Some(&self.credentials), id);
        action
            .headers_mut()
            .insert("if-match", metadata.etag.as_str());
        let response = self
            .send(
                self.http
                    .get(action.sign(SIGN_DURATION))
                    .header("If-Match", &metadata.etag),
            )
            .await?;
        let bytes = bounded(response, metadata.size as usize).await?;
        PreparedUpload {
            id: id.into(),
            name: self.name(id)?.into(),
            size: metadata.size,
            sha256: metadata.sha256.clone(),
            session: None,
        }
        .verify(&bytes)?;
        Ok(bytes)
    }
    pub async fn test_connection(&self) -> anyhow::Result<()> {
        self.list().await.map(|_| ())
    }
}
struct Metadata {
    size: u64,
    sha256: String,
    etag: String,
}
fn ensure_success(response: &Response) -> anyhow::Result<()> {
    anyhow::ensure!(
        response.status().is_success(),
        "S3 rejected the request (HTTP {}). Check the endpoint, region and bucket permissions.",
        response.status()
    );
    Ok(())
}
async fn bounded(mut response: Response, limit: usize) -> anyhow::Result<Vec<u8>> {
    ensure_success(&response)?;
    anyhow::ensure!(
        response.content_length().is_none_or(|n| n <= limit as u64),
        "S3 returned a response larger than expected."
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("S3 returned an incomplete response. Retry this operation."))?
    {
        anyhow::ensure!(
            bytes.len().saturating_add(chunk.len()) <= limit,
            "S3 returned a response larger than expected."
        );
        bytes.extend(chunk);
    }
    Ok(bytes)
}

#[async_trait]
impl BackupProvider for S3Backup {
    async fn reserve(&self, name: &str, data: &[u8]) -> anyhow::Result<PreparedUpload> {
        anyhow::ensure!(valid_name(name), "Invalid backup filename.");
        let upload = PreparedUpload::new(format!("{}{name}", self.prefix), name.into(), data);
        upload.verify(data)?;
        Ok(upload)
    }
    async fn upload_prepared(
        &self,
        upload: &mut PreparedUpload,
        data: &[u8],
        _: &dyn UploadCheckpoint,
    ) -> anyhow::Result<()> {
        upload.verify(data)?;
        anyhow::ensure!(
            self.name(&upload.id)? == upload.name,
            "The reserved S3 object does not match this backup."
        );
        if self.verify_upload(upload).await? {
            return Ok(());
        }
        let checksum = base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(data));
        let mut action = self.bucket.put_object(Some(&self.credentials), &upload.id);
        for (name, value) in [
            ("if-none-match", "*"),
            ("x-amz-meta-shep-backup", "1"),
            ("x-amz-meta-shep-sha256", upload.sha256.as_str()),
            ("x-amz-checksum-sha256", checksum.as_str()),
        ] {
            action.headers_mut().insert(name, value);
        }
        let sent = self
            .send(
                self.http
                    .put(action.sign(SIGN_DURATION))
                    .header("If-None-Match", "*")
                    .header("x-amz-meta-shep-backup", "1")
                    .header("x-amz-meta-shep-sha256", &upload.sha256)
                    .header("x-amz-checksum-sha256", checksum)
                    .body(data.to_vec()),
            )
            .await;
        // Even after a lost reply or 412, only the exact staged ciphertext is success.
        if self.verify_upload(upload).await? {
            return Ok(());
        }
        match sent {
            Ok(response) => {
                ensure_success(&response)?;
                anyhow::bail!(
                    "S3 did not expose the committed backup. Its reserved copy is kept for retry."
                );
            }
            Err(error) => Err(error),
        }
    }
    async fn upload(&self, name: &str, data: Vec<u8>) -> anyhow::Result<String> {
        let mut upload = self.reserve(name, &data).await?;
        self.upload_prepared(&mut upload, &data, &super::NoCheckpoint)
            .await?;
        Ok(upload.id)
    }
    async fn verify_upload(&self, upload: &PreparedUpload) -> anyhow::Result<bool> {
        let Some(metadata) = self.head(&upload.id).await? else {
            return Ok(false);
        };
        anyhow::ensure!(
            metadata.size == upload.size && metadata.sha256 == upload.sha256,
            "The reserved S3 object contains a different backup. It was not replaced."
        );
        upload.verify(&self.bytes(&upload.id, &metadata).await?)?;
        Ok(true)
    }
    async fn download(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        let metadata = self
            .head(id)
            .await?
            .context("This backup is no longer in S3. Refresh saved copies.")?;
        self.bytes(id, &metadata).await
    }
    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        let Some(metadata) = self.head(id).await? else {
            return Ok(());
        };
        let mut action = self.bucket.delete_object(Some(&self.credentials), id);
        action
            .headers_mut()
            .insert("if-match", metadata.etag.as_str());
        let response = self
            .send(
                self.http
                    .delete(action.sign(SIGN_DURATION))
                    .header("If-Match", metadata.etag),
            )
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        ensure_success(&response)
    }
    async fn list(&self) -> anyhow::Result<Vec<BackupCopy>> {
        let mut token = String::new();
        let mut pages = HashSet::new();
        let mut keys = HashSet::new();
        let mut copies = Vec::new();
        loop {
            anyhow::ensure!(
                pages.len() < 100 && pages.insert(token.clone()),
                "S3 returned incomplete or repeated pages. Existing backups were kept."
            );
            let mut action = self.bucket.list_objects_v2(Some(&self.credentials));
            action.query_mut().insert("prefix", self.prefix.as_str());
            action.query_mut().insert("max-keys", "100");
            if !token.is_empty() {
                action
                    .query_mut()
                    .insert("continuation-token", token.as_str());
            }
            let bytes = bounded(
                self.send(self.http.get(action.sign(SIGN_DURATION))).await?,
                LIST_LIMIT,
            )
            .await?;
            let xml = std::str::from_utf8(&bytes).context("S3 returned an invalid object list.")?;
            let doc =
                roxmltree::Document::parse(xml).context("S3 returned an invalid object list.")?;
            let root = doc.root_element();
            anyhow::ensure!(
                root.has_tag_name((XML_NAMESPACE, "ListBucketResult")),
                "S3 returned an unexpected object list."
            );
            let field = |name| {
                root.children()
                    .find(|n| n.has_tag_name((XML_NAMESPACE, name)))
                    .and_then(|n| n.text())
            };
            anyhow::ensure!(
                field("EncodingType") == Some("url"),
                "S3 did not confirm URL-encoded object names."
            );
            for node in root
                .children()
                .filter(|n| n.has_tag_name((XML_NAMESPACE, "Contents")))
            {
                let value = |name| {
                    node.children()
                        .find(|n| n.has_tag_name((XML_NAMESPACE, name)))
                        .and_then(|n| n.text())
                };
                let encoded = value("Key").context("S3 omitted an object key.")?;
                let id = percent_encoding::percent_decode_str(encoded)
                    .decode_utf8()
                    .context("S3 returned an invalid object key.")?
                    .into_owned();
                anyhow::ensure!(
                    keys.insert(id.clone()),
                    "S3 returned duplicate objects. Existing backups were kept."
                );
                if let Ok(name) = self.name(&id) {
                    let name = name.to_owned();
                    if self.head(&id).await?.is_some() {
                        copies.push(BackupCopy {
                            id,
                            name,
                            created_at: value("LastModified").unwrap_or_default().into(),
                        });
                    }
                }
            }
            match field("IsTruncated") {
                Some("false") => break,
                Some("true") => {
                    token = field("NextContinuationToken")
                        .filter(|s| !s.is_empty() && s.len() <= 4096)
                        .context("S3 omitted its next page token. Existing backups were kept.")?
                        .into()
                }
                _ => anyhow::bail!(
                    "S3 did not confirm a complete object list. Existing backups were kept."
                ),
            }
        }
        copies.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(copies)
    }
}
use sha2::Digest as _;

#[cfg(test)]
#[path = "s3_tests.rs"]
mod tests;
