//! Cross-process ownership for key creation and migration. State coordination
//! remains channel-owned; this file guard only excludes independent processes.
use anyhow::{Context, ensure};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};

const FILE: &str = ".cache-encryption.lock";

pub struct Guard {
    root: PathBuf,
    _lock: File,
}

impl Drop for Guard {
    fn drop(&mut self) {
        // End this owner's intent explicitly. A transient fork may retain a
        // CLOEXEC descriptor until exec; closing our descriptor alone would
        // unnecessarily keep its flock alive for that unrelated child.
        let _ = FileExt::unlock(&self._lock);
    }
}

impl Guard {
    /// Existing keyed workers retain a shared guard until admitted writes drain.
    pub fn reader(root: &Path) -> anyhow::Result<Self> {
        Self::acquire(root, false)
    }

    /// New key admission and legacy publication require exclusive ownership.
    pub fn migration(root: &Path) -> anyhow::Result<Self> {
        Self::acquire(root, true)
    }

    fn acquire(root: &Path, exclusive: bool) -> anyhow::Result<Self> {
        let root = root.canonicalize().context(
            "The cache folder is unavailable. Keep its contents and choose the original folder.",
        )?;
        ensure!(root.is_dir(), "The cache folder is not a directory.");
        let path = root.join(FILE);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                metadata.file_type().is_file() && metadata.len() == 0,
                "The cache ownership filename is occupied by another file. Its contents were kept."
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                lock.metadata()?.nlink() == 1,
                "The cache ownership file is linked elsewhere. No migration was started."
            );
        }
        let result = if exclusive {
            lock.try_lock_exclusive()
        } else {
            FileExt::try_lock_shared(&lock)
        };
        result.context("This cache is open in another Shep process or is being encrypted. Close its other windows and retry.")?;
        Ok(Self { root, _lock: lock })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_ownership_readers_exclude_migration_and_release_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let first = Guard::reader(dir.path()).unwrap();
        let second = Guard::reader(dir.path()).unwrap();
        assert!(Guard::migration(dir.path()).is_err());
        drop(first);
        assert!(Guard::migration(dir.path()).is_err());
        drop(second);
        let migration = Guard::migration(dir.path()).unwrap();
        assert!(Guard::reader(dir.path()).is_err());
        assert!(Guard::migration(dir.path()).is_err());
        assert_eq!(migration.root(), dir.path().canonicalize().unwrap());
        drop(migration);
        assert!(Guard::reader(dir.path()).is_ok());
    }

    #[test]
    fn cache_ownership_preserves_foreign_files_and_rejects_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        std::fs::write(&path, b"Keep this unrelated file").unwrap();
        assert!(Guard::migration(dir.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"Keep this unrelated file");
        #[cfg(unix)]
        {
            std::fs::remove_file(&path).unwrap();
            let target = dir.path().join("other");
            std::fs::write(&target, b"").unwrap();
            std::os::unix::fs::symlink(&target, &path).unwrap();
            assert!(Guard::migration(dir.path()).is_err());
            std::fs::remove_file(&path).unwrap();
            std::fs::hard_link(&target, &path).unwrap();
            assert!(Guard::migration(dir.path()).is_err());
        }
    }

    #[test]
    fn cache_ownership_is_enforced_between_processes() {
        let dir = tempfile::tempdir().unwrap();
        let run = |blocked: bool| {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "cache_cipher::ownership::tests::cache_ownership_child",
                    "--nocapture",
                ])
                .env("SHEP_FIXTURE_CACHE_ROOT", dir.path())
                .env(
                    "SHEP_FIXTURE_CACHE_BLOCKED",
                    if blocked { "1" } else { "0" },
                )
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        let reader = Guard::reader(dir.path()).unwrap();
        run(true);
        drop(reader);
        run(false);
    }

    #[test]
    fn cache_ownership_child() {
        let Some(path) = std::env::var_os("SHEP_FIXTURE_CACHE_ROOT") else {
            return;
        };
        let blocked = std::env::var("SHEP_FIXTURE_CACHE_BLOCKED").unwrap() == "1";
        assert_eq!(Guard::migration(Path::new(&path)).is_err(), blocked);
    }
}
