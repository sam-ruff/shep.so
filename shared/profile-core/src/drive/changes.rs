use super::*;

/// Changes to unrelated app data must reach the catalog too: a previously known
/// profile file losing its marker is an integrity failure, not a hidden removal.
#[derive(Clone, Debug)]
pub enum FileChange {
    Profile(File),
    Other(String),
    Removed(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeCursor {
    More(String),
    CaughtUp(String),
}
#[derive(Debug)]
pub struct ChangePage {
    pub changes: Vec<FileChange>,
    pub cursor: ChangeCursor,
}
impl Drive {
    /// Save this *before* starting a full file listing, then replay changes from
    /// it after that listing. It identifies this account's change stream, not an
    /// atomic snapshot of profile bodies or proof that an enrollment is ready.
    pub async fn start_page_token(&self) -> Result<String> {
        let body = wire::json(
            self.request(Method::GET, "drive/v3/changes/startPageToken")
                .query(&[("fields", "startPageToken")])
                .send()
                .await
                .map_err(|_| Error::Network)?,
        )
        .await?;
        body["startPageToken"]
            .as_str()
            .filter(|s| wire::page_token(s))
            .map(String::from)
            .ok_or(Error::Incomplete)
    }
    /// One bounded page. Persist the continuation only after every received
    /// change has been validated/applied; the catalog owns cross-page recovery.
    pub async fn changes_page(&self, token: &str) -> Result<ChangePage> {
        if !wire::page_token(token) {
            return Err(Error::Invalid);
        }
        let fields = format!(
            "nextPageToken,newStartPageToken,changes(fileId,removed,changeType,file({FILE_FIELDS}))"
        );
        let body = wire::json(
            self.request(Method::GET, "drive/v3/changes")
                .query(&[
                    ("spaces", "appDataFolder"),
                    ("pageSize", "50"),
                    ("includeRemoved", "true"),
                    ("restrictToMyDrive", "false"),
                    ("pageToken", token),
                    ("fields", fields.as_str()),
                ])
                .send()
                .await
                .map_err(|_| Error::Network)?,
        )
        .await?;
        let entries = body["changes"].as_array().ok_or(Error::Incomplete)?;
        if entries.len() > PAGE_SIZE {
            return Err(Error::TooLarge);
        }
        let cursor = match (body.get("nextPageToken"), body.get("newStartPageToken")) {
            (Some(Value::String(next)), None) if wire::page_token(next) && next != token => {
                ChangeCursor::More(next.clone())
            }
            (None, Some(Value::String(next))) if wire::page_token(next) => {
                ChangeCursor::CaughtUp(next.clone())
            }
            _ => return Err(Error::Incomplete),
        };
        let changes = entries
            .iter()
            .map(|entry| {
                let id = entry["fileId"]
                    .as_str()
                    .filter(|id| wire::id(id))
                    .ok_or(Error::Invalid)?;
                if entry["changeType"] != "file" {
                    return Err(Error::Invalid);
                }
                match entry["removed"].as_bool() {
                    Some(true) => Ok(FileChange::Removed(id.into())),
                    Some(false) => {
                        let file = &entry["file"];
                        if !file.is_object() || file["id"] != id {
                            return Err(Error::Invalid);
                        }
                        if file["appProperties"]["shepType"] == "profile" {
                            Ok(FileChange::Profile(wire::file(
                                file,
                                &self.principal,
                                &self.namespace,
                            )?))
                        } else {
                            Ok(FileChange::Other(id.into()))
                        }
                    }
                    None => Err(Error::Invalid),
                }
            })
            .collect::<Result<Vec<_>>>()?;
        // The stream can contain the same file more than once. Its immutable
        // identity/content is reconciled in the catalog, not discarded here.
        Ok(ChangePage { changes, cursor })
    }
}
