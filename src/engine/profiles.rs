use super::*;
use crate::profiles::{Catalog, Request, Session};

pub(super) async fn open_workspace(demo: bool) -> anyhow::Result<(Store, Option<Session>)> {
    let catalog = tokio::task::spawn_blocking(move || -> anyhow::Result<Option<Catalog>> {
        if demo {
            #[cfg(feature = "test-support")]
            if let Some(path) = crate::test_support::workspace::path_from_arguments()? {
                return Ok(Some(Catalog::open(
                    path.parent().context("The fixture needs a directory")?,
                    "fixture.sqlite",
                )?));
            }
            Ok(None)
        } else {
            let path = directories::ProjectDirs::from("so", "shep", "Shep")
                .context("Could not locate the app data directory")?
                .data_local_dir()
                .to_owned();
            Ok(Some(Catalog::open(&path, "shep.sqlite")?))
        }
    })
    .await??;
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
