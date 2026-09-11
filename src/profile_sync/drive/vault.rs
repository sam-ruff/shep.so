//! Replaceable credential key and vault files. Unlike profile records these are
//! created once and later deleted; nothing is ever updated in place.
use super::*;
use crate::profile_sync::vault::{Kind, NewFile, Remote, RemoteFile, Scope};
use shep_profile_core::vault::{MAX_FILE_BYTES, MAX_REVISION};

const TYPES: [&str; 2] = ["credential-key", "credential-vault"];
/// A healthy binding has one key and one vault file; concurrent writers and
/// interrupted passes add a few more. Anything larger is refused, not merged.
const MAX_FILES: usize = 64;

fn file_name(kind: Kind, key: Uuid, vault: Uuid) -> String {
    match kind {
        Kind::Key { .. } => format!("shep-credential-key-{key}.json"),
        Kind::Vault { .. } => format!("shep-credential-vault-{vault}.json"),
    }
}

impl Session {
    fn check_scope(&self, scope: Scope) -> anyhow::Result<()> {
        anyhow::ensure!(
            !scope.profile.is_nil() && !scope.generation.is_nil(),
            "Choose a valid shared profile before syncing passwords."
        );
        Ok(())
    }

    fn parse_credential(&self, file: &Value, scope: Scope) -> anyhow::Result<RemoteFile> {
        let properties = &file["appProperties"];
        anyhow::ensure!(
            file["trashed"] == false
                && file["ownedByMe"] == true
                && file["mimeType"] == "application/json"
                && file["spaces"]
                    .as_array()
                    .is_some_and(|spaces| spaces.len() == 1 && spaces[0] == "appDataFolder")
                && TYPES.contains(&properties["shepType"].as_str().unwrap_or_default())
                && properties["shepNamespace"] == self.binding.namespace_hash(),
            "A password vault file is not owned by this Shep application namespace. It was kept unchanged."
        );
        let key_file = properties["shepType"] == "credential-key";
        anyhow::ensure!(
            properties["shepFormat"]
                == if key_file {
                    "credential-key-v1"
                } else {
                    "credential-vault-v1"
                },
            shep_profile_core::vault::Error::Upgrade
        );
        anyhow::ensure!(
            canonical_uuid(properties["shepProfile"].as_str())? == scope.profile
                && canonical_uuid(properties["shepGeneration"].as_str())? == scope.generation,
            "Google Drive returned a password vault for another profile."
        );
        let key = canonical_uuid(properties["shepKey"].as_str())?;
        let kind = if key_file {
            Kind::Key {
                sequence: properties["shepSequence"]
                    .as_str()
                    .and_then(|s| s.parse::<u32>().ok())
                    .filter(|n| *n > 0)
                    .context("Drive omitted the password key sequence.")?,
            }
        } else {
            Kind::Vault {
                revision: properties["shepRevision"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .filter(|n| (1..=MAX_REVISION).contains(n))
                    .context("Drive omitted the password vault revision.")?,
            }
        };
        let name = file["name"].as_str().unwrap_or_default();
        let named = match kind {
            Kind::Key { .. } => name == file_name(kind, key, key),
            Kind::Vault { .. } => name
                .strip_prefix("shep-credential-vault-")
                .and_then(|rest| rest.strip_suffix(".json"))
                .is_some_and(|id| canonical_uuid(Some(id)).is_ok()),
        };
        anyhow::ensure!(
            named,
            "Drive returned an unexpected password vault filename."
        );
        let remote = RemoteFile {
            id: file["id"]
                .as_str()
                .filter(|id| valid_id(id) && id.len() <= 200)
                .context("Drive returned an invalid password vault file ID.")?
                .into(),
            key,
            kind,
            size: file["size"]
                .as_str()
                .and_then(|s| s.parse::<u64>().ok())
                .filter(|n| *n > 0 && *n <= MAX_FILE_BYTES as u64)
                .context("Drive returned an invalid password vault size.")?,
            sha256: properties["shepSha256"]
                .as_str()
                .filter(|s| valid_digest(s))
                .context("Drive omitted the password vault checksum.")?
                .into(),
        };
        if let Some(checksum) = file.get("sha256Checksum") {
            anyhow::ensure!(
                checksum.as_str() == Some(&remote.sha256),
                "Google Drive's password vault checksum disagrees with its metadata."
            );
        }
        Ok(remote)
    }

    async fn credential_metadata(
        &self,
        id: &str,
        scope: Scope,
    ) -> anyhow::Result<Option<RemoteFile>> {
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
        let file = self.parse_credential(&response_json(response).await?, scope)?;
        anyhow::ensure!(
            file.id == id,
            "Drive returned a different password vault file."
        );
        Ok(Some(file))
    }
}

#[async_trait::async_trait]
impl Remote for Session {
    async fn list(&self, scope: Scope) -> anyhow::Result<Vec<RemoteFile>> {
        self.check_scope(scope)?;
        let query = format!(
            "trashed = false and (appProperties has {{ key='shepType' and value='credential-key' }} or appProperties has {{ key='shepType' and value='credential-vault' }}) and appProperties has {{ key='shepProfile' and value='{}' }} and appProperties has {{ key='shepGeneration' and value='{}' }}",
            scope.profile, scope.generation
        );
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        let mut files = Vec::new();
        loop {
            let value = response_json(
                self.http
                    .get(self.url("/drive/v3/files")?)
                    .bearer_auth(self.token.expose_secret())
                    .query(&[
                        ("spaces", "appDataFolder"),
                        ("q", query.as_str()),
                        ("pageSize", "100"),
                        ("pageToken", cursor.as_deref().unwrap_or("")),
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
                "Google Drive returned an incomplete password vault list. Nothing was changed."
            );
            let page = value
                .get("files")
                .and_then(Value::as_array)
                .context("Google Drive omitted the password vault list. Nothing was changed.")?;
            for file in page {
                let file = self.parse_credential(file, scope)?;
                anyhow::ensure!(
                    seen.insert(file.id.clone()) && files.len() < MAX_FILES,
                    "Google Drive returned duplicate or too many password vault files. Nothing was changed."
                );
                files.push(file);
            }
            match value.get("nextPageToken") {
                None | Some(Value::Null) => break,
                Some(Value::String(token)) if token.is_empty() => break,
                Some(Value::String(token)) => {
                    check_token(token)?;
                    anyhow::ensure!(
                        cursor.as_deref() != Some(token.as_str()),
                        "Google Drive repeated the password vault page token."
                    );
                    cursor = Some(token.clone());
                }
                _ => anyhow::bail!("Google Drive returned an invalid password vault page token."),
            }
        }
        Ok(files)
    }

    async fn download(&self, scope: Scope, file: &RemoteFile) -> anyhow::Result<Vec<u8>> {
        self.check_scope(scope)?;
        let bytes = response_bytes(
            self.http
                .get(self.file_url(&file.id)?)
                .bearer_auth(self.token.expose_secret())
                .query(&[("alt", "media")])
                .send()
                .await?,
            file.size,
        )
        .await?;
        anyhow::ensure!(
            bytes.len() as u64 == file.size && digest(&bytes) == file.sha256,
            "A password vault file changed while it was read. Nothing was used from it."
        );
        Ok(bytes)
    }

    async fn create(&self, scope: Scope, file: NewFile) -> anyhow::Result<RemoteFile> {
        self.check_scope(scope)?;
        anyhow::ensure!(
            !file.bytes.is_empty() && file.bytes.len() <= MAX_FILE_BYTES,
            "The password vault is too large to upload."
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
        let ids = value["ids"]
            .as_array()
            .filter(|ids| ids.len() == 1 && value["space"] == "appDataFolder")
            .context("Google did not reserve exactly one password vault file ID.")?;
        let id = ids[0]
            .as_str()
            .filter(|id| valid_id(id) && id.len() <= 200)
            .context("Google reserved an invalid password vault file ID.")?;
        let expected = RemoteFile {
            id: id.into(),
            key: file.key,
            kind: file.kind,
            size: file.bytes.len() as u64,
            sha256: digest(&file.bytes),
        };
        let (kind, format, counter) = match file.kind {
            Kind::Key { sequence } => (
                "credential-key",
                "credential-key-v1",
                ("shepSequence", sequence.to_string()),
            ),
            Kind::Vault { revision } => (
                "credential-vault",
                "credential-vault-v1",
                ("shepRevision", revision.to_string()),
            ),
        };
        let mut properties = json!({"shepType":kind,"shepFormat":format,"shepNamespace":self.binding.namespace_hash(),
            "shepProfile":scope.profile.to_string(),"shepGeneration":scope.generation.to_string(),
            "shepKey":file.key.to_string(),"shepSha256":expected.sha256});
        properties[counter.0] = json!(counter.1);
        let metadata = json!({"id":id,"name":file_name(file.kind, file.key, Uuid::new_v4()),
            "mimeType":"application/json","parents":["appDataFolder"],"appProperties":properties});
        let boundary = format!("shep-credential-{}", Uuid::new_v4().simple());
        let mut body = format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: application/json\r\n\r\n").into_bytes();
        body.extend_from_slice(&file.bytes);
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
                .parse_credential(&value, scope)
                .is_ok_and(|created| created == expected)
        {
            return Ok(expected);
        }
        // A lost reply or conflict can follow a real commit; read the reserved ID.
        anyhow::ensure!(
            self.credential_metadata(id, scope).await?.as_ref() == Some(&expected),
            "Google Drive did not confirm the password vault upload. It will be retried."
        );
        Ok(expected)
    }

    async fn delete(&self, file: &RemoteFile) -> anyhow::Result<()> {
        let response = self
            .http
            .delete(self.file_url(&file.id)?)
            .bearer_auth(self.token.expose_secret())
            .send()
            .await?;
        anyhow::ensure!(
            response.status().is_success()
                || matches!(response.status(), StatusCode::NOT_FOUND | StatusCode::GONE),
            "Google Drive could not remove an old password vault file (HTTP {}). It will be retried.",
            response.status()
        );
        Ok(())
    }
}

#[cfg(test)]
pub(in crate::profile_sync) mod tests;
