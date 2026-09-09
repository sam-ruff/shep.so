//! Device-local causal history. The caller must authenticate the provider and
//! verify cloud file ownership before importing records. This journal does not
//! authenticate Google, apply accounts, or acknowledge a network upload itself.
mod merge;
mod schema;
mod worker;
pub use worker::Worker;

use crate::{Action, Change, Operation};
use fs2::FileExt;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, path::Path};
use uuid::Uuid;

pub const PAGE_SIZE: usize = 50;
pub const APPLY_BATCH: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "This profile belongs to another account, application or generation. Reopen the intended profile."
    )]
    Binding,
    #[error(
        "The profile history could not be saved or read. Check device storage and retry the same operation."
    )]
    Storage,
    #[error("This profile history is already open. Close the other owner before retrying.")]
    Owned,
    #[error("The profile changed while it was being reviewed. Refresh the review before saving.")]
    Changed,
    #[error("This field has concurrent changes. Review every version before resolving it.")]
    Conflict,
    #[error("This profile or account was removed. Create a new identity before adding it again.")]
    Removed,
    #[error(
        "Some profile history is missing or waiting to be applied. Finish discovery before publishing local changes."
    )]
    Incomplete,
    #[error(
        "An immutable profile operation has different bytes or request data. Keep both records for review."
    )]
    Identity,
    #[error(
        "The profile history contains a causal cycle. Keep the local setup and review the source."
    )]
    Cycle,
    #[error(
        "Too many independent profile versions need reconciliation. Update Shep before publishing more changes."
    )]
    Heads,
    #[error("The profile worker is busy. Retry this same request shortly.")]
    Busy,
    #[error("The profile worker stopped. Reopen it and retry this same request.")]
    Stopped,
    #[error(transparent)]
    Record(#[from] crate::Error),
}
impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        Self::Storage
    }
}
pub type Result<T> = std::result::Result<T, Error>;

/// Provider identity/namespace are trusted inputs from the future authenticated
/// transport, never a user-entered email or portable OAuth client/grant ID.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub namespace: String,
    pub principal: String,
    pub profile: Uuid,
    pub generation: Uuid,
}
impl Binding {
    fn validate(&self) -> Result<()> {
        if !crate::namespace(&self.namespace)
            || self.profile.is_nil()
            || self.generation.is_nil()
            || !crate::text(&self.principal, 320, false)
        {
            return Err(Error::Binding);
        }
        Ok(())
    }
    pub fn storage_key(&self) -> Result<String> {
        self.validate()?;
        Ok(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(self).map_err(|_| Error::Binding)?)
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resolution {
    pub target: String,
    pub versions: Vec<Uuid>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalEdit {
    /// Generate once before enqueueing; reuse the whole request after a lost reply.
    pub operation: Uuid,
    pub expected_revision: u64,
    pub changes: Vec<Change>,
    #[serde(default)]
    pub resolutions: Vec<Resolution>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    State,
    Import {
        record: String,
    },
    Edit {
        edit: LocalEdit,
    },
    Drain,
    Fields {
        after: Option<String>,
    },
    Versions {
        target: String,
        after: Option<Uuid>,
    },
    Value {
        target: String,
        operation: Uuid,
    },
    NextUpload,
    Reserve {
        operation: Uuid,
        file_id: String,
    },
    Confirm {
        operation: Uuid,
        file_id: String,
        sha256: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    pub device: Uuid,
    pub revision: u64,
    pub operations: u64,
    pub waiting: u64,
    pub ready: u64,
    pub queued: u64,
    pub fields: u64,
    pub conflicts: u64,
    pub removed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Field {
    pub target: String,
    pub versions: u64,
    pub conflict: bool,
    pub revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub operation: Uuid,
    pub device: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Upload {
    pub operation: Uuid,
    pub record: String,
    pub sha256: String,
    pub file_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Reply {
    State(State),
    Fields(Vec<Field>),
    Versions(Vec<Version>),
    Value(Change),
    Upload(Option<Upload>),
}

/// Synchronous implementation for an owning background thread, never a UI
/// handler. Worker supplies the bounded async facade used by native callers.
pub struct Journal {
    db: Connection,
    binding: Binding,
    device: Uuid,
    _lock: Option<File>,
}
impl Journal {
    pub fn open(path: &Path, binding: Binding) -> Result<Self> {
        binding.validate()?;
        // Choose the companion lock from the actual file, including symlinks.
        private_file(path)?;
        let path = path.canonicalize().map_err(|_| Error::Storage)?;
        // The lock is independent of SQLite, retained until the connection drops.
        let mut lock_path = path.as_os_str().to_owned();
        lock_path.push(".history-lock");
        let lock = private_file(Path::new(&lock_path))?;
        lock.try_lock_exclusive().map_err(|error| {
            if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
                Error::Owned
            } else {
                Error::Storage
            }
        })?;
        let db = Connection::open(&path)?;
        Self::initialize(db, binding, Some(lock))
    }
    pub fn memory(binding: Binding) -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?, binding, None)
    }
    fn initialize(mut db: Connection, binding: Binding, lock: Option<File>) -> Result<Self> {
        binding.validate()?;
        db.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
        )?;
        let device = schema::initialize(&mut db, &binding)?;
        db.execute_batch(
            "PRAGMA temp_store=FILE; CREATE TEMP TABLE history_ancestors(id TEXT PRIMARY KEY);",
        )?;
        Ok(Self {
            db,
            binding,
            device,
            _lock: lock,
        })
    }
    pub fn execute(&mut self, command: Command) -> Result<Reply> {
        match command {
            Command::State => Ok(Reply::State(self.state()?)),
            Command::Import { record } => self.import(record.as_bytes()).map(Reply::State),
            Command::Edit { edit } => self.edit(edit).map(Reply::State),
            Command::Drain => self.drain().map(Reply::State),
            Command::Fields { after } => self.fields(after.as_deref()).map(Reply::Fields),
            Command::Versions { target, after } => {
                self.versions(&target, after).map(Reply::Versions)
            }
            Command::Value { target, operation } => {
                self.value(&target, operation).map(Reply::Value)
            }
            Command::NextUpload => self.next_upload().map(Reply::Upload),
            Command::Reserve { operation, file_id } => {
                self.reserve(operation, &file_id).map(Reply::State)
            }
            Command::Confirm {
                operation,
                file_id,
                sha256,
            } => self.confirm(operation, &file_id, &sha256).map(Reply::State),
        }
    }
    pub fn state(&self) -> Result<State> {
        Ok(self.db.query_row(
            "SELECT revision,operations,waiting,queued,fields,conflicts,removed,ready FROM state",
            [],
            |r| {
                Ok(State {
                    device: self.device,
                    revision: count(r, 0)?,
                    operations: count(r, 1)?,
                    waiting: count(r, 2)?,
                    queued: count(r, 3)?,
                    fields: count(r, 4)?,
                    conflicts: count(r, 5)?,
                    removed: r.get(6)?,
                    ready: count(r, 7)?,
                })
            },
        )?)
    }
    pub fn fields(&self, after: Option<&str>) -> Result<Vec<Field>> {
        Ok(self.db.prepare("SELECT target,versions,revision FROM targets WHERE visible=1 AND target>? ORDER BY target LIMIT 50")?
            .query_map([after.unwrap_or("")], |r| {
                let target:String=r.get(0)?;
                let versions=count(r,1)?;
                Ok(Field { conflict:conflicting(&target,versions),target,versions,revision:count(r,2)? })
            })?
            .collect::<std::result::Result<_,_>>()?)
    }
    pub fn versions(&self, target: &str, after: Option<Uuid>) -> Result<Vec<Version>> {
        self.db.prepare("SELECT v.operation,o.device FROM versions v JOIN operations o ON o.id=v.operation JOIN targets t ON t.target=v.target WHERE t.visible=1 AND v.target=?1 AND v.operation>?2 ORDER BY v.operation LIMIT 50")?
            .query_map(params![target,after.map(|u|u.to_string()).unwrap_or_default()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?
            .map(|r| { let (op,dev)=r?; Ok(Version { operation:parse_uuid(&op)?, device:parse_uuid(&dev)? }) }).collect::<Result<_>>()
    }
    pub fn value(&self, target: &str, operation: Uuid) -> Result<Change> {
        let row = self.db.query_row("SELECT o.raw,v.position FROM versions v JOIN operations o ON o.id=v.operation JOIN targets t ON t.target=v.target WHERE t.visible=1 AND v.target=? AND v.operation=?", params![target,operation.to_string()], |r| Ok((r.get::<_,Vec<u8>>(0)?,r.get::<_,u32>(1)? as usize))).optional()?.ok_or(Error::Changed)?;
        Operation::decode(&row.0)?
            .changes
            .into_iter()
            .nth(row.1)
            .ok_or(Error::Storage)
    }
    pub fn next_upload(&self) -> Result<Option<Upload>> {
        self.db.query_row("SELECT id,raw,sha256,file_id FROM operations WHERE local=1 AND uploaded=0 ORDER BY seq LIMIT 1", [], |r| Ok((r.get::<_,String>(0)?,r.get::<_,Vec<u8>>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<String>>(3)?))).optional()?
            .map(|(id,raw,sha256,file_id)| Ok(Upload { operation:parse_uuid(&id)?,record:String::from_utf8(raw).map_err(|_|Error::Storage)?,sha256,file_id })).transpose()
    }
    pub fn reserve(&mut self, operation: Uuid, file_id: &str) -> Result<State> {
        if !valid_file_id(file_id) {
            return Err(Error::Identity);
        }
        let tx = self.db.transaction()?;
        let old = tx
            .query_row(
                "SELECT file_id FROM operations WHERE id=? AND local=1",
                [operation.to_string()],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .ok_or(Error::Changed)?;
        if old.as_deref().is_some_and(|old| old != file_id) {
            return Err(Error::Identity);
        }
        if old.is_none() {
            tx.execute(
                "UPDATE operations SET file_id=? WHERE id=?",
                params![file_id, operation.to_string()],
            )?;
            tx.execute("UPDATE state SET revision=revision+1", [])?;
        }
        tx.commit()?;
        self.state()
    }
    /// Only call after transport verifies this exact immutable file and digest.
    pub fn confirm(&mut self, operation: Uuid, file_id: &str, sha256: &str) -> Result<State> {
        let tx = self.db.transaction()?;
        let found = tx
            .query_row(
                "SELECT uploaded FROM operations WHERE id=? AND local=1 AND file_id=? AND sha256=?",
                params![operation.to_string(), file_id, sha256],
                |r| r.get::<_, bool>(0),
            )
            .optional()?
            .ok_or(Error::Identity)?;
        if !found {
            tx.execute(
                "UPDATE operations SET uploaded=1 WHERE id=?",
                [operation.to_string()],
            )?;
            tx.execute("UPDATE state SET queued=queued-1,revision=revision+1", [])?;
        }
        tx.commit()?;
        self.state()
    }
}
fn parse_uuid(value: &str) -> Result<Uuid> {
    Uuid::parse_str(value).map_err(|_| Error::Storage)
}
fn count(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(index)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })
}
fn conflicting(target: &str, versions: u64) -> bool {
    versions > 1 && !target.ends_with(":removed")
}
fn private_file(path: &Path) -> Result<File> {
    let mut options = File::options();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|_| Error::Storage)
}
fn valid_file_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub fn target(action: &Action) -> String {
    match action {
        Action::AccountConnection { account } => format!("account:{}:connection", account.id),
        Action::AccountName { id, .. } => format!("account:{id}:name"),
        Action::AccountRemoved { id } => format!("account:{id}:removed"),
        Action::Setting { key, .. } | Action::SettingRemoved { key } => format!(
            "setting:{}",
            serde_json::to_value(key).unwrap().as_str().unwrap()
        ),
        Action::ProfileName { .. } => "profile:name".into(),
        Action::ProfileRemoved => "profile:removed".into(),
    }
}
