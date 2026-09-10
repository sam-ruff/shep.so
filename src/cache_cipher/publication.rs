//! Guarded replacement of a plaintext cache by a verified encrypted candidate.
//! A journal beside the main file lets startup resume or roll back after a
//! crash at any step without losing the plaintext until the replacement commits.
//!
//! State is derived from the journal plus which files exist:
//!
//! | main | candidate | recovery | meaning | action |
//! | --- | --- | --- | --- | --- |
//! | plain | yes | no | checkpointed, not retired | re-verify candidate, continue |
//! | no | yes | yes | plaintext retired | rename candidate in, continue |
//! | keyed | no | yes | published, plaintext kept | verify, dispose, finish |
//! | keyed | no | no | published, plaintext disposed | verify, finish |
//! | plain | no | no | candidate lost before publication | drop journal |
//! | no | no | yes | candidate lost after retirement | restore plaintext |
//! | anything else | | | not produced by this machine | keep everything |
use super::{Key, migration::Candidate, ownership::Guard};
use anyhow::{Context, bail, ensure};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) const CANDIDATE_PREFIX: &str = ".shep-encrypted-";
pub(super) const CANDIDATE_SUFFIX: &str = ".partial";
const SCRATCH_PREFIX: &str = ".shep-cache-scratch-";
const JOURNAL_SUFFIX: &str = ".encryption-journal";
const RECOVERY_SUFFIX: &str = ".plaintext-recovery";
const PLAINTEXT_HEADER: &[u8] = b"SQLite format 3\0";
const KEEP: &str = "Nothing was changed. Keep the cache folder contents and retry.";

/// Publication steps in order. Tests interrupt before a step to model a crash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Step {
    Checkpoint,
    Journal,
    Retire,
    Publish,
    Verify,
    Dispose,
    Finish,
}

impl Step {
    pub const ALL: [Step; 7] = [
        Step::Checkpoint,
        Step::Journal,
        Step::Retire,
        Step::Publish,
        Step::Verify,
        Step::Dispose,
        Step::Finish,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// No migration was journalled; the main file is whatever it was.
    Idle,
    /// The verified encrypted cache is in place and the journal is closed.
    Completed,
    /// The plaintext is back in place; the incomplete candidate was discarded.
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub outcome: Outcome,
    /// Candidate/scratch leftovers that belonged to no journalled migration.
    pub removed_orphans: usize,
    /// Files this machine did not create and therefore never deletes.
    pub kept: Vec<PathBuf>,
}

/// Cheap change detection for the plaintext between staging and publication.
/// It is a guard against a cooperating process ignoring the ownership lock,
/// not a substitute for that lock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Fingerprint {
    length: u64,
    modified: Option<(u64, u32)>,
    change_counter: u32,
    wal_length: u64,
}

impl Fingerprint {
    pub(super) fn read(main: &Path) -> anyhow::Result<Self> {
        let metadata = fs::metadata(main)?;
        let modified = metadata.modified().ok().and_then(|time| {
            let since = time.duration_since(std::time::UNIX_EPOCH).ok()?;
            Some((since.as_secs(), since.subsec_nanos()))
        });
        let mut header = [0u8; 28];
        let mut file = fs::File::open(main)?;
        let read = std::io::Read::read(&mut file, &mut header)?;
        let change_counter = if read == header.len() {
            u32::from_be_bytes([header[24], header[25], header[26], header[27]])
        } else {
            0
        };
        let wal_length = fs::metadata(sidecar(main, "-wal")).map_or(0, |m| m.len());
        Ok(Self {
            length: metadata.len(),
            modified,
            change_counter,
            wal_length,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    version: u8,
    candidate: String,
    user_version: i64,
    application_id: i64,
    plaintext: Fingerprint,
}

struct Layout {
    directory: PathBuf,
    main: PathBuf,
}

impl Layout {
    fn new(guard: &Guard, main: &Path) -> anyhow::Result<Self> {
        let name = main
            .file_name()
            .context("The cache path has no filename")?
            .to_owned();
        let directory = main
            .parent()
            .context("The cache path has no folder")?
            .canonicalize()
            .context("The cache folder is unavailable.")?;
        ensure!(
            directory.starts_with(guard.root()),
            "The cache is outside the owned data folder. {KEEP}"
        );
        Ok(Self {
            main: directory.join(name),
            directory,
        })
    }

    fn with_suffix(&self, suffix: &str) -> PathBuf {
        sidecar(&self.main, suffix)
    }

    fn journal(&self) -> PathBuf {
        self.with_suffix(JOURNAL_SUFFIX)
    }

    fn recovery(&self) -> PathBuf {
        self.with_suffix(RECOVERY_SUFFIX)
    }

    fn read_journal(&self) -> anyhow::Result<Option<Journal>> {
        let bytes = match fs::read(self.journal()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let journal: Journal = serde_json::from_slice(&bytes).context(
            "The cache encryption journal is unreadable. Keep the cache folder contents.",
        )?;
        ensure!(
            journal.version == 1,
            "The cache encryption journal was written by a newer Shep. Update Shep before opening it."
        );
        ensure!(
            Path::new(&journal.candidate)
                .file_name()
                .is_some_and(|name| name == journal.candidate.as_str()),
            "The cache encryption journal names an invalid candidate. Keep the cache folder contents."
        );
        Ok(Some(journal))
    }

    fn write_journal(&self, journal: &Journal) -> anyhow::Result<()> {
        let file = tempfile::Builder::new()
            .prefix(CANDIDATE_PREFIX)
            .suffix(CANDIDATE_SUFFIX)
            .tempfile_in(&self.directory)?;
        serde_json::to_writer(file.as_file(), journal)?;
        file.as_file().sync_all()?;
        file.persist(self.journal())
            .map_err(|error| error.error)
            .context("Could not record the cache encryption journal.")?;
        sync_directory(&self.directory)
    }
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn sync_directory(directory: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    fs::File::open(directory)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}

fn starts_with_plaintext_header(path: &Path) -> anyhow::Result<bool> {
    let mut header = [0u8; 16];
    let mut file = fs::File::open(path)?;
    let read = std::io::Read::read(&mut file, &mut header)?;
    Ok(read == header.len() && header[..] == *PLAINTEXT_HEADER)
}

fn remove_if_present(path: &Path) -> anyhow::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("Could not remove {}", path.display())),
    }
}

/// Replace `main` with a verified candidate staged from it. Requires the
/// exclusive guard, no open connection to `main`, and a candidate in the same
/// folder. Failure before the journal step leaves only the unchanged plaintext.
pub fn publish(guard: &Guard, candidate: Candidate, main: &Path, key: &Key) -> anyhow::Result<()> {
    publish_with(guard, candidate, main, key, &|_| false)
}

pub(crate) fn publish_with(
    guard: &Guard,
    candidate: Candidate,
    main: &Path,
    key: &Key,
    interrupt: &dyn Fn(Step) -> bool,
) -> anyhow::Result<()> {
    let layout = Layout::new(guard, main)?;
    ensure!(
        layout.read_journal()?.is_none(),
        "An earlier cache encryption is still recorded. Recover it before starting another. {KEEP}"
    );
    ensure!(
        !layout.recovery().exists(),
        "An earlier plaintext recovery file is still present. {KEEP}"
    );
    ensure!(
        starts_with_plaintext_header(&layout.main)?,
        "The cache is not a plaintext database. {KEEP}"
    );
    let candidate_directory = candidate
        .path()
        .parent()
        .context("The candidate has no folder")?
        .canonicalize()?;
    ensure!(
        candidate_directory == layout.directory,
        "The encrypted candidate must be staged beside the cache. {KEEP}"
    );
    ensure!(
        Fingerprint::read(&layout.main)? == *candidate.source(),
        "The cache changed after the encrypted copy was made. The copy was discarded and the original database was kept; retry."
    );
    let mut publication = Publication {
        layout,
        key,
        pending: Some(candidate),
        candidate: PathBuf::new(),
        user_version: 0,
        application_id: 0,
    };
    publication.run(Step::Checkpoint, interrupt)
}

struct Publication<'a> {
    layout: Layout,
    key: &'a Key,
    pending: Option<Candidate>,
    candidate: PathBuf,
    user_version: i64,
    application_id: i64,
}

impl Publication<'_> {
    fn run(&mut self, from: Step, interrupt: &dyn Fn(Step) -> bool) -> anyhow::Result<()> {
        for step in Step::ALL.into_iter().filter(|step| *step >= from) {
            ensure!(
                !interrupt(step),
                "Cache encryption was interrupted before {step:?}. Recovery resumes or restores it."
            );
            match step {
                Step::Checkpoint => self.checkpoint()?,
                Step::Journal => self.journal()?,
                Step::Retire => self.rename(&self.layout.main, &self.layout.recovery())?,
                Step::Publish => self.publish()?,
                Step::Verify => self.verify()?,
                Step::Dispose => self.dispose()?,
                Step::Finish => self.finish()?,
            }
        }
        Ok(())
    }

    /// Fold the legacy WAL into the main file and leave DELETE journalling so
    /// no WAL/SHM sidecar can outlive the rename. A busy checkpoint means
    /// another connection still holds the plaintext, which the guard forbids.
    fn checkpoint(&self) -> anyhow::Result<()> {
        let main = &self.layout.main;
        let c = Connection::open_with_flags(
            main,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        let mode: String = c.query_row("PRAGMA main.journal_mode", [], |r| r.get(0))?;
        if mode.eq_ignore_ascii_case("wal") {
            let busy: i64 =
                c.query_row("PRAGMA main.wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
            ensure!(
                busy == 0,
                "The cache is still open elsewhere, so its write-ahead log could not be folded in. Close other Shep windows and retry."
            );
            let mode: String = c.query_row("PRAGMA main.journal_mode=DELETE", [], |r| r.get(0))?;
            ensure!(
                mode.eq_ignore_ascii_case("delete"),
                "The cache is still open elsewhere, so its journal mode could not be changed. Close other Shep windows and retry."
            );
        }
        c.close().map_err(|(_, error)| error)?;
        for suffix in ["-wal", "-journal"] {
            let path = sidecar(main, suffix);
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };
            ensure!(
                metadata.len() == 0,
                "{} still holds unapplied changes. The original database was kept; retry after closing other Shep windows.",
                path.display()
            );
            remove_if_present(&path)?;
        }
        // An index file without its log carries no data once the header no
        // longer selects WAL; SQLite normally unlinks it on the last close.
        remove_if_present(&sidecar(main, "-shm"))?;
        fs::File::open(main)?.sync_all()?;
        sync_directory(&self.layout.directory)
    }

    fn journal(&mut self) -> anyhow::Result<()> {
        let candidate = self
            .pending
            .take()
            .context("The encrypted candidate was already consumed")?;
        let (user_version, application_id) = versions(self.key, candidate.path())?;
        let path = candidate.keep()?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("The candidate filename is not valid text")?
            .to_owned();
        let journal = Journal {
            version: 1,
            candidate: name,
            user_version,
            application_id,
            plaintext: Fingerprint::read(&self.layout.main)?,
        };
        if let Err(error) = self.layout.write_journal(&journal) {
            let _ = remove_if_present(&path);
            return Err(error);
        }
        self.candidate = path;
        self.user_version = user_version;
        self.application_id = application_id;
        Ok(())
    }

    fn rename(&self, from: &Path, to: &Path) -> anyhow::Result<()> {
        ensure!(
            !to.exists(),
            "{} already exists. Keep the cache folder contents and recover before retrying.",
            to.display()
        );
        fs::rename(from, to)
            .with_context(|| format!("Could not move {} to {}", from.display(), to.display()))?;
        sync_directory(&self.layout.directory)
    }

    fn publish(&self) -> anyhow::Result<()> {
        for suffix in ["-wal", "-shm", "-journal"] {
            ensure!(
                !sidecar(&self.candidate, suffix).exists(),
                "The encrypted candidate still has an open journal. Keep the cache folder contents and retry."
            );
        }
        self.rename(&self.candidate, &self.layout.main)
    }

    /// The keyed open authenticates every page it reads; the schema count and
    /// version pragmas prove the published file is the journalled candidate.
    fn verify(&self) -> anyhow::Result<()> {
        let (user_version, application_id) = versions(self.key, &self.layout.main)?;
        ensure!(
            (user_version, application_id) == (self.user_version, self.application_id),
            "The published cache does not match the journalled candidate."
        );
        Ok(())
    }

    /// The plaintext is only disposed of after the keyed replacement passed an
    /// authenticated read. Ordinary deletion is used; earlier copies and
    /// storage history cannot be retroactively encrypted.
    fn dispose(&self) -> anyhow::Result<()> {
        remove_if_present(&self.layout.recovery())?;
        sync_directory(&self.layout.directory)
    }

    fn finish(&self) -> anyhow::Result<()> {
        remove_if_present(&self.layout.journal())?;
        sync_directory(&self.layout.directory)
    }
}

fn versions(key: &Key, path: &Path) -> anyhow::Result<(i64, i64)> {
    let c = key.open(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let user_version = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let application_id = c.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    Ok((user_version, application_id))
}

fn verify_candidate(key: &Key, path: &Path, journal: &Journal) -> anyhow::Result<()> {
    let c = key.open(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let integrity: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(
        integrity == "ok",
        "The encrypted candidate failed validation."
    );
    let mut cipher = c.prepare("PRAGMA cipher_integrity_check")?;
    ensure!(
        cipher.query([])?.next()?.is_none(),
        "The encrypted candidate failed authentication."
    );
    drop(cipher);
    let user_version: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let application_id: i64 = c.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    ensure!(
        (user_version, application_id) == (journal.user_version, journal.application_id),
        "The encrypted candidate does not match the journal."
    );
    Ok(())
}

/// Resolve any interrupted publication of `main`, then remove leftovers that
/// belong to no journalled migration. Requires the exclusive guard.
pub fn recover(guard: &Guard, main: &Path, key: &Key) -> anyhow::Result<Report> {
    let layout = Layout::new(guard, main)?;
    let mut kept = Vec::new();
    let outcome = match layout.read_journal()? {
        Some(journal) => resume(&layout, journal, key)?,
        None => {
            if layout.recovery().exists() {
                kept.push(layout.recovery());
            }
            Outcome::Idle
        }
    };
    let removed_orphans = clean_orphans(guard, &layout.directory)?;
    Ok(Report {
        outcome,
        removed_orphans,
        kept,
    })
}

fn resume(layout: &Layout, journal: Journal, key: &Key) -> anyhow::Result<Outcome> {
    let candidate = layout.directory.join(&journal.candidate);
    let recovery = layout.recovery();
    let main_plain = if layout.main.exists() {
        Some(starts_with_plaintext_header(&layout.main)?)
    } else {
        None
    };
    let mut publication = Publication {
        layout: Layout {
            directory: layout.directory.clone(),
            main: layout.main.clone(),
        },
        key,
        pending: None,
        candidate: candidate.clone(),
        user_version: journal.user_version,
        application_id: journal.application_id,
    };
    let none = |_| false;
    match (main_plain, candidate.exists(), recovery.exists()) {
        (Some(true), true, false) => {
            let unchanged = Fingerprint::read(&layout.main)? == journal.plaintext;
            if !unchanged || verify_candidate(key, &candidate, &journal).is_err() {
                remove_if_present(&candidate)?;
                publication.finish()?;
                return Ok(Outcome::RolledBack);
            }
            // The journal step already ran; checkpointing again is idempotent.
            publication.checkpoint()?;
            publication.run(Step::Retire, &none)?;
            Ok(Outcome::Completed)
        }
        (None, true, true) => {
            if verify_candidate(key, &candidate, &journal).is_err() {
                remove_if_present(&candidate)?;
                publication.rename(&recovery, &layout.main)?;
                publication.finish()?;
                return Ok(Outcome::RolledBack);
            }
            publication.run(Step::Publish, &none)?;
            Ok(Outcome::Completed)
        }
        (Some(false), false, true) => {
            if publication.verify().is_err() {
                // The plaintext is complete: no write reached the encrypted
                // file before verification, so restoring it loses nothing.
                publication.rename(&layout.main, &candidate)?;
                publication.rename(&recovery, &layout.main)?;
                remove_if_present(&candidate)?;
                publication.finish()?;
                return Ok(Outcome::RolledBack);
            }
            publication.run(Step::Dispose, &none)?;
            Ok(Outcome::Completed)
        }
        (Some(false), false, false) => {
            publication.verify().context("The encrypted cache could not be unlocked and its plaintext was already disposed of. Keep the database and recover its original key.")?;
            publication.finish()?;
            Ok(Outcome::Completed)
        }
        (Some(true), false, false) => {
            publication.finish()?;
            Ok(Outcome::RolledBack)
        }
        (None, false, true) => {
            publication.rename(&recovery, &layout.main)?;
            publication.finish()?;
            Ok(Outcome::RolledBack)
        }
        (main, candidate, recovery) => bail!(
            "The cache folder is in a state Shep did not create (main {main:?}, candidate {candidate}, recovery {recovery}). Keep its contents and restore from a backup or contact support."
        ),
    }
}

/// Remove encrypted candidates and session scratch folders that belong to no
/// journalled migration. The exclusive guard proves no cooperating owner is
/// still using them.
pub fn clean_orphans(guard: &Guard, directory: &Path) -> anyhow::Result<usize> {
    let directory = directory.canonicalize()?;
    ensure!(
        directory.starts_with(guard.root()),
        "The folder is outside the owned data folder. {KEEP}"
    );
    let mut live = std::collections::HashSet::new();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if let Some(main) = name.strip_suffix(JOURNAL_SUFFIX) {
            let layout = Layout {
                directory: directory.clone(),
                main: directory.join(main),
            };
            if let Some(journal) = layout.read_journal()? {
                live.insert(journal.candidate);
            }
        }
    }
    let mut removed = 0;
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let file_type = entry.file_type()?;
        let stale_candidate =
            file_type.is_file() && candidate_base(&name).is_some_and(|base| !live.contains(base));
        let stale_scratch = name.starts_with(SCRATCH_PREFIX) && file_type.is_dir();
        if stale_candidate {
            fs::remove_file(entry.path())?;
            removed += 1;
        } else if stale_scratch {
            fs::remove_dir_all(entry.path())?;
            removed += 1;
        }
    }
    if removed > 0 {
        sync_directory(&directory)?;
    }
    Ok(removed)
}

/// The candidate name a file belongs to, including its SQLite sidecars.
fn candidate_base(name: &str) -> Option<&str> {
    if !name.starts_with(CANDIDATE_PREFIX) {
        return None;
    }
    let end = name.find(CANDIDATE_SUFFIX)? + CANDIDATE_SUFFIX.len();
    let (base, rest) = name.split_at(end);
    matches!(rest, "" | "-journal" | "-wal" | "-shm").then_some(base)
}

#[cfg(test)]
mod tests;
