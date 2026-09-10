use super::*;
use crate::{
    model::Preferences,
    providers::{
        drive_http::{response_bytes, response_json, valid_id},
        google::{Google, Service},
    },
};
use reqwest::StatusCode;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use std::collections::HashSet;

const FIELDS: &str = "id,name,trashed,spaces,mimeType,ownedByMe,appProperties,size,sha256Checksum";
const PAGE_SIZE: usize = 100;
const TOKEN_LIMIT: usize = 4096;

/// A single verified credential snapshot for a sync pass. Obtain a fresh session
/// through Google for later passes; do not retain an expired token indefinitely.
/// Endpoints can only be replaced inside this module's loopback tests.
pub struct Session {
    http: reqwest::Client,
    base: url::Url,
    token: SecretString,
    binding: Binding,
}

#[derive(Debug)]
pub struct Page {
    pub records: Vec<RemoteRecord>,
    pub next: Option<String>,
    pub(super) binding: Binding,
    pub(super) cursor: Option<String>,
    pub(super) profile: Option<(Uuid, Uuid)>,
}

impl Session {
    #[cfg(feature = "test-support")]
    pub(crate) async fn fixture(
        url: &str,
        preferences: &Preferences,
        namespace: String,
    ) -> anyhow::Result<Self> {
        let base = url::Url::parse(url)?;
        anyhow::ensure!(
            base.scheme() == "http"
                && base.host_str() == Some("127.0.0.1")
                && base.username().is_empty()
                && base.password().is_none(),
            "Profile fixtures require the owned loopback server."
        );
        let session = Self {
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            base,
            token: SecretString::from("fixture-profile-token"),
            binding: Binding::new(preferences.google_connection_id.clone(), namespace)?,
        };
        session.verify_identity().await?;
        Ok(session)
    }
    pub async fn connect(
        google: &Google,
        preferences: &Preferences,
        namespace: String,
    ) -> anyhow::Result<Self> {
        let binding = Binding::new(preferences.google_connection_id.clone(), namespace)?;
        let token = google.token_for(preferences, Service::ProfileSync).await?;
        let session = Self {
            http: google.http.clone(),
            base: google.api_base.clone(),
            token,
            binding,
        };
        session.verify_identity().await?;
        Ok(session)
    }
    pub(super) async fn catalog_drive(&self) -> anyhow::Result<shep_profile_core::drive::Drive> {
        #[cfg(any(test, feature = "test-support"))]
        if self.base.scheme() == "http" {
            return Ok(shep_profile_core::drive::Drive::connect_fixture(
                self.base.join("/")?,
                self.binding.namespace().into(),
                Some(self.binding.identity()),
            )
            .await?);
        }
        Ok(shep_profile_core::drive::Drive::connect(
            self.token.clone(),
            self.binding.namespace().into(),
            Some(self.binding.identity()),
        )
        .await?)
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub async fn page_for(&self, scan: &journal::Scan) -> anyhow::Result<Page> {
        anyhow::ensure!(
            scan.binding() == &self.binding && !scan.complete(),
            "This discovery belongs to another Google connection or has already finished."
        );
        self.list_page(scan.cursor(), scan.profile()).await
    }
    fn url(&self, path: &str) -> anyhow::Result<url::Url> {
        Ok(self.base.join(path)?)
    }
    fn file_url(&self, id: &str) -> anyhow::Result<url::Url> {
        anyhow::ensure!(
            valid_id(id) && id.len() <= 200,
            "Invalid Drive profile file ID."
        );
        self.url(&format!("/drive/v3/files/{id}"))
    }
    async fn verify_identity(&self) -> anyhow::Result<()> {
        self.binding.validate()?;
        let value = response_json(
            self.http
                .get(self.url("/drive/v3/about")?)
                .bearer_auth(self.token.expose_secret())
                .query(&[("fields", "user(permissionId)")])
                .send()
                .await?,
        )
        .await?;
        let identity = value["user"]["permissionId"]
            .as_str()
            .filter(|id| valid_id(id))
            .context("Google Drive did not identify this account. Reconnect in Preferences.")?;
        anyhow::ensure!(
            format!("drive:{identity}") == self.binding.identity,
            "The Google account changed. Reconnect before syncing profiles; pending changes were kept."
        );
        Ok(())
    }

    /// One page, never an entire history. The enrollment's durable scan tracks
    /// earlier tokens and file/operation IDs across pages before applying a scan.
    /// Empty intermediate pages with a next token are not completion.
    pub async fn list_page(
        &self,
        cursor: Option<&str>,
        profile: Option<(Uuid, Uuid)>,
    ) -> anyhow::Result<Page> {
        if let Some(cursor) = cursor {
            check_token(cursor)?;
        }
        let mut query =
            "trashed = false and appProperties has { key='shepType' and value='profile' }"
                .to_string();
        if let Some((profile, generation)) = profile {
            anyhow::ensure!(
                !profile.is_nil() && !generation.is_nil(),
                "Choose a valid profile generation."
            );
            query.push_str(&format!(" and appProperties has {{ key='shepProfile' and value='{profile}' }} and appProperties has {{ key='shepGeneration' and value='{generation}' }}"));
        }
        let value = response_json(
            self.http
                .get(self.url("/drive/v3/files")?)
                .bearer_auth(self.token.expose_secret())
                .query(&[
                    ("spaces", "appDataFolder"),
                    ("q", query.as_str()),
                    ("pageSize", "100"),
                    ("pageToken", cursor.unwrap_or("")),
                    (
                        "fields",
                        &format!("nextPageToken,incompleteSearch,files({FIELDS})"),
                    ),
                ])
                .send()
                .await?,
        )
        .await?;
        anyhow::ensure!(
            matches!(
                value.get("incompleteSearch"),
                None | Some(Value::Bool(false))
            ),
            "Google Drive returned an incomplete or invalid profile list. The local setup was kept."
        );
        let files = value.get("files").and_then(Value::as_array).context(
            "Google Drive omitted the profile list. Retry discovery; this is not an empty account.",
        )?;
        anyhow::ensure!(
            files.len() <= PAGE_SIZE,
            "Google Drive exceeded the requested profile page size."
        );
        let mut ids = HashSet::new();
        let mut operations = HashSet::new();
        let mut records = Vec::with_capacity(files.len());
        for file in files {
            let record = self.parse_file(file)?;
            anyhow::ensure!(
                ids.insert(record.id.clone())
                    && operations.insert((
                        record.key.profile,
                        record.key.generation,
                        record.key.operation
                    )),
                "Google Drive returned duplicate profile records. Keep the current setup and retry discovery."
            );
            anyhow::ensure!(
                profile.is_none_or(|p| p == (record.key.profile, record.key.generation)),
                "Google Drive returned another profile generation."
            );
            records.push(record);
        }
        let next = match value.get("nextPageToken") {
            None | Some(Value::Null) => None,
            Some(Value::String(token)) if token.is_empty() => None,
            Some(Value::String(token)) => {
                check_token(token)?;
                Some(token.clone())
            }
            _ => anyhow::bail!("Google Drive returned an invalid profile page token."),
        };
        anyhow::ensure!(
            next.is_none() || next.as_deref() != cursor,
            "Google Drive repeated the profile page token. Retry discovery."
        );
        Ok(Page {
            records,
            next,
            binding: self.binding.clone(),
            cursor: cursor.map(str::to_owned),
            profile,
        })
    }

    fn parse_file(&self, file: &Value) -> anyhow::Result<RemoteRecord> {
        let properties = &file["appProperties"];
        anyhow::ensure!(
            file["trashed"] == false
                && file["ownedByMe"] == true
                && file["mimeType"] == "application/json"
                && file["spaces"]
                    .as_array()
                    .is_some_and(|spaces| spaces.len() == 1 && spaces[0] == "appDataFolder")
                && properties["shepType"] == "profile"
                && properties["shepNamespace"] == self.binding.namespace_hash(),
            "This Drive file is not a Shep profile in the selected application namespace. It was kept unchanged."
        );
        anyhow::ensure!(
            properties["shepFormat"] == "operation-v1",
            shep_profile_core::Error::Upgrade
        );
        let key = Key {
            profile: canonical_uuid(properties["shepProfile"].as_str())?,
            generation: canonical_uuid(properties["shepGeneration"].as_str())?,
            operation: canonical_uuid(properties["shepOperation"].as_str())?,
        };
        let record = RemoteRecord {
            id: file["id"]
                .as_str()
                .context("Drive omitted the profile file ID")?
                .into(),
            key,
            size: file["size"]
                .as_str()
                .context("Drive omitted the profile record size")?
                .parse()?,
            sha256: properties["shepSha256"]
                .as_str()
                .context("Drive omitted the profile checksum")?
                .into(),
        };
        record.validate()?;
        anyhow::ensure!(
            file["name"] == key.filename(),
            "Drive returned an unexpected profile filename."
        );
        if let Some(checksum) = file.get("sha256Checksum") {
            anyhow::ensure!(
                checksum.as_str() == Some(&record.sha256),
                "Google Drive's profile checksum disagrees with the record metadata."
            );
        }
        Ok(record)
    }

    async fn metadata(&self, id: &str) -> anyhow::Result<Option<(RemoteRecord, bool)>> {
        let response = self
            .http
            .get(self.file_url(id)?)
            .bearer_auth(self.token.expose_secret())
            .query(&[("fields", FIELDS)])
            .send()
            .await?;
        if matches!(response.status(), StatusCode::NOT_FOUND | StatusCode::GONE) {
            return Ok(None);
        }
        let value = response_json(response).await?;
        let record = self.parse_file(&value)?;
        anyhow::ensure!(
            record.id == id,
            "Drive returned a different profile file identity."
        );
        Ok(Some((record, value.get("sha256Checksum").is_some())))
    }
    async fn media(&self, remote: &RemoteRecord) -> anyhow::Result<Record> {
        remote.validate()?;
        let bytes = response_bytes(
            self.http
                .get(self.file_url(&remote.id)?)
                .bearer_auth(self.token.expose_secret())
                .query(&[("alt", "media")])
                .send()
                .await?,
            remote.size,
        )
        .await?;
        let record = Record::decode(&self.binding.namespace, bytes)?;
        remote.verify(&record)?;
        Ok(record)
    }
    pub async fn download(&self, expected: &RemoteRecord) -> anyhow::Result<Record> {
        expected.validate()?;
        let (current, _) = self.metadata(&expected.id).await?.context(
            "The profile record disappeared. Retry discovery; the local setup was kept.",
        )?;
        anyhow::ensure!(
            &current == expected,
            "A previously listed profile record changed. The local setup was kept."
        );
        self.media(expected).await
    }

    /// Reservation cannot publish account data. Journal::prepare must durably
    /// store this exact identity and original bytes before upload is available.
    pub async fn reserve(&self, record: Record) -> anyhow::Result<ReservedUpload> {
        anyhow::ensure!(
            record.operation.namespace == self.binding.namespace,
            "The profile record belongs to another application namespace."
        );
        let value = response_json(
            self.http
                .get(self.url("/drive/v3/files/generateIds")?)
                .bearer_auth(self.token.expose_secret())
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
            "Google reserved the profile ID in an unexpected space."
        );
        let ids = value["ids"]
            .as_array()
            .context("Google did not reserve a profile record ID")?;
        anyhow::ensure!(
            ids.len() == 1,
            "Google did not reserve exactly one profile record ID."
        );
        let id = ids[0]
            .as_str()
            .filter(|id| valid_id(id) && id.len() <= 200)
            .context("Google reserved an invalid profile record ID")?;
        Ok(ReservedUpload {
            binding: self.binding.clone(),
            remote: RemoteRecord {
                id: id.into(),
                key: record.key(),
                size: record.bytes.len() as u64,
                sha256: record.sha256.clone(),
            },
            record,
        })
    }

    async fn confirmed(&self, upload: &ReservedUpload) -> anyhow::Result<bool> {
        let Some((remote, server_checksum)) = self.metadata(&upload.remote.id).await? else {
            return Ok(false);
        };
        anyhow::ensure!(
            remote == upload.remote,
            "The reserved Drive ID contains another profile record. It was not overwritten; the pending change was kept."
        );
        if !server_checksum {
            remote.verify(&self.media(&remote).await?)?;
        }
        Ok(true)
    }
    /// Only the durable journal can mint this input. Retries always use the same
    /// ID/bytes; no update/delete endpoint can alter another device's operation.
    pub async fn upload(&self, durable: &journal::DurableUpload) -> anyhow::Result<RemoteRecord> {
        let upload = durable.upload();
        upload.validate()?;
        anyhow::ensure!(
            upload.binding == self.binding,
            "The pending upload belongs to another Google account or application. Reconnect its original account to retry."
        );
        if self.confirmed(upload).await? {
            return Ok(upload.remote.clone());
        }
        anyhow::ensure!(
            !durable.acknowledged(),
            "A previously confirmed profile record is no longer available in this Google application. Check the account/application or restore the profile; it was not recreated."
        );
        let boundary = format!("shep-profile-{}", Uuid::new_v4());
        let key = upload.remote.key;
        let metadata = json!({"id":upload.remote.id,"name":key.filename(),"mimeType":"application/json","parents":["appDataFolder"],
            "appProperties":{"shepType":"profile","shepFormat":"operation-v1","shepNamespace":self.binding.namespace_hash(),"shepProfile":key.profile.to_string(),"shepGeneration":key.generation.to_string(),"shepOperation":key.operation.to_string(),"shepSha256":upload.remote.sha256}});
        let mut body = format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/json\r\n\r\n").into_bytes();
        body.extend_from_slice(upload.record.bytes());
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let response = self
            .http
            .post(self.url("/upload/drive/v3/files")?)
            .bearer_auth(self.token.expose_secret())
            .query(&[("uploadType", "multipart"), ("fields", FIELDS)])
            .header(
                "Content-Type",
                format!("multipart/related; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await;
        if let Ok(response) = response
            && matches!(response.status(), StatusCode::OK | StatusCode::CREATED)
            && let Ok(value) = response_json(response).await
            && self
                .parse_file(&value)
                .is_ok_and(|remote| remote == upload.remote)
            && value.get("sha256Checksum").is_some()
        {
            return Ok(upload.remote.clone());
        }
        // A lost reply, 409 or invalid success body can follow a real commit.
        // Verify that exact ID before allowing the worker to acknowledge it.
        anyhow::ensure!(
            self.confirmed(upload).await?,
            "Google Drive has not confirmed this profile change. Retry its saved pending upload; the local change was kept."
        );
        Ok(upload.remote.clone())
    }
}
pub(super) fn check_token(value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !value.is_empty() && value.len() <= TOKEN_LIMIT && !value.chars().any(char::is_control),
        "Google Drive returned an invalid profile page token."
    );
    Ok(())
}

#[cfg(test)]
pub(in crate::profile_sync) mod tests;
