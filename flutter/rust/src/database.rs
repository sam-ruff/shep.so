//! Independent cached reads and FIFO writes. No filesystem or SQLite on Dart's
//! UI isolate, and no credential material in this schema.
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

static PROFILES: OnceLock<Mutex<HashMap<PathBuf, Weak<Database>>>> = OnceLock::new();

pub struct Database {
    pub(crate) path: PathBuf,
    pub operations: Arc<crate::operations::Operations>,
    readers: [Arc<Mutex<Connection>>; 2],
    writer: Arc<Mutex<Connection>>,
    selections: Arc<Mutex<Connection>>,
    selection_order: Arc<AsyncMutex<()>>,
    selection_slots: Arc<Semaphore>,
    order: Arc<AsyncMutex<()>>,
    reads: Arc<Semaphore>,
    writes: Arc<Semaphore>,
    next: AtomicUsize,
    lease: Arc<File>,
}
fn lock_profile(file: &File) -> Result<()> {
    #[cfg(target_os = "android")]
    let result: std::io::Result<()> = {
        use std::os::fd::AsRawFd;
        // SAFETY: File owns this live descriptor for the whole call; flock
        // takes no pointers and does not transfer ownership. Closing the final
        // Arc<File> releases the kernel lock. std's pinned Android backend
        // currently returns Unsupported for its equivalent File::try_lock.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    };
    #[cfg(not(target_os = "android"))]
    let result: std::io::Result<()> = file.try_lock().map_err(Into::into);
    result.map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            anyhow::anyhow!("This profile is open in another Shep process. Close that process and reopen this profile.")
        } else {
            anyhow::anyhow!("The device could not lock its private mail cache. Reopen Shep or check device storage: {error}")
        }
    })
}

impl Database {
    pub async fn open(path: String) -> Result<Arc<Self>> {
        tokio::task::spawn_blocking(move || {
            let path = PathBuf::from(path);
            anyhow::ensure!(
                path.is_absolute(),
                "Use the application's private data directory."
            );
            let parent = path.parent().context("Missing application directory")?;
            std::fs::create_dir_all(parent)?;
            // Resolve a symlink alias before choosing the companion lock. The
            // lock file is never removed: replacing it would split ownership.
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)?;
            let path = path.canonicalize()?;
            let mut registry = PROFILES
                .get_or_init(Mutex::default)
                .lock()
                .map_err(|_| anyhow::anyhow!("Profile coordination failed. Reopen Shep."))?;
            registry.retain(|_, p| p.strong_count() != 0);
            if let Some(profile) = registry.get(&path).and_then(Weak::upgrade) {
                let version: u32 = profile.readers[0]
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Cache connection failed. Reopen Shep."))?
                    .query_row("PRAGMA user_version", [], |r| r.get(0))?;
                anyhow::ensure!(
                    version <= 24,
                    "This cache requires a newer Shep version. Update before reopening it."
                );
                return Ok(profile);
            }
            let mut lock_path = path.as_os_str().to_owned();
            lock_path.push(".owner-lock");
            let lease = Arc::new(
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(lock_path)?,
            );
            lock_profile(&lease)?;
            let writer = Connection::open(&path)?;
            writer.busy_timeout(std::time::Duration::from_secs(5))?;
            writer.execute_batch(
                "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
            )?;
            let version: u32 = writer.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            anyhow::ensure!(
                version <= 24,
                "This cache requires a newer Shep version. Update before reopening it."
            );
            if version > 0 && version < 24 {
                let existing: bool = writer.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='folder_creations')", [], |row| row.get(0))?;
                let column: bool = writer.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('folder_creations') WHERE name='mutation')", [], |row| row.get(0))?;
                if existing && !column {
                    writer.execute("ALTER TABLE folder_creations ADD COLUMN mutation TEXT", [])?;
                }
            }
            if version > 0 && version < 17 {
                let has_actions: bool = writer.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='individual_mail_actions')",
                    [],
                    |row| row.get(0),
                )?;
                let has_accepted_fields: bool = writer.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('individual_mail_actions') WHERE name='accepted_fields')",
                    [],
                    |row| row.get(0),
                )?;
                if has_actions && !has_accepted_fields {
                    writer.execute(
                        "ALTER TABLE individual_mail_actions ADD COLUMN accepted_fields TEXT",
                        [],
                    )?;
                }
            }
            if version > 0 && version < 18 {
                let has_slots: bool = writer.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='credential_slots')",
                    [],
                    |row| row.get(0),
                )?;
                let has_error: bool = writer.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('credential_slots') WHERE name='error')",
                    [],
                    |row| row.get(0),
                )?;
                if has_slots && !has_error {
                    writer.execute("ALTER TABLE credential_slots ADD COLUMN error TEXT", [])?;
                }
            }
            if version > 0 && version < 19 {
                writer.execute_batch(
                    "CREATE TABLE IF NOT EXISTS calendar_events(source_id TEXT NOT NULL,id TEXT NOT NULL,event TEXT NOT NULL,PRIMARY KEY(source_id,id));
                     CREATE TABLE IF NOT EXISTS calendar_sources(id TEXT PRIMARY KEY,source TEXT NOT NULL);
                     CREATE TABLE IF NOT EXISTS calendar_actions(id TEXT PRIMARY KEY,status TEXT NOT NULL CHECK(status IN ('queued','running','waiting','succeeded','rejected','uncertain','repair','cancelled')),error TEXT,created INTEGER NOT NULL,mutation TEXT NOT NULL);
                     CREATE TABLE IF NOT EXISTS calendar_action_receipts(action TEXT PRIMARY KEY REFERENCES calendar_actions(id) ON DELETE CASCADE,receipt TEXT NOT NULL);
                     CREATE TABLE IF NOT EXISTS calendar_intents(source_id TEXT NOT NULL,event_id TEXT NOT NULL,action TEXT NOT NULL REFERENCES calendar_actions(id),PRIMARY KEY(source_id,event_id));
                     CREATE INDEX IF NOT EXISTS calendar_action_status ON calendar_actions(status,created,id);
                     CREATE INDEX IF NOT EXISTS calendar_action_history ON calendar_actions(created DESC,id);
                     CREATE INDEX IF NOT EXISTS calendar_action_attention ON calendar_actions(created DESC,id) WHERE status NOT IN ('succeeded','cancelled');",
                )?;
            }
            if version > 0 && version < 20 {
                let has_subject:bool=writer.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('calendar_actions') WHERE name='subject')",[],|row|row.get(0))?;
                if !has_subject { writer.execute("ALTER TABLE calendar_actions ADD COLUMN subject TEXT",[])?; }
                writer.execute_batch("CREATE TABLE IF NOT EXISTS calendar_binding(id INTEGER PRIMARY KEY CHECK(id=1),subject TEXT NOT NULL); CREATE TABLE IF NOT EXISTS calendar_clock(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL); INSERT OR IGNORE INTO calendar_clock VALUES(1,0);")?;
            }
            if version > 0 && version < 21 {
                let has_request: bool = writer.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('calendar_actions') WHERE name='admission_request')",
                    [],
                    |row| row.get(0),
                )?;
                if !has_request {
                    writer.execute(
                        "ALTER TABLE calendar_actions ADD COLUMN admission_request TEXT",
                        [],
                    )?;
                }
            }
            if version > 0 && version < 22 {
                let has_connection: bool = writer.query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('calendar_sources') WHERE name='connection_id')",
                    [],
                    |row| row.get(0),
                )?;
                if !has_connection {
                    writer.execute(
                        "ALTER TABLE calendar_sources ADD COLUMN connection_id TEXT REFERENCES calendar_connections(id) ON DELETE CASCADE",
                        [],
                    )?;
                }
                for (column, kind) in [
                    ("connection_id", "TEXT"),
                    ("connection_revision", "INTEGER"),
                    ("credential_slot", "TEXT"),
                ] {
                    let exists: bool = writer.query_row(
                        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('calendar_actions') WHERE name=?1)",
                        [column],
                        |row| row.get(0),
                    )?;
                    if !exists {
                        writer.execute(
                            &format!("ALTER TABLE calendar_actions ADD COLUMN {column} {kind}"),
                            [],
                        )?;
                    }
                }
            }
            writer.execute_batch(include_str!("schema.sql"))?;
            writer.execute_batch(include_str!("folders/schema.sql"))?;
            crate::folders::recover(&writer)?;
            // Exclusive ownership proves no previous native process can still
            // finish these SMTP operations. They require explicit review.
            writer.execute(
                "UPDATE outgoing SET state='uncertain' WHERE state='submitting'",
                [],
            )?;
            writer.execute(
                "UPDATE outgoing_sent SET state='uncertain' WHERE state='appending'",
                [],
            )?;
            writer.execute(
                "UPDATE individual_mail_actions SET status='repair',error='The provider acknowledged this change. Finish saving it locally.' WHERE status='running' AND EXISTS(SELECT 1 FROM individual_mail_action_receipts WHERE action=individual_mail_actions.id)",
                [],
            )?;
            writer.execute(
                "UPDATE individual_mail_actions SET status='uncertain',error='The app closed before the provider result was saved. Refresh the affected folders before retrying.' WHERE status='running'",
                [],
            )?;
            writer.execute(
                "UPDATE calendar_actions SET status='repair',error='The calendar provider saved this event. Finish saving it on this device.' WHERE status='running' AND EXISTS(SELECT 1 FROM calendar_action_receipts WHERE action=calendar_actions.id)",
                [],
            )?;
            writer.execute(
                "UPDATE calendar_actions SET status='uncertain',error='The app closed before the provider result was saved. Inspect the event before retrying.' WHERE status='running'",
                [],
            )?;
            crate::calendar::connections::restart(&writer)?;
            crate::groups::restart(&writer)?;
            let reader = || -> Result<Arc<Mutex<Connection>>> {
                let connection =
                    Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
                connection.busy_timeout(std::time::Duration::from_secs(5))?;
                Ok(Arc::new(Mutex::new(connection)))
            };
            let selections = reader()?;
            {
                let db = selections
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Selection storage failed. Reopen Shep."))?;
                crate::selection::schema(&db)?;
            }
            let profile = Arc::new(Self {
                path: path.clone(),
                operations: Arc::new(crate::operations::Operations::new()),
                readers: [reader()?, reader()?],
                writer: Arc::new(Mutex::new(writer)),
                selections,
                selection_order: Arc::new(AsyncMutex::new(())),
                selection_slots: Arc::new(Semaphore::new(32)),
                order: Arc::new(AsyncMutex::new(())),
                reads: Arc::new(Semaphore::new(32)),
                writes: Arc::new(Semaphore::new(32)),
                next: AtomicUsize::new(0),
                lease,
            });
            registry.insert(path, Arc::downgrade(&profile));
            Ok(profile)
        })
        .await?
    }
    #[cfg(test)]
    pub fn pending_writes(&self) -> usize {
        32 - self.writes.available_permits()
    }
    #[cfg(test)]
    pub fn pending_selections(&self) -> usize {
        32 - self.selection_slots.available_permits()
    }
    pub async fn selection<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let permit = self
            .selection_slots
            .clone()
            .try_acquire_owned()
            .context("Selection is catching up. Retry that action shortly.")?;
        let ordered = self.selection_order.clone().lock_owned().await;
        let connection = self.selections.clone();
        let lease = self.lease.clone();
        tokio::task::spawn_blocking(move || {
            let (_permit, _ordered, _lease) = (permit, ordered, lease);
            let mut db = connection
                .lock()
                .map_err(|_| anyhow::anyhow!("Selection storage failed. Reopen Shep."))?;
            job(&mut db)
        })
        .await?
    }
    pub async fn read<T: Send + 'static>(
        &self,
        job: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let permit = self
            .reads
            .clone()
            .try_acquire_owned()
            .context("Cached reads are busy. Retry shortly.")?;
        let connection = self.readers[self.next.fetch_add(1, Ordering::Relaxed) % 2].clone();
        let lease = self.lease.clone();
        tokio::task::spawn_blocking(move || {
            let (_permit, _lease) = (permit, lease);
            let connection = connection
                .lock()
                .map_err(|_| anyhow::anyhow!("Cache connection failed. Reopen Shep."))?;
            job(&connection)
        })
        .await?
    }
    pub async fn write<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let permit = self
            .writes
            .clone()
            .try_acquire_owned()
            .context("Saving is busy. Retry shortly.")?;
        let ordered = self.order.clone().lock_owned().await;
        let connection = self.writer.clone();
        let lease = self.lease.clone();
        tokio::task::spawn_blocking(move || {
            // Cancellation cannot release FIFO order or profile ownership
            // before the blocking write actually finishes.
            let (_permit, _ordered, _lease) = (permit, ordered, lease);
            let mut connection = connection
                .lock()
                .map_err(|_| anyhow::anyhow!("Cache connection failed. Reopen Shep."))?;
            job(&mut connection)
        })
        .await?
    }
}
