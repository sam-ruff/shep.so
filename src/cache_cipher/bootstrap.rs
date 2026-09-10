//! Startup ownership of a data root. One worker holds the exclusive guard from
//! key admission through recovery and publication of every database in the
//! root, then shares that guard with each database owner it hands out. No
//! catalog or profile connection opens before recovery has run.
use super::{Key, key_store::Keys, migration, ownership::Guard, publication};
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{mpsc, oneshot, watch};

/// Device-local root identity. It names the key namespace and is never read
/// from an imported database.
const MARKER: &str = ".cache-root";
const SYNC_DIRECTORY: &str = "profile-sync";
const PROFILES_DIRECTORY: &str = "profiles";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Use the device key only for a root that already has one. Never create a
    /// key or convert a plaintext root. This is the production policy.
    Existing,
    /// Create the device key for a plaintext root and publish encrypted copies
    /// of every database it contains before anything opens.
    Migrate,
}

#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    version: u8,
    id: uuid::Uuid,
}

/// Everything an owner in the root needs. The shared guard is released only
/// when the last owner has closed its connection after draining admitted work.
#[derive(Clone)]
pub struct Root {
    directory: PathBuf,
    key: Option<Arc<Key>>,
    guard: Arc<Guard>,
}

impl std::fmt::Debug for Root {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Root")
            .field("directory", &self.directory)
            .field("keyed", &self.key.is_some())
            .finish()
    }
}

impl Root {
    /// A plaintext root held as a reader, for owners that never migrate.
    pub fn reader(directory: &Path) -> anyhow::Result<Self> {
        let guard = Guard::reader(directory)?;
        Ok(Self {
            directory: guard.root().to_owned(),
            key: None,
            guard: Arc::new(guard),
        })
    }

    /// A keyed root held as a reader, for tests that bypass recovery.
    #[cfg(test)]
    pub(crate) fn keyed(directory: &Path, key: Arc<Key>) -> anyhow::Result<Self> {
        let mut root = Self::reader(directory)?;
        root.key = Some(key);
        Ok(root)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn key(&self) -> Option<Arc<Key>> {
        self.key.clone()
    }

    pub fn guard(&self) -> Arc<Guard> {
        self.guard.clone()
    }

    /// Open a database in this root with the root's routing. A keyed root never
    /// falls back to a plaintext connection.
    pub fn open(
        &self,
        path: &Path,
        flags: rusqlite::OpenFlags,
    ) -> anyhow::Result<rusqlite::Connection> {
        super::open(self.key.as_deref(), path, flags)
    }
}

pub type KeySource = Arc<dyn Fn(uuid::Uuid) -> Keys + Send + Sync>;

struct Request {
    directory: PathBuf,
    legacy_filename: String,
    policy: Policy,
    cancel: watch::Receiver<bool>,
    reply: oneshot::Sender<anyhow::Result<Root>>,
}

/// Handle to the owning worker. Requests are admitted one at a time.
#[derive(Clone)]
pub struct Bootstrap {
    commands: mpsc::Sender<Request>,
}

/// Dropping the open future cancels staging; the plaintext is kept.
struct CancelOnDrop(watch::Sender<bool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

impl Bootstrap {
    pub fn start(keys: KeySource) -> anyhow::Result<Self> {
        let (commands, mut input) = mpsc::channel::<Request>(1);
        std::thread::Builder::new()
            .name("shep-cache-root".into())
            .spawn(move || {
                while let Some(request) = input.blocking_recv() {
                    let result = open_root(
                        &keys,
                        &request.directory,
                        &request.legacy_filename,
                        request.policy,
                        &request.cancel,
                    );
                    let _ = request.reply.send(result);
                }
            })
            .context("Could not start the cache ownership worker")?;
        Ok(Self { commands })
    }

    pub async fn open(
        &self,
        directory: PathBuf,
        legacy_filename: String,
        policy: Policy,
    ) -> anyhow::Result<Root> {
        let (cancel, cancellation) = watch::channel(false);
        let _cancel = CancelOnDrop(cancel);
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Request {
                directory,
                legacy_filename,
                policy,
                cancel: cancellation,
                reply,
            })
            .await
            .map_err(|_| anyhow::anyhow!("The cache ownership worker is unavailable."))?;
        result
            .await
            .context("The cache ownership worker stopped before opening the data folder")?
    }
}

fn cancelled(cancel: &watch::Receiver<bool>) -> bool {
    *cancel.borrow() || cancel.has_changed().is_err()
}

fn open_root(
    keys: &KeySource,
    directory: &Path,
    legacy_filename: &str,
    policy: Policy,
    cancel: &watch::Receiver<bool>,
) -> anyhow::Result<Root> {
    fs::create_dir_all(directory)?;
    // A cooperating Shep that already owns the root has run recovery; this
    // process joins it as a reader, waiting briefly if that Shep is still
    // opening the root. Migration needs the root to itself.
    let (guard, exclusive) = match Guard::migration(directory) {
        Ok(guard) => (guard, true),
        Err(_) if policy == Policy::Existing => (Guard::join(directory)?, false),
        Err(error) => return Err(error),
    };
    let root = guard.root().to_owned();
    let key = match (read_marker(&root)?, policy) {
        (Some(id), _) => Some(futures::executor::block_on(keys(id).load())?),
        (None, Policy::Existing) => None,
        (None, Policy::Migrate) => {
            // The key exists before the marker names it, so a crash between the
            // two leaves a plaintext root that simply admits a fresh key later.
            let id = uuid::Uuid::new_v4();
            let key = futures::executor::block_on(keys(id).create())?;
            write_marker(&root, id)?;
            Some(key)
        }
    };
    if let Some(key) = &key {
        for main in inventory(&root, legacy_filename)? {
            ensure!(
                !cancelled(cancel),
                "Opening the cache was cancelled. The original database was kept."
            );
            if exclusive {
                let report = publication::recover(&guard, &main, key)?;
                if report.outcome != publication::Outcome::Idle || !report.kept.is_empty() {
                    tracing::info!(main = %main.display(), ?report, "cache encryption recovery");
                }
            }
            if !main.exists() {
                continue;
            }
            if !publication::starts_with_plaintext_header(&main)? {
                // Authenticate the key before any owner can create schema or
                // write beneath a mismatched key.
                key.open(&main, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .with_context(|| main.display().to_string())?;
                continue;
            }
            ensure!(
                exclusive,
                "This cache is being encrypted by another Shep process. Wait for it to finish and retry."
            );
            let directory = main.parent().context("The cache has no folder")?;
            let cancellation = cancel.clone();
            let candidate =
                migration::stage(&main, directory, key, move || cancelled(&cancellation))?;
            publication::publish(&guard, candidate, &main, key)?;
        }
    }
    let guard = if exclusive { guard.share()? } else { guard };
    // Sharing re-takes the lock, so another process may have owned the root in
    // between. Only key creation there could change the decision made above.
    ensure!(
        read_marker(&root)?.is_some() == key.is_some(),
        "Another Shep process changed this cache's encryption while it was being opened. Retry."
    );
    Ok(Root {
        directory: root,
        key: key.map(Arc::new),
        guard: Arc::new(guard),
    })
}

fn read_marker(root: &Path) -> anyhow::Result<Option<uuid::Uuid>> {
    let bytes = match fs::read(root.join(MARKER)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let marker: Marker = serde_json::from_slice(&bytes).context(
        "The cache identity file is unreadable. Keep the data folder and restore it from a backup.",
    )?;
    ensure!(
        marker.version == 1,
        "The cache identity was written by a newer Shep. Update Shep before opening it."
    );
    ensure!(!marker.id.is_nil(), "The cache identity file is invalid.");
    Ok(Some(marker.id))
}

fn write_marker(root: &Path, id: uuid::Uuid) -> anyhow::Result<()> {
    let file = tempfile::Builder::new()
        .prefix(".shep-cache-root-")
        .tempfile_in(root)?;
    serde_json::to_writer(file.as_file(), &Marker { version: 1, id })?;
    file.as_file().sync_all()?;
    file.persist_noclobber(root.join(MARKER))
        .map_err(|error| error.error)
        .context("Could not record the cache identity. No database was encrypted.")?;
    #[cfg(unix)]
    fs::File::open(root)?.sync_all()?;
    Ok(())
}

/// Every database this root may own, in a deterministic order, including
/// journalled mains whose file is currently absent. Import staging files and
/// candidates are not databases of the root.
fn inventory(root: &Path, legacy_filename: &str) -> anyhow::Result<Vec<PathBuf>> {
    let mut directories = vec![root.to_owned(), root.join(SYNC_DIRECTORY)];
    let profiles = root.join(PROFILES_DIRECTORY);
    if let Ok(entries) = fs::read_dir(&profiles) {
        let mut found = Vec::new();
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if uuid::Uuid::parse_str(&name).is_ok() {
                found.push(entry.path());
            }
        }
        found.sort();
        for profile in found {
            directories.push(profile.join(SYNC_DIRECTORY));
            directories.push(profile);
        }
    }
    let mut mains = Vec::new();
    for directory in directories {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        let mut names = std::collections::BTreeSet::new();
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if let Some(main) = name.strip_suffix(publication::JOURNAL_SUFFIX) {
                names.insert(main.to_owned());
            } else if name == legacy_filename
                || (name.ends_with(".sqlite") && !name.starts_with('.'))
            {
                names.insert(name);
            }
        }
        mains.extend(names.into_iter().map(|name| directory.join(name)));
    }
    Ok(mains)
}

#[cfg(test)]
mod tests;
