//! Durable remote discovery, separate from an enrolled device's local history.
mod progress;
mod storage;
mod worker;
pub use worker::Discovery;

use super::{ChangeCursor, ChangePage, Drive, File, FileChange, Page, wire};
use crate::history::{self, Binding, Journal, Overview};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "This discovery catalog belongs to another Google account or application. Reopen the intended setup."
    )]
    Binding,
    #[error("Profile discovery could not save its progress. Check device storage and retry.")]
    Storage,
    #[error("Profile discovery is already open elsewhere. Close the other owner before retrying.")]
    Owned,
    #[error(
        "Profile discovery changed during this request. Use its current state before retrying."
    )]
    Changed,
    #[error("Profile discovery is busy. Retry this request shortly.")]
    Busy,
    #[error("Profile discovery stopped. Reopen it and retry.")]
    Stopped,
    #[error(
        "Profile files or pagination changed unexpectedly. Keep the cached setup and restart discovery; do not publish new changes yet."
    )]
    Integrity,
    #[error(
        "Previously discovered profile files are missing. Keep the cached setup and review the source before publishing changes."
    )]
    Missing,
    #[error("Profile discovery has a saved error. Retry or restart the scan before continuing.")]
    Failed,
    #[error(transparent)]
    Provider(#[from] super::Error),
    #[error(transparent)]
    History(#[from] history::Error),
}
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}
type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub namespace: String,
    pub principal: String,
}
impl Scope {
    fn validate(&self) -> Result<()> {
        if !crate::namespace(&self.namespace)
            || !self.principal.strip_prefix("drive:").is_some_and(wire::id)
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
    pub fn storage_key(&self) -> Result<String> {
        self.validate()?;
        Ok(wire::sha256(
            &serde_json::to_vec(self).map_err(|_| Error::Binding)?,
        ))
    }
    fn check(&self, drive: &Drive) -> Result<()> {
        if self.namespace != drive.namespace() || self.principal != drive.principal() {
            return Err(Error::Binding);
        }
        Ok(())
    }
    fn binding(&self, profile: Uuid, generation: Uuid) -> Binding {
        Binding {
            namespace: self.namespace.clone(),
            principal: self.principal.clone(),
            profile,
            generation,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Initial,
    Files,
    Changes,
    Complete,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub revision: u64,
    pub scan: u64,
    pub phase: Phase,
    pub files: u64,
    pub profiles: u64,
    pub pending: u64,
    pub incomplete_profiles: u64,
    pub completed_revision: Option<u64>,
    pub error: Option<String>,
}

/// Counts cover remote definitions and setting intents, including resets. This does not establish local account
/// activation, credential availability, enrollment or provider login success.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub profile: Uuid,
    pub generation: Uuid,
    pub name: Option<String>,
    pub name_conflict: bool,
    pub accounts: u64,
    pub settings: u64,
    pub operations: u64,
    pub waiting: u64,
    pub ready: u64,
    pub conflicts: u64,
    pub removed: bool,
    pub revision: u64,
}
impl Profile {
    fn from_overview(binding: &Binding, overview: Overview) -> Self {
        Self {
            profile: binding.profile,
            generation: binding.generation,
            name: overview.name,
            name_conflict: overview.name_conflict,
            accounts: overview.accounts,
            settings: overview.settings,
            operations: overview.state.operations,
            waiting: overview.state.waiting,
            ready: overview.state.ready,
            conflicts: overview.state.conflicts,
            removed: overview.state.removed,
            revision: overview.state.revision,
        }
    }
    pub fn cursor(&self) -> String {
        format!("{}:{}", self.profile, self.generation)
    }
}

enum Work {
    Start,
    List(Option<String>),
    Changes(String),
    Download { position: u64, file: File },
    Drain { profile: Uuid, generation: Uuid },
    Advance,
    Done,
}
struct Catalog {
    db: Connection,
    scope: Scope,
    remote_root: PathBuf,
    projection: Option<(Binding, Journal)>,
    _lock: std::fs::File,
}
impl Catalog {
    fn open(path: &Path, scope: Scope) -> Result<Self> {
        scope.validate()?;
        storage::directory(path.parent().ok_or(Error::Storage)?)?;
        storage::private_file(path)?;
        let path = path.canonicalize().map_err(|_| Error::Storage)?;
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".discovery-lock");
        let lock = storage::private_file(Path::new(&lock_path))?;
        fs2::FileExt::try_lock_exclusive(&lock).map_err(|error| {
            if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
                Error::Owned
            } else {
                Error::Storage
            }
        })?;
        let mut db = Connection::open(&path)?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA temp_store=FILE;")?;
        storage::initialize(&mut db, &scope)?;
        let mut root = path.as_os_str().to_owned();
        root.push(".observations");
        let remote_root = PathBuf::from(root);
        storage::directory(&remote_root)?;
        Ok(Self {
            db,
            scope,
            remote_root,
            projection: None,
            _lock: lock,
        })
    }
    fn check_revision(&self, expected: u64) -> Result<()> {
        let current: u64 = self
            .db
            .query_row("SELECT revision FROM state", [], |r| history::count(r, 0))?;
        if expected != current {
            return Err(Error::Changed);
        }
        Ok(())
    }
    fn state(&self) -> Result<State> {
        Ok(self.db.query_row(
            "SELECT revision,scan,phase,files,profiles,completed_revision,fault,
            (SELECT count(*) FROM pending),
            (SELECT count(*) FROM profiles WHERE waiting>0) FROM state",
            [],
            |row| {
                let phase: String = row.get(2)?;
                let phase = match phase.as_str() {
                    "initial" => Phase::Initial,
                    "files" => Phase::Files,
                    "changes" => Phase::Changes,
                    "complete" => Phase::Complete,
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                Ok(State {
                    revision: history::count(row, 0)?,
                    scan: history::count(row, 1)?,
                    phase,
                    files: history::count(row, 3)?,
                    profiles: history::count(row, 4)?,
                    completed_revision: storage::optional_count(row, 5)?,
                    error: row.get(6)?,
                    pending: history::count(row, 7)?,
                    incomplete_profiles: history::count(row, 8)?,
                })
            },
        )?)
    }
    fn profiles(&self, after: Option<&str>) -> Result<Vec<Profile>> {
        if after.is_some_and(|key| key.len() > 73) {
            return Err(Error::Changed);
        }
        self.db
            .prepare("SELECT summary FROM profiles WHERE key>? ORDER BY key LIMIT 50")?
            .query_map([after.unwrap_or("")], |r| r.get::<_, String>(0))?
            .map(|row| serde_json::from_str(&row?).map_err(|_| Error::Storage))
            .collect()
    }
    fn journal(&mut self, binding: Binding) -> Result<&mut Journal> {
        if !self
            .projection
            .as_ref()
            .is_some_and(|(old, _)| old == &binding)
        {
            self.projection = None;
            let path = self
                .remote_root
                .join(format!("{}.sqlite", binding.storage_key()?));
            let journal = Journal::open(&path, binding.clone())?;
            self.projection = Some((binding, journal));
        }
        Ok(&mut self.projection.as_mut().unwrap().1)
    }
    fn load_file(&self, text: &str) -> Result<File> {
        if text.len() > 16384 {
            return Err(Error::Storage);
        }
        let value = crate::json::decode(text.as_bytes()).map_err(|_| Error::Storage)?;
        Ok(wire::file(
            &value,
            &self.scope.principal,
            &self.scope.namespace,
        )?)
    }
    fn work(&self) -> Result<(u64, Work)> {
        let state = self.state()?;
        if state.error.is_some() {
            return Err(Error::Failed);
        }
        let pending = self
            .db
            .query_row(
                "SELECT position,data FROM pending ORDER BY position LIMIT 1",
                [],
                |r| Ok((history::count(r, 0)?, r.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((position, data)) = pending {
            return Ok((
                state.revision,
                Work::Download {
                    position,
                    file: self.load_file(&data)?,
                },
            ));
        }
        let ready = self
            .db
            .query_row(
                "SELECT summary FROM profiles WHERE ready>0 ORDER BY key LIMIT 1",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?;
        if let Some(ready) = ready {
            let ready: Profile = serde_json::from_str(&ready).map_err(|_| Error::Storage)?;
            return Ok((
                state.revision,
                Work::Drain {
                    profile: ready.profile,
                    generation: ready.generation,
                },
            ));
        }
        let (has_page, token): (bool, Option<String>) =
            self.db
                .query_row("SELECT has_page,current_token FROM state", [], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })?;
        let work = if has_page {
            Work::Advance
        } else {
            match state.phase {
                Phase::Initial => Work::Start,
                Phase::Files => Work::List(token),
                Phase::Changes => Work::Changes(token.ok_or(Error::Storage)?),
                Phase::Complete => Work::Done,
            }
        };
        Ok((state.revision, work))
    }
}
