use super::*;

impl Catalog {
    pub(crate) async fn check_export_destination(
        &self,
        destination: PathBuf,
    ) -> anyhow::Result<()> {
        let root = self.root.clone();
        let legacy = self.legacy_filename.clone();
        tokio::task::spawn_blocking(move || {
            anyhow::ensure!(destination.is_absolute(), "Choose an absolute export path");
            let target = destination.parent().context("Choose an export folder")?.canonicalize()?.join(destination.file_name().context("Choose an export filename")?);
            crate::profile_sync::paths::protect(&root,&target)?;
            anyhow::ensure!(!target.starts_with(root.join("profiles")) && !target.starts_with(root.join("bulk-locks")), "Choose an export destination outside Shep's profile and operation folders.");
            let check = |database: PathBuf| -> anyhow::Result<()> {
                for suffix in ["", "-wal", "-shm", "-journal"] {
                    let mut path = database.as_os_str().to_os_string();
                    path.push(suffix);
                    let path = PathBuf::from(path);
                    anyhow::ensure!(!crate::transfer::same_path(&path, &target) && !crate::transfer::same_file(&path, &target)?, "Choose an export destination outside Shep's profile catalog, caches and journal files.");
                }
                Ok(())
            };
            check(root.join(CATALOG_FILE))?;
            check(root.join(legacy))?;
            check(root.join("backup-uploads.sqlite"))?;
            let entries = match std::fs::read_dir(root.join("profiles")) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            for entry in entries {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    crate::profile_sync::paths::protect(&entry.path(),&target)?;
                    check(entry.path().join("shep.sqlite"))?;
                    check(entry.path().join("backup-uploads.sqlite"))?;
                }
            }
            Ok(())
        }).await?
    }
}
