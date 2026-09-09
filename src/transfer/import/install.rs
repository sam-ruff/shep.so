use super::*;
use crate::profiles::{Catalog, Id};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InstallPhase {
    #[default]
    Preparing,
    Publishing,
    Registering,
}

#[derive(Debug, Clone)]
pub struct Installed {
    pub id: Id,
    pub name: String,
    pub path: PathBuf,
    /// A published database must remain visible as saved even if a subsequent
    /// directory flush or catalog update fails. Recovery adopts that same file.
    pub warning: Option<String>,
    pub registered: bool,
}

pub struct Installation {
    cancel: watch::Sender<bool>,
    pub progress: watch::Receiver<InstallPhase>,
    result: Option<oneshot::Receiver<anyhow::Result<Option<Installed>>>>,
}
impl Installation {
    pub fn cancel(&self) {
        self.cancel.send_replace(true);
    }
    pub async fn finish(&mut self) -> anyhow::Result<Option<Installed>> {
        let result = self
            .result
            .as_mut()
            .context("This installation was already observed")?
            .await;
        self.result = None;
        result.context("Import stopped before acknowledging its result. Reopen Profiles to recover a completed copy.")?
    }
}

impl Prepared {
    /// Consumes the reviewed private copy. The original filename is never read
    /// again. This only adds a profile; opening it requires an explicit switch.
    pub fn install(
        self,
        catalog: Catalog,
        name: String,
        local: Preferences,
    ) -> anyhow::Result<Installation> {
        self.install_observed(catalog, name, local, |_| {})
    }

    fn install_observed(
        self,
        catalog: Catalog,
        name: String,
        local: Preferences,
        mut observe: impl FnMut(InstallPhase) + Send + 'static,
    ) -> anyhow::Result<Installation> {
        let name = crate::profiles::name_checked(name)?;
        let (cancel, cancellation) = watch::channel(false);
        let (updates, progress) = watch::channel(InstallPhase::Preparing);
        let (reply, result) = oneshot::channel();
        tokio::spawn(async move {
            let path = catalog.path(Id::Imported(self.id));
            let published = tokio::task::spawn_blocking(move || {
                publish(self, path, &name, &local, &cancellation, |phase| {
                    updates.send_replace(phase);
                    observe(phase);
                })
            })
            .await;
            let result = match published {
                Ok(Ok(Some(mut saved))) => {
                    // Publication is the commit boundary. Cancellation after
                    // it cannot remove the copy or turn success into Cancelled.
                    let id = match saved.id {
                        Id::Imported(id) => id,
                        Id::Legacy => unreachable!(),
                    };
                    let registration = async {
                        catalog.reserve(id, saved.name.clone()).await?;
                        catalog.finish(id).await
                    }
                    .await;
                    match registration {
                        Ok(profile) => {
                            saved.name = profile.name;
                            saved.registered = true;
                        }
                        Err(error) => add_warning(
                            &mut saved,
                            format!(
                                "The database was saved, but profile registration needs retry: {error:#}. Reopen Profiles to recover this copy."
                            ),
                        ),
                    }
                    Ok(Some(saved))
                }
                Ok(result) => result,
                Err(error) => Err(error.into()),
            };
            let _ = reply.send(result);
        });
        Ok(Installation {
            cancel,
            progress,
            result: Some(result),
        })
    }
}

fn add_warning(saved: &mut Installed, message: String) {
    match &mut saved.warning {
        Some(warning) => {
            warning.push(' ');
            warning.push_str(&message);
        }
        None => saved.warning = Some(message),
    }
}

// Removes only the empty directory this call created. After file publication,
// even an error or dropped observer must preserve the completed database.
struct EmptyDirectory(PathBuf);
impl Drop for EmptyDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn publish(
    prepared: Prepared,
    path: PathBuf,
    name: &str,
    local: &Preferences,
    cancel: &watch::Receiver<bool>,
    mut observe: impl FnMut(InstallPhase),
) -> anyhow::Result<Option<Installed>> {
    let result = (|| {
        check_cancel(cancel)?;
        fences::apply(prepared.path(), prepared.id, name, local, cancel)?;
        prepared.file.as_file().sync_all()?;
        let directory = path.parent().context("The profile has no directory")?;
        let parent = directory
            .parent()
            .context("The profile has no parent directory")?;
        std::fs::create_dir_all(parent)?;
        anyhow::ensure!(
            parent.symlink_metadata()?.file_type().is_dir(),
            "The profiles folder must be an ordinary directory"
        );
        std::fs::create_dir(directory).context("This profile destination already exists or cannot be created. The existing files were kept.")?;
        let _empty = EmptyDirectory(directory.into());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
        observe(InstallPhase::Publishing);
        check_cancel(cancel)?;
        // Same filesystem as the private copy; never overwrite an existing
        // database, and close SQLite before moving its main file on any OS.
        let committed = prepared
            .file
            .persist_noclobber(&path)
            .map_err(|error| error.error)?;
        drop(committed);
        let saved = Installed {
            id: Id::Imported(prepared.id),
            name: name.into(),
            path: path.clone(),
            registered: false,
            warning: None,
        };
        #[cfg(unix)]
        let saved = {
            let mut saved = saved;
            let flush = || -> std::io::Result<()> {
                std::fs::File::open(directory)?.sync_all()?;
                std::fs::File::open(parent)?.sync_all()
            };
            if let Err(error) = flush() {
                add_warning(
                    &mut saved,
                    format!(
                        "The database was saved, but its directory could not be flushed: {error}. Keep the original export until this profile reopens successfully."
                    ),
                );
            }
            saved
        };
        observe(InstallPhase::Registering);
        Ok(Some(saved))
    })();
    // An acknowledged publication always wins over a racing Cancel. Only a
    // precommit failure/cancellation may discard the private copy.
    if result.is_err() && cancelled(cancel) {
        Ok(None)
    } else {
        result
    }
}

#[cfg(test)]
mod tests;
