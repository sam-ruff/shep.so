//! All provider/history state belongs to one local workspace, not the shared ID
//! alone. Database transfer protects the whole folder, including journal aliases.
use anyhow::Context;
use shep_profile_core::history;
use std::path::{Path, PathBuf};

pub(crate) const DIRECTORY: &str = "profile-sync";
#[derive(Clone)]
pub(crate) struct Paths {
    root: PathBuf,
}
impl Paths {
    pub fn for_cache(cache: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            root: cache
                .parent()
                .context("The mail cache needs a workspace directory")?
                .join(DIRECTORY),
        })
    }
    pub async fn journal(&self) -> anyhow::Result<super::journal::Journal> {
        let root = self.root.clone();
        tokio::task::spawn_blocking(move || {
            std::fs::create_dir_all(&root)?;
            super::journal::Journal::open(Some(&root.join("drive.sqlite")))
        })
        .await?
    }
    pub fn history(&self, binding: &history::Binding) -> anyhow::Result<PathBuf> {
        Ok(self.root.join(format!("{}.sqlite", binding.storage_key()?)))
    }
}

/// Export's destination parent is already canonicalized. Inspect every member
/// of this flat provider directory so a hard link or symlink cannot overwrite a
/// live history, its SQLite sidecar, or the independent-process ownership file.
pub(crate) fn protect(cache_parent: &Path, target: &Path) -> anyhow::Result<()> {
    let root = cache_parent.join(DIRECTORY);
    let canonical = match root.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => root.clone(),
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(
        !target.starts_with(&root)
            && !target.starts_with(&canonical)
            && !crate::transfer::same_path(&root, target)
            && !crate::transfer::same_path(&canonical, target),
        "Choose an export destination outside Shep's profile sync data."
    );
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = entry?.path();
        anyhow::ensure!(
            !crate::transfer::same_file(&path, target)?,
            "This export destination aliases active profile sync data. Choose another file."
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn denied(store: &crate::store::Store, path: PathBuf) {
        let mut export = crate::transfer::export_database(store.clone(), path, true)
            .await
            .unwrap();
        assert!(export.finish().await.is_err());
    }
    #[tokio::test]
    async fn profile_paths_are_workspace_scoped_and_export_protects_every_existing_file_and_alias()
    {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache.sqlite");
        let store = crate::store::Store::open(&cache).unwrap();
        let paths = Paths::for_cache(&cache).unwrap();
        let _journal = paths.journal().await.unwrap();
        let binding = history::Binding {
            namespace: "so.shep.fixture".into(),
            principal: "drive:fixture".into(),
            profile: uuid::Uuid::new_v4(),
            generation: uuid::Uuid::new_v4(),
        };
        let history_path = paths.history(&binding).unwrap();
        let worker = history::Worker::open(history_path.clone(), binding.clone())
            .await
            .unwrap();
        assert_ne!(
            history_path,
            Paths::for_cache(&dir.path().join("another/cache.sqlite"))
                .unwrap()
                .history(&binding)
                .unwrap()
        );
        let entries = std::fs::read_dir(&paths.root)
            .unwrap()
            .map(|p| p.unwrap().path())
            .collect::<Vec<_>>();
        assert!(entries.iter().any(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("history-lock")
        }));
        for (_index, path) in entries.iter().enumerate() {
            denied(&store, path.clone()).await;
            #[cfg(unix)]
            {
                let alias = dir.path().join(format!("alias-{_index}.sqlite"));
                std::fs::hard_link(path, &alias).unwrap();
                denied(&store, alias.clone()).await;
                std::fs::remove_file(&alias).unwrap();
                std::os::unix::fs::symlink(path, &alias).unwrap();
                denied(&store, alias).await;
            }
        }
        assert!(protect(dir.path(), &paths.root.join("future-history.sqlite")).is_err());
        assert!(protect(dir.path(), &dir.path().join("ordinary-export.sqlite")).is_ok());
        worker.close().await.unwrap();
    }
}
