//! Complete database snapshots. Copying owns a separate read connection and
//! a private destination file, never the mail-cache worker for its duration.
use crate::store::Store;
use anyhow::Context;
use rusqlite::{
    Connection, OpenFlags,
    backup::{Backup, StepResult},
};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::sync::{oneshot, watch};

const PAGES_PER_STEP: i32 = 128;
const BUSY_LIMIT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Preparing,
    Copying,
    Finishing,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Progress {
    pub phase: Phase,
    pub copied_pages: u32,
    pub total_pages: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Saved {
        path: PathBuf,
        bytes: u64,
        warning: Option<String>,
    },
    Cancelled,
}

#[derive(Debug, Clone)]
pub enum Request {
    Export {
        request: u64,
        destination: PathBuf,
        replace: bool,
    },
    Cancel(u64),
}

#[derive(Debug, Clone)]
pub enum Update {
    Progress(Progress),
    Finished(Result<Outcome, String>),
}

pub struct Export {
    cancel: Option<oneshot::Sender<()>>,
    pub progress: watch::Receiver<Progress>,
    result: Option<oneshot::Receiver<anyhow::Result<Outcome>>>,
}
impl Export {
    pub fn cancel(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
    }

    /// Cancelling this wait leaves its acknowledgment available to a later wait.
    /// Dropping the whole Export cancels copying at the next page boundary.
    pub async fn finish(&mut self) -> anyhow::Result<Outcome> {
        let result = self
            .result
            .as_mut()
            .context("This export was already observed")?
            .await;
        self.result = None;
        result.context("The database export stopped before acknowledging its result")?
    }
}

/// `replace` is permission to replace the exact file selected in a Save dialog.
/// Every export commits through an atomic rename; the active cache is forbidden.
pub async fn export_database(
    store: Store,
    destination: PathBuf,
    replace: bool,
) -> anyhow::Result<Export> {
    start_export(store, destination, replace, |_, _| {}).await
}

/// Only the isolated native harness can request this hold. The copy owns real
/// SQLite handles and a private file until the ordinary Cancel/close path runs.
#[cfg(feature = "test-support")]
pub(crate) async fn export_fixture_database(
    store: Store,
    destination: PathBuf,
    replace: bool,
) -> anyhow::Result<Export> {
    let mut held = false;
    start_export(store, destination, replace, move |progress, cancel| {
        if progress.phase == Phase::Copying && !held {
            held = true;
            let _ = futures::executor::block_on(cancel);
        }
    })
    .await
}

async fn start_export(
    store: Store,
    destination: PathBuf,
    replace: bool,
    mut observe: impl FnMut(Progress, &mut oneshot::Receiver<()>) + Send + 'static,
) -> anyhow::Result<Export> {
    // Bound full snapshots independently of provider/read/persistence capacity.
    // The existing lease also excludes another process exporting this cache.
    let lease = store.bulk_lease("database-export".into()).await?;
    let source = store
        .run(|connection| {
            connection
                .path()
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .context("Database export requires a saved workspace")
        })
        .await?;
    let (cancel, cancelled) = oneshot::channel();
    let (progress, updates) = watch::channel(Progress::default());
    let (reply, result) = oneshot::channel();
    tokio::task::spawn_blocking(move || {
        let outcome = copy_database(
            &source,
            &destination,
            replace,
            cancelled,
            |value, cancel| {
                progress.send_replace(value);
                observe(value, cancel);
            },
        );
        drop(lease);
        let _ = reply.send(outcome);
    });
    Ok(Export {
        cancel: Some(cancel),
        progress: updates,
        result: Some(result),
    })
}

fn cancelled(input: &mut oneshot::Receiver<()>) -> bool {
    !matches!(input.try_recv(), Err(oneshot::error::TryRecvError::Empty))
}

fn copy_database(
    source: &Path,
    destination: &Path,
    replace: bool,
    mut cancel: oneshot::Receiver<()>,
    mut progress: impl FnMut(Progress, &mut oneshot::Receiver<()>),
) -> anyhow::Result<Outcome> {
    if cancelled(&mut cancel) {
        return Ok(Outcome::Cancelled);
    }
    let (source, destination) = checked_destination(source, destination)?;
    let parent = destination.parent().context("Choose an export folder")?;
    anyhow::ensure!(
        replace || !destination.try_exists()?,
        "This export file already exists. Choose another name or confirm replacement."
    );
    let temporary = tempfile::Builder::new()
        .prefix(".shep-export-")
        .suffix(".partial")
        .tempfile_in(parent)
        .context("Could not create the export file. Check the folder and available space.")?;
    let input = Connection::open_with_flags(
        &source,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    input.busy_timeout(Duration::from_millis(100))?;
    input.execute_batch("PRAGMA cache_size=-2048; PRAGMA query_only=ON;")?;
    // Pin the source generation. Later WAL writes remain writable and are not
    // mixed into this snapshot or able to restart a large copy indefinitely.
    let snapshot = input.unchecked_transaction()?;
    snapshot.query_row("SELECT count(*) FROM sqlite_schema", [], |row| {
        row.get::<_, i64>(0)
    })?;
    let mut output = Connection::open(temporary.path())?;
    output.execute_batch(
        "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA cache_size=-2048;",
    )?;
    let last = {
        let backup = Backup::new(&snapshot, &mut output)?;
        let mut stalled_since = None;
        loop {
            if cancelled(&mut cancel) {
                return Ok(Outcome::Cancelled);
            }
            let step = backup.step(PAGES_PER_STEP)?;
            match step {
                StepResult::Done | StepResult::More => {
                    stalled_since = None;
                    let value = backup.progress();
                    let total_pages = u32::try_from(value.pagecount)?;
                    let remaining = u32::try_from(value.remaining)?;
                    let last = Progress {
                        phase: Phase::Copying,
                        copied_pages: total_pages.saturating_sub(remaining),
                        total_pages,
                    };
                    progress(last, &mut cancel);
                    if step == StepResult::Done {
                        break last;
                    }
                }
                StepResult::Busy | StepResult::Locked => {
                    anyhow::ensure!(
                        stalled_since.get_or_insert_with(Instant::now).elapsed() < BUSY_LIMIT,
                        "The database stayed busy. Retry the export when the other database operation has finished."
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                _ => anyhow::bail!("SQLite returned an unsupported snapshot state"),
            }
        }
    };
    // A standalone database must not depend on a temporary WAL/SHM filename.
    let journal: String = output.query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))?;
    anyhow::ensure!(
        journal == "delete",
        "Could not finish the standalone database export"
    );
    output.close().map_err(|(_, error)| error)?;
    drop(snapshot);
    drop(input);
    progress(
        Progress {
            phase: Phase::Finishing,
            ..last
        },
        &mut cancel,
    );
    if cancelled(&mut cancel) {
        return Ok(Outcome::Cancelled);
    }
    temporary.as_file().sync_all()?;
    let bytes = temporary.as_file().metadata()?.len();
    if cancelled(&mut cancel) {
        return Ok(Outcome::Cancelled);
    }
    if replace {
        temporary.persist(&destination)
    } else {
        temporary.persist_noclobber(&destination)
    }
    .context("Could not publish the database export. The destination was not replaced.")?;
    // Commit has happened. A directory flush failure must never look like a
    // missing copy and cause the user to overwrite it by retrying blindly.
    #[cfg(unix)]
    let warning = std::fs::File::open(parent).and_then(|directory| directory.sync_all()).err()
        .map(|_| "Database exported, but the folder could not confirm its final disk flush. Keep this copy and check the destination drive.".into());
    #[cfg(not(unix))]
    let warning = None;
    Ok(Outcome::Saved {
        path: destination,
        bytes,
        warning,
    })
}

fn checked_destination(source: &Path, destination: &Path) -> anyhow::Result<(PathBuf, PathBuf)> {
    anyhow::ensure!(destination.is_absolute(), "Choose an absolute export path");
    let source = source.canonicalize()?;
    let source_parent = source
        .parent()
        .context("The mail cache has no parent folder")?;
    let filename = destination
        .file_name()
        .context("Choose an export filename")?;
    let destination = destination
        .parent()
        .context("Choose an export folder")?
        .canonicalize()?
        .join(filename);
    let mut protected = vec![source.clone(), source_parent.join("backup-uploads.sqlite")];
    for database in protected.clone() {
        for suffix in ["-wal", "-shm", "-journal"] {
            let mut name = database.as_os_str().to_os_string();
            name.push(suffix);
            protected.push(PathBuf::from(name));
        }
    }
    for path in protected {
        anyhow::ensure!(
            !same_path(&path, &destination) && !same_file(&path, &destination)?,
            "Choose a destination outside Shep's active database and journal files."
        );
    }
    anyhow::ensure!(
        !destination.starts_with(source_parent.join("bulk-locks")),
        "Choose a destination outside Shep's operation lock folder."
    );
    Ok((source, destination))
}

fn same_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn same_file(a: &Path, b: &Path) -> anyhow::Result<bool> {
    if !a.try_exists()? || !b.try_exists()? {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // stat, not open/close: closing an ordinary descriptor to a live SQLite
        // file can invalidate that process's POSIX advisory locks.
        let a = a.metadata()?;
        let b = b.metadata()?;
        Ok(a.dev() == b.dev() && a.ino() == b.ino())
    }
    #[cfg(windows)]
    {
        Ok(same_file::is_same_file(a, b)?)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Ok(a.canonicalize()? == b.canonicalize()?)
    }
}

#[cfg(test)]
mod tests;
