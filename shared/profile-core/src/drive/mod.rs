//! Native Google Drive profile transport. Call from an owned background task.
//! A verified session retains one short-lived access token, never refresh tokens.
//! Listing a page is not enrollment or proof of a complete cloud snapshot.
pub mod catalog;
mod changes;
#[cfg(test)]
mod tests;
mod wire;
pub use changes::{ChangeCursor, ChangePage, FileChange};

use crate::{Operation, history};
use history::{Command, Reply, Worker};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode, Url};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, time::Duration};
use uuid::Uuid;

pub const PAGE_SIZE: usize = 50;
const JSON_LIMIT: usize = 256 * 1024;
const FILE_FIELDS: &str =
    "id,name,trashed,ownedByMe,spaces,mimeType,appProperties,size,sha256Checksum";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "Google access has expired or lacks Drive permission. Reconnect the same Google account and retry."
    )]
    Authorization,
    #[error(
        "Google Drive denied this request. Check the account's Drive permission and service limits before retrying."
    )]
    Denied,
    #[error(
        "Google returned another account. Keep the current profile and reconnect its Google account."
    )]
    Identity,
    #[error(
        "These profiles use another application namespace. Review the Google application configuration before continuing."
    )]
    Namespace,
    #[error(
        "Google Drive could not be reached. Keep the local setup and retry the same operation."
    )]
    Network,
    #[error("Google Drive rejected the request (HTTP {0}). Keep the local setup and retry.")]
    Http(u16),
    #[error(
        "Google Drive returned invalid or unowned profile data. Keep the local setup and retry discovery."
    )]
    Invalid,
    #[error(
        "Google Drive returned an incomplete profile page. Restart discovery before creating or enrolling a profile."
    )]
    Incomplete,
    #[error(
        "Google Drive returned too much profile data. Keep the local setup and review the source."
    )]
    TooLarge,
    #[error(
        "An immutable profile file has changed. Keep the local setup and review the conflicting source."
    )]
    Changed,
    #[error(
        "The profile upload could not be confirmed. Retry the same saved upload; do not create another operation."
    )]
    Unconfirmed,
    #[error(
        "The profile file was saved to Google, but its discovery receipt could not be saved. Check device storage and retry this same publication."
    )]
    DiscoveryReceipt,
    #[error(transparent)]
    History(#[from] history::Error),
    #[error(transparent)]
    Record(#[from] crate::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

/// Constructed only after checking the private file's ownership and metadata.
/// This is metadata evidence; download checks the actual operation bytes too.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct File {
    principal: String,
    namespace: String,
    id: String,
    profile: Uuid,
    generation: Uuid,
    operation: Uuid,
    size: usize,
    sha256: String,
}
impl File {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn profile(&self) -> Uuid {
        self.profile
    }
    pub fn generation(&self) -> Uuid {
        self.generation
    }
    pub fn operation(&self) -> Uuid {
        self.operation
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// At most 50 metadata records. A durable discovery owner must retain visited
/// tokens/file IDs across pages, reject cycles/duplicates and reconcile arrivals.
/// Empty files with a next token is a valid *partial* page, never an empty setup.
#[derive(Debug)]
pub struct Page {
    pub files: Vec<File>,
    pub next: Option<String>,
}

/// No Debug/Serialize implementation: the access token must not enter logs or
/// portable metadata. Renew access by constructing a new session with the saved
/// principal. Namespace is configured per application project, not OAuth client.
pub struct Drive {
    http: Client,
    base: Url,
    token: SecretString,
    principal: String,
    namespace: String,
}
impl Drive {
    pub async fn connect(
        token: SecretString,
        namespace: String,
        expected_principal: Option<&str>,
    ) -> Result<Self> {
        let http = Self::client_builder()
            .https_only(true)
            .build()
            .map_err(|_| Error::Network)?;
        Self::verify(
            http,
            Url::parse("https://www.googleapis.com/").expect("fixed Google endpoint"),
            token,
            namespace,
            expected_principal,
        )
        .await
    }
    // The loopback fixture uses this same redirect/timeout policy. Keep the
    // HTTPS-only production endpoint separate from test-only HTTP connections.
    fn client_builder() -> reqwest::ClientBuilder {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
    }
    async fn verify(
        http: Client,
        base: Url,
        token: SecretString,
        namespace: String,
        expected_principal: Option<&str>,
    ) -> Result<Self> {
        if !crate::namespace(&namespace) {
            return Err(Error::Namespace);
        }
        if token.expose_secret().is_empty() || token.expose_secret().len() > 16384 {
            return Err(Error::Authorization);
        }
        let mut session = Self {
            http,
            base,
            token,
            principal: String::new(),
            namespace,
        };
        let user = wire::json(
            session
                .request(Method::GET, "drive/v3/about")
                .query(&[("fields", "user(permissionId)")])
                .send()
                .await
                .map_err(|_| Error::Network)?,
        )
        .await?;
        let id = user["user"]["permissionId"]
            .as_str()
            .filter(|id| wire::id(id))
            .ok_or(Error::Invalid)?;
        session.principal = format!("drive:{id}");
        if expected_principal.is_some_and(|expected| session.principal != expected) {
            return Err(Error::Identity);
        }
        Ok(session)
    }
    pub fn principal(&self) -> &str {
        &self.principal
    }
    pub fn namespace(&self) -> &str {
        &self.namespace
    }
    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, self.base.join(path).expect("validated API path"))
            .bearer_auth(self.token.expose_secret())
    }
    fn bind(&self, worker: &Worker) -> Result<()> {
        if worker.binding().principal != self.principal {
            return Err(Error::Identity);
        }
        if worker.binding().namespace != self.namespace {
            return Err(Error::Namespace);
        }
        Ok(())
    }

    pub async fn list_page(&self, token: Option<&str>) -> Result<Page> {
        if token.is_some_and(|token| !wire::page_token(token)) {
            return Err(Error::Invalid);
        }
        let fields = format!("nextPageToken,incompleteSearch,files({FILE_FIELDS})");
        let mut request = self.request(Method::GET, "drive/v3/files").query(&[
            ("spaces", "appDataFolder"),
            ("corpora", "user"),
            ("pageSize", "50"),
            ("fields", fields.as_str()),
            // Include all Shep profile versions/namespaces. An unsupported or
            // mismatched profile must not masquerade as a first-use empty setup.
            (
                "q",
                "trashed = false and appProperties has { key='shepType' and value='profile' }",
            ),
        ]);
        if let Some(token) = token {
            request = request.query(&[("pageToken", token)]);
        }
        let value = wire::json(request.send().await.map_err(|_| Error::Network)?).await?;
        if value["incompleteSearch"] != false {
            return Err(Error::Incomplete);
        }
        let files = value["files"].as_array().ok_or(Error::Invalid)?;
        if files.len() > PAGE_SIZE {
            return Err(Error::TooLarge);
        }
        let next = match value.get("nextPageToken") {
            None => None,
            Some(Value::String(next)) if wire::page_token(next) && Some(next.as_str()) != token => {
                Some(next.clone())
            }
            _ => return Err(Error::Incomplete),
        };
        let mut seen = HashSet::new();
        let files = files
            .iter()
            .map(|value| {
                let file = wire::file(value, &self.principal, &self.namespace)?;
                if !seen.insert(file.id.clone()) {
                    return Err(Error::Incomplete);
                }
                Ok(file)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Page { files, next })
    }

    async fn metadata(&self, id: &str) -> Result<Option<File>> {
        if !wire::id(id) {
            return Err(Error::Invalid);
        }
        let response = self
            .request(Method::GET, &format!("drive/v3/files/{id}"))
            .query(&[("fields", FILE_FIELDS)])
            .send()
            .await
            .map_err(|_| Error::Network)?;
        // Only an explicit 404 establishes absence. Permissions, redirects and
        // failed requests must never trigger a replacement upload.
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let file = wire::file(
            &wire::json(response).await?,
            &self.principal,
            &self.namespace,
        )?;
        if file.id != id {
            return Err(Error::Changed);
        }
        Ok(Some(file))
    }

    /// Recheck metadata since the list, then hash and validate the exact body.
    /// A changed/deleted file never imports a replacement or an empty operation.
    pub async fn download(&self, file: &File) -> Result<String> {
        if file.principal != self.principal {
            return Err(Error::Identity);
        }
        if file.namespace != self.namespace {
            return Err(Error::Namespace);
        }
        if self.metadata(&file.id).await?.as_ref() != Some(file) {
            return Err(Error::Changed);
        }
        self.media(file).await
    }
    async fn media(&self, file: &File) -> Result<String> {
        let response = self
            .request(Method::GET, &format!("drive/v3/files/{}", file.id))
            .query(&[("alt", "media")])
            .send()
            .await
            .map_err(|_| Error::Network)?;
        let bytes = wire::bytes(response, file.size).await?;
        if bytes.len() != file.size || wire::sha256(&bytes) != file.sha256 {
            return Err(Error::Changed);
        }
        let operation = Operation::decode(&bytes)?;
        if operation.namespace != file.namespace
            || operation.profile != file.profile
            || operation.generation != file.generation
            || operation.operation != file.operation
        {
            return Err(Error::Changed);
        }
        String::from_utf8(bytes).map_err(|_| Error::Invalid)
    }
    pub async fn import(&self, worker: &Worker, file: &File) -> Result<history::State> {
        self.bind(worker)?;
        if worker.binding().profile != file.profile
            || worker.binding().generation != file.generation
        {
            return Err(history::Error::Binding.into());
        }
        let record = self.download(file).await?;
        let Reply::State(state) = worker.request(Command::Import { record }).await? else {
            return Err(Error::Invalid);
        };
        Ok(state)
    }

    /// Upload at most one queued immutable operation. The journal owns its exact
    /// bytes and reserved file ID before POST. Dropping this future can lose the
    /// reply, not the reservation; another call reconciles that same remote ID.
    /// The caller must finish discovery/reconciliation before publishing edits.
    pub async fn upload_next(&self, worker: &Worker) -> Result<Option<Uuid>> {
        self.upload_next_inner(worker, None).await
    }
    /// Production publication must retain its own verified file identities before
    /// the local history can mark them uploaded, including after a lost response.
    pub async fn upload_next_tracked(
        &self,
        worker: &Worker,
        catalog: &catalog::Discovery,
    ) -> Result<Option<Uuid>> {
        if catalog.scope().namespace != self.namespace
            || catalog.scope().principal != self.principal
        {
            return Err(Error::Identity);
        }
        self.upload_next_inner(worker, Some(catalog)).await
    }
    async fn upload_next_inner(
        &self,
        worker: &Worker,
        catalog: Option<&catalog::Discovery>,
    ) -> Result<Option<Uuid>> {
        self.bind(worker)?;
        let mut upload = match worker.request(Command::NextUpload).await? {
            Reply::Upload(Some(upload)) => upload,
            Reply::Upload(None) => return Ok(None),
            _ => return Err(Error::Invalid),
        };
        let operation = Operation::decode(upload.record.as_bytes())?;
        if operation.namespace != self.namespace
            || operation.profile != worker.binding().profile
            || operation.generation != worker.binding().generation
            || operation.operation != upload.operation
            || wire::sha256(upload.record.as_bytes()) != upload.sha256
        {
            return Err(Error::Changed);
        }
        let id = match upload.file_id.take() {
            Some(id) => id,
            None => {
                let reservation = wire::json(
                    self.request(Method::GET, "drive/v3/files/generateIds")
                        .query(&[
                            ("count", "1"),
                            ("space", "appDataFolder"),
                            ("type", "files"),
                        ])
                        .send()
                        .await
                        .map_err(|_| Error::Network)?,
                )
                .await?;
                let ids = reservation["ids"].as_array().ok_or(Error::Invalid)?;
                if reservation["space"] != "appDataFolder" || ids.len() != 1 {
                    return Err(Error::Invalid);
                }
                let id = ids[0]
                    .as_str()
                    .filter(|s| wire::id(s))
                    .ok_or(Error::Invalid)?;
                worker
                    .request(Command::Reserve {
                        operation: upload.operation,
                        file_id: id.into(),
                    })
                    .await?;
                id.into()
            }
        };
        let file = File {
            principal: self.principal.clone(),
            namespace: self.namespace.clone(),
            id,
            profile: operation.profile,
            generation: operation.generation,
            operation: operation.operation,
            size: upload.record.len(),
            sha256: upload.sha256,
        };
        if !self.confirmed(&file).await? {
            let (content_type, body) = wire::multipart(&file, upload.record.as_bytes())?;
            let response = self
                .request(Method::POST, "upload/drive/v3/files")
                .query(&[("uploadType", "multipart"), ("fields", "id")])
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .body(body)
                .send()
                .await;
            // No automatic second POST. Resolve successful or uncertain writes
            // by reading the reserved file. A 409 alone is not an acknowledgment.
            if let Ok(response) = &response {
                let status = response.status();
                if !(matches!(status.as_u16(), 200 | 201 | 408 | 409 | 429)
                    || status.is_server_error())
                {
                    return Err(wire::status_error(status));
                }
            }
            drop(response); // Never parse/echo an untrusted upload response body.
            if !self.confirmed(&file).await? {
                return Err(Error::Unconfirmed);
            }
        }
        if let Some(catalog) = catalog {
            catalog
                .accept_upload(file.clone(), upload.record)
                .await
                .map_err(|_| Error::DiscoveryReceipt)?;
        }
        worker
            .request(Command::Confirm {
                operation: file.operation,
                file_id: file.id,
                sha256: file.sha256,
            })
            .await?;
        Ok(Some(file.operation))
    }
    async fn confirmed(&self, expected: &File) -> Result<bool> {
        let Some(actual) = self.metadata(&expected.id).await? else {
            return Ok(false);
        };
        if actual != *expected {
            return Err(Error::Changed);
        }
        // Provider checksum availability varies. Always verify actual bytes;
        // appProperties or an upload response alone cannot confirm the content.
        self.media(&actual).await?;
        Ok(true)
    }
}
