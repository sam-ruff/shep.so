use super::*;
use crate::{
    cache_cipher::bootstrap::{Bootstrap, Policy},
    profiles::{Catalog, Request, Session},
};

/// The data root and legacy cache filename for this launch, or none for the
/// memory-only demo workspace.
fn workspace_location(demo: bool) -> anyhow::Result<Option<(std::path::PathBuf, String)>> {
    if demo {
        #[cfg(feature = "test-support")]
        if let Some(path) = crate::test_support::workspace::path_from_arguments()? {
            let directory = path
                .parent()
                .context("The fixture needs a directory")?
                .to_owned();
            return Ok(Some((directory, "fixture.sqlite".into())));
        }
        return Ok(None);
    }
    let path = directories::ProjectDirs::from("so", "shep", "Shep")
        .context("Could not locate the app data directory")?
        .data_local_dir()
        .to_owned();
    Ok(Some((path, "shep.sqlite".into())))
}

pub(super) async fn open_workspace(demo: bool) -> anyhow::Result<(Store, Option<Session>)> {
    let catalog = match workspace_location(demo)? {
        Some((directory, legacy)) => {
            // Production never creates a key or converts a plaintext root: the
            // encrypted path is taken only for a root that already owns a key.
            let bootstrap = Bootstrap::start(Arc::new(crate::cache_cipher::key_store::Keys::new))?;
            let root = bootstrap
                .open(directory, legacy.clone(), Policy::Existing)
                .await?;
            Some(tokio::task::spawn_blocking(move || Catalog::open_in(&root, &legacy)).await??)
        }
        None => None,
    };
    match catalog {
        Some(catalog) => {
            let (store, session) = catalog.open_active(demo).await?;
            Ok((store, Some(session)))
        }
        None => Ok((tokio::task::spawn_blocking(Store::memory).await??, None)),
    }
}

impl Engine {
    pub(super) async fn profiles_command(
        &self,
        request: u64,
        action: Request,
        mut output: Output,
    ) -> anyhow::Result<()> {
        let result = async {
            let session = self
                .profiles
                .as_ref()
                .context("Profiles require a saved workspace")?;
            let mut warning = None;
            let offset = match action {
                Request::List { offset } => {
                    warning = match session.catalog.recover_imports().await {
                        Ok(recovered) => recovered.message(),
                        Err(error) => Some(format!(
                            "Could not check for completed imports: {error:#}. Refresh to retry."
                        )),
                    };
                    offset
                }
                Request::Rename {
                    id,
                    revision,
                    name,
                    offset,
                } => {
                    session.catalog.rename(id, revision, name).await?;
                    offset
                }
                Request::Activate {
                    id,
                    revision,
                    offset,
                } => {
                    session.catalog.activate(id, revision).await?;
                    offset
                }
            };
            let mut snapshot = session.snapshot(offset).await?;
            snapshot.warning = warning;
            Ok::<_, anyhow::Error>(Arc::new(snapshot))
        }
        .await;
        output
            .send(Event::Profiles(
                request,
                result.map_err(|error| format!("{error:#}")),
            ))
            .await?;
        Ok(())
    }
}
