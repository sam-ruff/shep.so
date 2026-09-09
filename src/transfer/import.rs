//! Stage an untrusted database into a private copy before reviewing or opening
//! it as a workspace. No Store is ever created from the selected input file.
use super::{BUSY_LIMIT, PAGES_PER_STEP};
use crate::{
    model::{Account, CalendarSource, Draft, Preferences},
    store::Store,
};
use anyhow::Context;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension,
    backup::{Backup, StepResult},
    config::DbConfig,
    limits::Limit,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::sync::{oneshot, watch};

mod fences;
mod install;
pub use install::{InstallPhase, Installation, Installed};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    CheckingSource,
    Copying,
    CheckingCopy,
    Reviewing,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub copied_pages: u32,
    pub total_pages: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSummary {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Review {
    pub bytes: u64,
    pub messages: u64,
    pub accounts: Vec<AccountSummary>,
    pub calendars: usize,
    pub drafts: usize,
    pub pending_outgoing: u64,
    pub pending_bulk: u64,
    pub pending_folders: u64,
    pub pending_moves: u64,
    pub pending_credentials: u64,
}

/// The reviewed file stays owned and private. Dropping a review removes it;
/// activation must consume this exact copy, never reopen the original filename.
#[derive(Debug)]
pub struct Prepared {
    file: tempfile::NamedTempFile,
    id: uuid::Uuid,
    pub review: Review,
}
impl Prepared {
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

pub struct Import {
    cancel: watch::Sender<bool>,
    pub progress: watch::Receiver<Progress>,
    result: Option<oneshot::Receiver<anyhow::Result<Option<Prepared>>>>,
}
impl Import {
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }

    pub async fn finish(&mut self) -> anyhow::Result<Option<Prepared>> {
        let result = self
            .result
            .as_mut()
            .context("This import was already observed")?
            .await;
        self.result = None;
        result.context("Database import stopped before acknowledging its result")?
    }
}

type Schema = BTreeMap<String, (String, String, Option<String>)>;

/// Start copying in a worker; the original workspace remains open and unchanged.
/// A prepared result is not authorization to activate it or replay its jobs.
pub async fn stage(store: Store, source: PathBuf) -> anyhow::Result<Import> {
    stage_observed(store, source, |_, _| {}).await
}

async fn stage_observed(
    store: Store,
    source: PathBuf,
    observe: impl FnMut(Progress, watch::Receiver<bool>) + Send + 'static,
) -> anyhow::Result<Import> {
    stage_checked(store, source, false, observe).await
}

#[cfg(feature = "test-support")]
pub(crate) async fn stage_fixture(
    store: Store,
    source: PathBuf,
    hold: bool,
) -> anyhow::Result<Import> {
    let mut held = false;
    stage_checked(store, source, true, move |progress, mut cancel| {
        if hold && progress.phase == Phase::Copying && !held {
            held = true;
            let _ = futures::executor::block_on(cancel.changed());
        }
    })
    .await
}

async fn stage_checked(
    store: Store,
    source: PathBuf,
    require_fixture: bool,
    mut observe: impl FnMut(Progress, watch::Receiver<bool>) + Send + 'static,
) -> anyhow::Result<Import> {
    // Share the existing export lease: one full transfer per cache, including
    // callers in another process. This name preserves older export coordination.
    let lease = store.bulk_lease("database-export".into()).await?;
    let directory = store
        .run(|c| {
            c.path()
                .filter(|p| !p.is_empty())
                .and_then(|p| Path::new(p).parent())
                .map(Path::to_path_buf)
                .context("Import requires a saved workspace")
        })
        .await?;
    // Derive executable schema only from application code, never from the
    // selected file or a live cache that could contain foreign additions.
    let expected = Store::memory()?.run(|c| schema(c)).await?;
    let (cancel, cancellation) = watch::channel(false);
    let (updates, progress) = watch::channel(Progress::default());
    let (reply, result) = oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let result = prepare(
            &source,
            &directory,
            &expected,
            require_fixture,
            &cancellation,
            |progress| {
                updates.send_replace(progress);
                observe(progress, cancellation.clone());
            },
        );
        drop(lease);
        let result = if cancelled(&cancellation) {
            Ok(None)
        } else {
            result.map(Some)
        };
        let _ = reply.send(result);
    });
    Ok(Import {
        cancel,
        progress,
        result: Some(result),
    })
}

fn cancelled(cancel: &watch::Receiver<bool>) -> bool {
    *cancel.borrow() || cancel.has_changed().is_err()
}

fn check_cancel(cancel: &watch::Receiver<bool>) -> anyhow::Result<()> {
    anyhow::ensure!(!cancelled(cancel), "Database import cancelled");
    Ok(())
}

fn defensive(connection: &Connection, cancel: &watch::Receiver<bool>) -> anyhow::Result<()> {
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE, true)?;
    connection.set_db_config(DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA, false)?;
    connection.set_limit(Limit::SQLITE_LIMIT_ATTACHED, 0)?;
    // Bound executable schema/query complexity, never message or database size.
    connection.set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 131_072)?;
    connection.set_limit(Limit::SQLITE_LIMIT_EXPR_DEPTH, 100)?;
    let cancellation = cancel.clone();
    connection.progress_handler(10_000, Some(move || cancelled(&cancellation)))?;
    connection.busy_timeout(Duration::from_millis(100))?;
    Ok(())
}

fn integrity(connection: &Connection, full: bool) -> anyhow::Result<()> {
    let sql = if full {
        "PRAGMA integrity_check(1)"
    } else {
        "PRAGMA quick_check(1)"
    };
    let answer: String = connection
        .query_row(sql, [], |row| row.get(0))
        .context("The selected file is not a readable SQLite database")?;
    anyhow::ensure!(
        answer == "ok",
        "The database is damaged. Export a fresh copy from the original device."
    );
    Ok(())
}

fn schema(connection: &Connection) -> anyhow::Result<Schema> {
    let mut result = Schema::new();
    let mut statement =
        connection.prepare("SELECT name,type,tbl_name,sql FROM sqlite_schema ORDER BY name")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let owner: String = row.get(2)?;
        let sql: Option<String> = row.get(3)?;
        // ANALYZE adds optional data-only tables with definitions owned by SQLite.
        let statistics = match name.as_str() {
            "sqlite_stat1" => Some("CREATE TABLE sqlite_stat1(tbl,idx,stat)"),
            "sqlite_stat4" => Some("CREATE TABLE sqlite_stat4(tbl,idx,neq,nlt,ndlt,sample)"),
            _ => None,
        };
        if statistics.is_some_and(|expected| {
            kind == "table" && owner == name && sql.as_deref() == Some(expected)
        }) {
            continue;
        }
        anyhow::ensure!(
            result.insert(name, (kind, owner, sql)).is_none(),
            "The database contains duplicate schema objects."
        );
    }
    Ok(result)
}

fn validate_schema(connection: &Connection, expected: &Schema) -> anyhow::Result<()> {
    let version: u32 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    anyhow::ensure!(
        (2..=crate::store::DATABASE_VERSION).contains(&version),
        "This database version is not supported. Use matching, current Shep versions on both devices."
    );
    let mut expected = expected.clone();
    if version < 4 {
        expected.retain(|_, (_, owner, _)| owner != "backup_history");
    }
    if version == 2 {
        expected.retain(|_, (_, owner, _)| owner != "imported_operations");
    }
    anyhow::ensure!(
        schema(connection)? == expected,
        "This database has an unsupported schema. Use matching, current Shep versions on both devices and export again."
    );
    Ok(())
}

fn prepare(
    source: &Path,
    directory: &Path,
    expected: &Schema,
    require_fixture: bool,
    cancel: &watch::Receiver<bool>,
    mut progress: impl FnMut(Progress),
) -> anyhow::Result<Prepared> {
    check_cancel(cancel)?;
    anyhow::ensure!(
        source.metadata()?.is_file(),
        "Choose a SQLite database file."
    );
    let source = Connection::open_with_flags(
        source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    defensive(&source, cancel)?;
    integrity(&source, false)?;
    source.execute_batch("PRAGMA cache_size=-2048; PRAGMA query_only=ON;")?;
    let snapshot = source.unchecked_transaction()?;
    // Pin before validation and page copying so changing the original file's
    // contents cannot replace the schema that was checked on this connection.
    snapshot.query_row("SELECT count(*) FROM sqlite_schema", [], |_| Ok(()))?;
    validate_schema(&snapshot, expected)?;
    if require_fixture {
        let marker: i32 = snapshot.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        anyhow::ensure!(
            marker == 0x5348_5054,
            "Preview can import only an owned fixture database"
        );
    }
    check_cancel(cancel)?;
    let file = tempfile::Builder::new()
        .prefix(".shep-import-")
        .suffix(".sqlite")
        .tempfile_in(directory)?;
    let mut copy = Connection::open(file.path())?;
    defensive(&copy, cancel)?;
    copy.execute_batch(
        "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA cache_size=-2048;",
    )?;
    let latest = {
        let backup = Backup::new(&snapshot, &mut copy)?;
        let mut stalled = None;
        loop {
            check_cancel(cancel)?;
            match backup.step(PAGES_PER_STEP)? {
                step @ (StepResult::More | StepResult::Done) => {
                    stalled = None;
                    let value = backup.progress();
                    let total_pages = u32::try_from(value.pagecount)?;
                    let remaining = u32::try_from(value.remaining)?;
                    let latest = Progress {
                        phase: Phase::Copying,
                        copied_pages: total_pages.saturating_sub(remaining),
                        total_pages,
                    };
                    progress(latest);
                    if step == StepResult::Done {
                        break latest;
                    }
                }
                StepResult::Busy | StepResult::Locked => {
                    anyhow::ensure!(
                        stalled.get_or_insert_with(Instant::now).elapsed() < BUSY_LIMIT,
                        "The source stayed busy. Close other database tools and retry the import."
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => anyhow::bail!("SQLite returned an unsupported import state"),
            }
        }
    };
    drop(snapshot);
    drop(source);
    progress(Progress {
        phase: Phase::CheckingCopy,
        ..latest
    });
    check_cancel(cancel)?;
    // Copying is followed by a full check of the actual private file. No data
    // query, migration, trigger or provider recovery runs before schema approval.
    integrity(&copy, true)?;
    validate_schema(&copy, expected)?;
    anyhow::ensure!(
        copy.query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
            .optional()?
            .is_none(),
        "The database contains broken references. Export a fresh copy from the original device."
    );
    progress(Progress {
        phase: Phase::Reviewing,
        ..latest
    });
    let mut review = review(&copy)?;
    copy.close().map_err(|(_, error)| error)?;
    check_cancel(cancel)?;
    file.as_file().sync_all()?;
    review.bytes = file.as_file().metadata()?.len();
    Ok(Prepared {
        file,
        id: uuid::Uuid::new_v4(),
        review,
    })
}

fn setting<T: serde::de::DeserializeOwned + Default>(
    c: &Connection,
    key: &str,
) -> anyhow::Result<T> {
    let value: Option<String> = c
        .query_row("SELECT value FROM kv WHERE key=?", [key], |r| r.get(0))
        .optional()?;
    value
        .map(|s| {
            serde_json::from_str(&s).context("The database contains unreadable settings or drafts")
        })
        .unwrap_or_else(|| Ok(T::default()))
}

fn count(c: &Connection, sql: &str) -> anyhow::Result<u64> {
    let value: i64 = c.query_row(sql, [], |r| r.get(0))?;
    Ok(u64::try_from(value)?)
}

fn review(c: &Connection) -> anyhow::Result<Review> {
    let accounts: Vec<Account> = setting(c, "accounts")?;
    for account in &accounts {
        account
            .validate()
            .context("An imported account needs corrected settings before export")?;
    }
    let calendars: Vec<CalendarSource> = setting(c, "calendars")?;
    crate::credentials::validate_connections(&accounts, &calendars)?;
    let drafts: Vec<Draft> = setting(c, "drafts")?;
    let preferences: Preferences = setting(c, "preferences")?;
    preferences
        .validate()
        .context("The imported preferences are not supported")?;
    Ok(Review {
        bytes: 0,
        messages: count(c, "SELECT count(*) FROM messages")?,
        accounts: accounts
            .into_iter()
            .map(|a| AccountSummary {
                name: a.name,
                email: a.email,
            })
            .collect(),
        calendars: calendars.len(),
        drafts: drafts.len(),
        pending_outgoing: count(
            c,
            "SELECT count(*) FROM outgoing WHERE stage NOT IN ('Complete','Released')",
        )?,
        pending_bulk: count(
            c,
            "SELECT count(*) FROM bulk_jobs WHERE EXISTS(SELECT 1 FROM bulk_items WHERE job=bulk_jobs.id AND status NOT IN ('done','cancelled'))",
        )?,
        pending_folders: count(c, "SELECT count(*) FROM folder_jobs WHERE closed=0")?,
        pending_moves: count(
            c,
            "SELECT count(*) FROM mail_moves WHERE stage NOT IN ('located','kept')",
        )?,
        pending_credentials: count(c, "SELECT count(*) FROM credential_cleanup")?,
    })
}

#[cfg(test)]
mod tests;
