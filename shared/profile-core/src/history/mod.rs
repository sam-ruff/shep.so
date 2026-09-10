//! Device-local causal history. The caller must authenticate the provider and
//! verify cloud file ownership before importing records. This journal does not
//! authenticate Google, apply accounts, or acknowledge a network upload itself.
//!
//! The protocol types and the pure in-memory journal compile everywhere; the
//! SQLite journal, its worker and file ownership need the native `history`
//! feature.
pub mod memory;
mod protocol;
pub use protocol::*;

#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
mod connection;
#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
mod export;
#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
mod merge;
#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
pub(crate) mod ownership;
#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
mod schema;
#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
mod worker;
#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
pub use connection::ConnectionFactory;
#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
pub use worker::Worker;

#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
mod journal {
    use super::*;
    use crate::{Action, Operation};
    use rusqlite::{Connection, OptionalExtension, params};
    use std::{fs::File, path::Path};
    use uuid::Uuid;

    impl From<rusqlite::Error> for Error {
        fn from(_: rusqlite::Error) -> Self {
            Self::Storage
        }
    }

    /// Synchronous implementation for an owning background thread, never a UI
    /// handler. Worker supplies the bounded async facade used by native callers.
    pub struct Journal {
        pub(super) db: Connection,
        pub(super) binding: Binding,
        pub(super) device: Uuid,
        pub(super) _lock: Option<ownership::OwnedLock>,
    }
    impl Journal {
        pub fn open(path: &Path, binding: Binding) -> Result<Self> {
            Self::open_with(path, binding, &ConnectionFactory::default())
        }
        /// Open under the ordinary file-ownership guard using a client-owned factory.
        /// The factory must key/configure the connection before returning it.
        pub fn open_with(
            path: &Path,
            binding: Binding,
            connections: &ConnectionFactory,
        ) -> Result<Self> {
            binding.validate()?;
            // Choose the companion lock from the actual file, including symlinks.
            private_file(path)?;
            let path = path.canonicalize().map_err(|_| Error::Storage)?;
            // The lock is independent of SQLite, retained until the connection drops.
            let mut lock_path = path.as_os_str().to_owned();
            lock_path.push(".history-lock");
            let lock = private_file(Path::new(&lock_path))?;
            use fs2::FileExt;
            lock.try_lock_exclusive().map_err(|error| {
                if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() {
                    Error::Owned
                } else {
                    Error::Storage
                }
            })?;
            let lock = ownership::OwnedLock::acquired(lock);
            let db = connections.open(&path)?;
            Self::initialize(db, binding, Some(lock))
        }
        pub fn memory(binding: Binding) -> Result<Self> {
            let connection = Connection::open_in_memory()?;
            connection.pragma_update(None, "temp_store", "FILE")?;
            Self::initialize(connection, binding, None)
        }
        fn initialize(
            mut db: Connection,
            binding: Binding,
            lock: Option<ownership::OwnedLock>,
        ) -> Result<Self> {
            binding.validate()?;
            db.execute_batch(
                "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
            )?;
            let device = schema::initialize(&mut db, &binding)?;
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
                Command::ExportRecord {
                    expected_revision,
                    after,
                } => self
                    .export_record(expected_revision, after)
                    .map(Reply::Record),
                Command::ExportAcknowledgedRecord {
                    expected_revision,
                    after,
                } => self
                    .export_acknowledged_record(expected_revision, after)
                    .map(Reply::Record),
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
            let mut state = self.db.query_row(
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
                        initialized: false,
                    })
                },
            )?;
            state.initialized = !state.removed && state.waiting == 0 && state.ready == 0 &&
                self.db.query_row("SELECT EXISTS(SELECT 1 FROM versions v JOIN targets t ON t.target=v.target JOIN operations o ON o.id=v.operation WHERE v.target='profile:setup' AND t.visible=1 AND t.versions=1 AND json_extract(CAST(o.raw AS TEXT),'$.changes[0].complete')=1)", [], |r|r.get::<_,bool>(0))?;
            Ok(state)
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
        pub fn overview(&self) -> Result<Overview> {
            let state = self.state()?;
            let names = self.versions("profile:name", None)?;
            let name = if names.len() == 1 {
                match self.value("profile:name", names[0].operation)?.action {
                    Action::ProfileName { name } => Some(name),
                    _ => return Err(Error::Storage),
                }
            } else {
                None
            };
            // Indexed visible-field ranges, independent of the operation history's
            // length. No full profile values or account arrays leave SQLite here.
            let accounts = self.db.query_row(
                "SELECT count(*) FROM targets WHERE visible=1 AND target>='account:' AND target<'account;' AND target GLOB '*:connection'", [], |r| count(r, 0))?;
            let settings = self.db.query_row(
                "SELECT count(*) FROM targets WHERE visible=1 AND target>='setting:' AND target<'setting;'", [], |r| count(r, 0))?;
            Ok(Overview {
                state,
                name,
                name_conflict: names.len() > 1,
                accounts,
                settings,
            })
        }
        pub fn versions(&self, target: &str, after: Option<Uuid>) -> Result<Vec<Version>> {
            self.db.prepare("SELECT v.operation,o.device FROM versions v JOIN operations o ON o.id=v.operation JOIN targets t ON t.target=v.target WHERE t.visible=1 AND v.target=?1 AND v.operation>?2 ORDER BY v.operation LIMIT 50")?
                .query_map(params![target,after.map(|u|u.to_string()).unwrap_or_default()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?
                .map(|r| { let (op,dev)=r?; Ok(Version { operation:parse_uuid(&op)?, device:parse_uuid(&dev)? }) }).collect::<Result<_>>()
        }
        pub fn value(&self, target: &str, operation: Uuid) -> Result<crate::Change> {
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
    pub(crate) fn count(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
        u64::try_from(row.get::<_, i64>(index)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })
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
}
#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
pub use journal::Journal;
#[cfg(all(feature = "history", not(target_arch = "wasm32")))]
pub(crate) use journal::count;
