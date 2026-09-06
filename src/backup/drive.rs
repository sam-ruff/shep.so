use super::*;
use reqwest::{Response, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
#[cfg(test)]
#[path = "drive_tests.rs"]
mod tests;

const FILE_FIELDS: &str =
    "id,name,createdTime,trashed,spaces,mimeType,appProperties,size,sha256Checksum";
const JSON_LIMIT: u64 = 2 * 1024 * 1024;
const CHUNK: usize = 1024 * 1024;

// Only the production wrapper chooses credentials and endpoints. Contract tests
// borrow the same protocol implementation with a loopback client and fixture token.
struct Api<'a> {
    http: &'a reqwest::Client,
    base: url::Url,
    token: &'a str,
}
impl Api<'_> {
    fn url(&self, path: &str) -> anyhow::Result<url::Url> {
        Ok(self.base.join(path)?)
    }
    fn file_url(&self, id: &str) -> anyhow::Result<url::Url> {
        anyhow::ensure!(valid_id(id), "Invalid Drive file ID.");
        self.url(&format!("/drive/v3/files/{id}"))
    }
    async fn identity(&self) -> anyhow::Result<String> {
        let value = response_json(
            self.http
                .get(self.url("/drive/v3/about")?)
                .bearer_auth(self.token)
                .query(&[("fields", "user(permissionId)")])
                .send()
                .await?,
        )
        .await?;
        let id = value["user"]["permissionId"]
            .as_str()
            .filter(|id| valid_id(id))
            .context("Google Drive did not identify the connected account")?;
        Ok(format!("drive:{id}"))
    }
    async fn list(&self) -> anyhow::Result<Vec<BackupCopy>> {
        let mut next = String::new();
        let mut pages = HashSet::new();
        let mut ids = HashSet::new();
        let mut copies = Vec::new();
        loop {
            anyhow::ensure!(
                pages.len() < 100 && pages.insert(next.clone()),
                "Google Drive repeated a page token or exceeded 100 pages. Older copies were kept."
            );
            let value = response_json(self.http.get(self.url("/drive/v3/files")?).bearer_auth(self.token).query(&[
                ("spaces", "appDataFolder"), ("q", "trashed = false and appProperties has { key='shepBackup' and value='1' }"),
                ("fields", &format!("nextPageToken,incompleteSearch,files({FILE_FIELDS})")), ("pageSize", "100"), ("pageToken", &next),
            ]).send().await?).await?;
            anyhow::ensure!(
                value["incompleteSearch"] != true,
                "Google Drive returned an incomplete backup list. Older copies were kept."
            );
            if let Some(files) = value.get("files") {
                for file in files
                    .as_array()
                    .context("Invalid Google Drive backup list")?
                {
                    // Other application data must never become a retention candidate.
                    if !file["name"].as_str().is_some_and(valid_name) {
                        continue;
                    }
                    owned(file)?;
                    let id = file["id"].as_str().unwrap();
                    anyhow::ensure!(
                        ids.insert(id.to_owned()),
                        "Google Drive returned duplicate file IDs. Older copies were kept."
                    );
                    copies.push(BackupCopy {
                        id: id.into(),
                        name: file["name"].as_str().unwrap().into(),
                        created_at: file["createdTime"].as_str().unwrap_or("").into(),
                    });
                }
            }
            match value.get("nextPageToken") {
                None | Some(Value::Null) => break,
                Some(Value::String(token)) if token.is_empty() => break,
                Some(Value::String(token)) => next = token.clone(),
                _ => anyhow::bail!("Invalid Google Drive page token. Older copies were kept."),
            }
        }
        copies.sort_by(|a, b| b.name.cmp(&a.name));
        Ok(copies)
    }
    async fn metadata(&self, id: &str) -> anyhow::Result<Option<Value>> {
        let response = self
            .http
            .get(self.file_url(id)?)
            .bearer_auth(self.token)
            .query(&[("fields", FILE_FIELDS)])
            .send()
            .await?;
        if matches!(response.status(), StatusCode::NOT_FOUND | StatusCode::GONE) {
            return Ok(None);
        }
        let value = response_json(response).await?;
        owned(&value)?;
        anyhow::ensure!(
            value["id"] == id,
            "Google Drive returned a different file identity."
        );
        Ok(Some(value))
    }
    async fn media(&self, id: &str, size: u64) -> anyhow::Result<Vec<u8>> {
        anyhow::ensure!(
            size <= MAX_DECODED,
            "The backup exceeds the restore size limit."
        );
        let bytes = response_bytes(
            self.http
                .get(self.file_url(id)?)
                .bearer_auth(self.token)
                .query(&[("alt", "media")])
                .send()
                .await?,
            size,
        )
        .await?;
        anyhow::ensure!(
            bytes.len() as u64 == size,
            "Google Drive returned an incomplete backup. Try downloading it again."
        );
        Ok(bytes)
    }
    async fn download(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        let file = self
            .metadata(id)
            .await?
            .context("This backup is no longer on Google Drive. Refresh copies.")?;
        let bytes = self.media(id, file_size(&file)?).await?;
        if let Some(digest) = file["sha256Checksum"].as_str() {
            anyhow::ensure!(
                digest == format!("{:x}", Sha256::digest(&bytes)),
                "The downloaded backup checksum does not match Google Drive."
            );
        }
        Ok(bytes)
    }
    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        if self.metadata(id).await?.is_none() {
            return Ok(());
        }
        let response = self
            .http
            .delete(self.file_url(id)?)
            .bearer_auth(self.token)
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success()
                || matches!(response.status(), StatusCode::NOT_FOUND | StatusCode::GONE),
            "Google Drive did not confirm backup deletion (HTTP {}).",
            response.status()
        );
        Ok(())
    }
    async fn reserve(&self, name: &str, data: &[u8]) -> anyhow::Result<PreparedUpload> {
        anyhow::ensure!(valid_name(name), "Invalid backup filename.");
        let value = response_json(
            self.http
                .get(self.url("/drive/v3/files/generateIds")?)
                .bearer_auth(self.token)
                .query(&[
                    ("count", "1"),
                    ("space", "appDataFolder"),
                    ("type", "files"),
                ])
                .send()
                .await?,
        )
        .await?;
        anyhow::ensure!(
            value["space"] == "appDataFolder",
            "Google Drive reserved an ID in an unexpected space."
        );
        let ids = value["ids"]
            .as_array()
            .context("Google Drive did not reserve a backup ID")?;
        anyhow::ensure!(
            ids.len() == 1,
            "Google Drive did not reserve exactly one backup ID."
        );
        let id = ids[0]
            .as_str()
            .filter(|id| valid_id(id))
            .context("Invalid reserved Drive ID")?;
        Ok(PreparedUpload::new(id.into(), name.into(), data))
    }
    async fn confirmed(&self, upload: &PreparedUpload) -> anyhow::Result<bool> {
        let Some(file) = self.metadata(&upload.id).await? else {
            return Ok(false);
        };
        self.matches_upload(&file, upload).await?;
        Ok(true)
    }
    async fn matches_upload(&self, file: &Value, upload: &PreparedUpload) -> anyhow::Result<()> {
        owned(file)?;
        anyhow::ensure!(
            file["id"] == upload.id
                && file["name"] == upload.name
                && file_size(file)? == upload.size
                && file["appProperties"]["shepSha256"] == upload.sha256,
            "The reserved Drive file does not match this backup. It was not replaced."
        );
        if let Some(digest) = file["sha256Checksum"].as_str() {
            anyhow::ensure!(
                digest == upload.sha256,
                "Google Drive stored different backup bytes. The pending archive was kept."
            );
        } else {
            // Drive documents checksums as optional. Verify ciphertext directly
            // when it has not supplied a server checksum yet.
            upload.verify(&self.media(&upload.id, upload.size).await?)?;
        }
        Ok(())
    }
    fn session_url(&self, value: &str) -> anyhow::Result<url::Url> {
        let url = self.base.join(value)?;
        anyhow::ensure!(
            url.origin() == self.base.origin()
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
                && url.path() == "/upload/drive/v3/files"
                && url
                    .query_pairs()
                    .any(|(key, value)| key == "uploadType" && value == "resumable")
                && url
                    .query_pairs()
                    .any(|(key, value)| key == "upload_id" && !value.is_empty()),
            "Google Drive returned an untrusted upload session URL. No archive bytes were sent."
        );
        Ok(url)
    }
    async fn finish_response(
        &self,
        response: Response,
        upload: &PreparedUpload,
    ) -> anyhow::Result<()> {
        if let Ok(value) = response_json(response).await
            && self.matches_upload(&value, upload).await.is_ok()
        {
            return Ok(());
        }
        anyhow::ensure!(
            self.confirmed(upload).await?,
            "Google Drive has not confirmed the uploaded archive. Retry this pending copy."
        );
        Ok(())
    }
    async fn upload(
        &self,
        upload: &mut PreparedUpload,
        data: &[u8],
        checkpoint: &dyn UploadCheckpoint,
    ) -> anyhow::Result<()> {
        upload.verify(data)?;
        anyhow::ensure!(valid_id(&upload.id), "Invalid reserved Drive ID.");
        if self.confirmed(upload).await? {
            return Ok(());
        }
        let mut restarts = 0;
        let mut stalled = 0;
        let mut offset = 0usize;
        let mut probe = upload.session.is_some();
        loop {
            anyhow::ensure!(
                stalled < 4,
                "Google Drive upload stopped making progress. Retry to resume the saved pending copy."
            );
            if upload.session.is_none() {
                anyhow::ensure!(
                    restarts < 3,
                    "Google Drive repeatedly expired the upload session. Retry the saved pending copy later."
                );
                restarts += 1;
                let response = self.http.post(self.url("/upload/drive/v3/files")?).bearer_auth(self.token)
                    .query(&[("uploadType", "resumable"), ("fields", FILE_FIELDS)])
                    .header("X-Upload-Content-Type", "application/octet-stream").header("X-Upload-Content-Length", upload.size)
                    .json(&json!({"id":upload.id,"name":upload.name,"mimeType":"application/octet-stream","parents":["appDataFolder"],"appProperties":{"shepBackup":"1","shepSha256":upload.sha256}}))
                    .send().await?;
                if response.status() == StatusCode::CONFLICT {
                    anyhow::ensure!(
                        self.confirmed(upload).await?,
                        "The reserved Drive ID is not yet readable. Retry the pending copy."
                    );
                    return Ok(());
                }
                anyhow::ensure!(
                    response.status() == StatusCode::OK,
                    "Google Drive could not start the pending upload (HTTP {}).",
                    response.status()
                );
                let location = response
                    .headers()
                    .get("location")
                    .context("Google Drive did not return an upload session")?
                    .to_str()?;
                upload.session = Some(self.session_url(location)?.into());
                checkpoint.save(upload).await?;
                offset = 0;
                probe = false;
            }
            let session = self.session_url(upload.session.as_deref().unwrap())?;
            let end = (offset + CHUNK).min(data.len());
            let request = self.http.put(session).bearer_auth(self.token);
            let response = if probe || offset == data.len() {
                request
                    .header("Content-Length", 0)
                    .header("Content-Range", format!("bytes */{}", data.len()))
                    .send()
                    .await
            } else {
                request
                    .header("Content-Type", "application/octet-stream")
                    .header(
                        "Content-Range",
                        format!("bytes {offset}-{}/{}", end - 1, data.len()),
                    )
                    .body(data[offset..end].to_vec())
                    .send()
                    .await
            };
            let response = match response {
                Ok(response) => response,
                Err(_) => {
                    stalled += 1;
                    probe = true;
                    continue;
                }
            };
            match response.status() {
                StatusCode::OK | StatusCode::CREATED => {
                    return self.finish_response(response, upload).await;
                }
                StatusCode::PERMANENT_REDIRECT => {
                    let received = received_offset(
                        response
                            .headers()
                            .get("range")
                            .map(|value| value.to_str())
                            .transpose()?,
                        data.len(),
                    )?;
                    anyhow::ensure!(
                        received >= offset,
                        "Google Drive forgot previously acknowledged bytes. The pending archive was kept."
                    );
                    anyhow::ensure!(
                        received <= if probe { data.len() } else { end },
                        "Google Drive acknowledged bytes that were not sent. The pending archive was kept."
                    );
                    if received > offset {
                        stalled = 0;
                    } else {
                        stalled += 1;
                    }
                    offset = received;
                    probe = offset == data.len();
                }
                StatusCode::NOT_FOUND | StatusCode::GONE => {
                    if self.confirmed(upload).await? {
                        return Ok(());
                    }
                    upload.session = None;
                    checkpoint.save(upload).await?;
                    stalled = 0;
                }
                status if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS => {
                    stalled += 1;
                    probe = true;
                    tokio::time::sleep(std::time::Duration::from_millis(250 * stalled)).await;
                }
                status => anyhow::bail!(
                    "Google Drive paused the upload (HTTP {status}). The pending copy can be retried."
                ),
            }
        }
    }
}

fn received_offset(range: Option<&str>, size: usize) -> anyhow::Result<usize> {
    let Some(range) = range else {
        return Ok(0);
    };
    let end: usize = range
        .strip_prefix("bytes=0-")
        .context("Invalid Drive upload acknowledgment range")?
        .parse()?;
    anyhow::ensure!(
        end < size,
        "Google Drive acknowledged bytes outside the pending archive."
    );
    Ok(end + 1)
}
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 512
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn owned(file: &Value) -> anyhow::Result<()> {
    anyhow::ensure!(
        file["id"].as_str().is_some_and(valid_id)
            && file["name"].as_str().is_some_and(valid_name)
            && file["trashed"] == false
            && file["appProperties"]["shepBackup"] == "1"
            && file["spaces"]
                .as_array()
                .is_some_and(|spaces| spaces.iter().any(|space| space == "appDataFolder"))
            && file["mimeType"] == "application/octet-stream",
        "This Drive file is not an owned Shep backup in app data. It was not changed."
    );
    Ok(())
}
fn file_size(file: &Value) -> anyhow::Result<u64> {
    Ok(file["size"]
        .as_str()
        .context("Google Drive omitted the backup size")?
        .parse()?)
}
async fn response_bytes(mut response: Response, limit: u64) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        response.status().is_success(),
        "Google Drive request failed (HTTP {}).",
        response.status()
    );
    anyhow::ensure!(
        response.content_length().is_none_or(|size| size <= limit),
        "Google Drive response exceeds the size limit."
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() as u64 + chunk.len() as u64 <= limit,
            "Google Drive response exceeds the size limit."
        );
        bytes.extend(chunk);
    }
    Ok(bytes)
}
async fn response_json(response: Response) -> anyhow::Result<Value> {
    Ok(serde_json::from_slice(
        &response_bytes(response, JSON_LIMIT).await?,
    )?)
}

impl DriveBackup {
    pub fn new(google: Google, preferences: Preferences) -> Self {
        Self {
            google,
            preferences,
            verified: Default::default(),
        }
    }
    fn api<'a>(&'a self, token: &'a SecretString) -> Api<'a> {
        Api {
            http: &self.google.http,
            base: url::Url::parse("https://www.googleapis.com/").expect("official Drive URL"),
            token: token.expose_secret(),
        }
    }
    pub(crate) async fn account_identity(&self) -> anyhow::Result<String> {
        let token = self.google.token(&self.preferences).await?;
        self.api(&token).identity().await
    }
    async fn verified_token(&self) -> anyhow::Result<SecretString> {
        let token = self.google.token(&self.preferences).await?;
        self.verified
            .get_or_try_init(|| async {
                anyhow::ensure!(
                    self.api(&token).identity().await? == self.preferences.google_connection_id,
                    "Reconnect Google in Preferences to verify this backup account."
                );
                Ok::<_, anyhow::Error>(())
            })
            .await?;
        Ok(token)
    }
}
#[async_trait]
impl BackupProvider for DriveBackup {
    async fn verify_upload(&self, upload: &PreparedUpload) -> anyhow::Result<bool> {
        let token = self.verified_token().await?;
        self.api(&token).confirmed(upload).await
    }
    async fn list(&self) -> anyhow::Result<Vec<BackupCopy>> {
        let token = self.verified_token().await?;
        self.api(&token).list().await
    }
    async fn download(&self, id: &str) -> anyhow::Result<Vec<u8>> {
        let token = self.verified_token().await?;
        self.api(&token).download(id).await
    }
    async fn delete(&self, id: &str) -> anyhow::Result<()> {
        let token = self.verified_token().await?;
        self.api(&token).delete(id).await
    }
    async fn reserve(&self, name: &str, data: &[u8]) -> anyhow::Result<PreparedUpload> {
        let token = self.verified_token().await?;
        self.api(&token).reserve(name, data).await
    }
    async fn upload_prepared(
        &self,
        upload: &mut PreparedUpload,
        data: &[u8],
        checkpoint: &dyn UploadCheckpoint,
    ) -> anyhow::Result<()> {
        let token = self.verified_token().await?;
        self.api(&token).upload(upload, data, checkpoint).await
    }
    async fn upload(&self, name: &str, data: Vec<u8>) -> anyhow::Result<String> {
        let mut upload = self.reserve(name, &data).await?;
        self.upload_prepared(&mut upload, &data, &NoCheckpoint)
            .await?;
        Ok(upload.id)
    }
}
