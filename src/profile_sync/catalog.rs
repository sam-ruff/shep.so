//! Named remote discovery reuses the shared owning catalog and its change stream.
//! Its observations remain separate from this device's enrolled history/edits.
use super::{control::Control, drive::Session, enrollment::Snapshot, paths::Paths};
use crate::store::Store;
use anyhow::{Context, ensure};
use shep_profile_core::drive::catalog::{Discovery, Phase, Profile, Scope, State};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Review {
    path: PathBuf,
    key: Option<std::sync::Arc<crate::cache_cipher::Key>>,
    scope: Scope,
    revision: u64,
    pub(super) local: Snapshot,
    pub(super) files: u64,
    pub(super) total: u64,
    pub(super) profiles: Vec<Profile>,
    pub(super) after: Option<String>,
    pub(super) more: bool,
}
impl Review {
    fn check(&self, state: &State) -> anyhow::Result<()> {
        ensure!(
            state.revision == self.revision
                && state.phase == Phase::Complete
                && state.completed_revision == Some(state.revision)
                && state.error.is_none(),
            "Shared profiles changed. Discover them again before continuing."
        );
        Ok(())
    }
    pub(super) async fn validate(&self, session: &Session) -> anyhow::Result<()> {
        ensure!(
            self.scope.namespace == session.binding().namespace()
                && self.scope.principal == session.binding().identity(),
            "Google changed. Discover profiles again with the intended account."
        );
        let catalog = Discovery::open_with(
            self.path.clone(),
            self.scope.clone(),
            crate::cache_cipher::profile_connections(self.key.clone()),
        )
        .await?;
        let result = async { self.check(&catalog.state().await?) }.await;
        catalog
            .close()
            .await
            .context("Could not finish saving profile discovery")?;
        result
    }
    pub(crate) async fn page(&self, store: &Store, after: Option<String>) -> anyhow::Result<Self> {
        store.check_profile_review(self.local.clone()).await?;
        let catalog = Discovery::open_with(
            self.path.clone(),
            self.scope.clone(),
            crate::cache_cipher::profile_connections(self.key.clone()),
        )
        .await?;
        let result = async {
            self.check(&catalog.state().await?)?;
            let profiles = catalog.profiles(after.clone()).await?;
            let more = has_more(&catalog, &profiles).await?;
            store.check_profile_review(self.local.clone()).await?;
            Ok(Self {
                profiles,
                more,
                after,
                ..self.clone()
            })
        }
        .await;
        catalog
            .close()
            .await
            .context("Could not finish saving profile discovery")?;
        result
    }
}

pub(crate) async fn discover(
    store: &Store,
    session: &Session,
    paths: &Paths,
    control: &Control,
) -> anyhow::Result<Review> {
    let local = store.profile_enrollment().await?;
    ensure!(
        local.available && local.google_identity == session.binding().identity(),
        "Google changed. Reconnect and discover profiles again."
    );
    let drive = control.read(session.catalog_drive()).await?;
    let scope = Scope {
        namespace: drive.namespace().into(),
        principal: drive.principal().into(),
    };
    let path = paths.catalog(&scope)?;
    let catalog = Discovery::open_with(
        path.clone(),
        scope.clone(),
        crate::cache_cipher::profile_connections(paths.key.clone()),
    )
    .await?;
    let result = async {
        let mut state = catalog.state().await?;
        if state.error.is_some() {
            state = catalog.retry(state.revision).await?;
        } else if state.phase == Phase::Complete {
            state = catalog.refresh(state.revision, false).await?;
        }
        while state.phase != Phase::Complete {
            control.check()?;
            // The shared owner drains any SQL admitted before GET cancellation.
            state = control
                .read(async { Ok(catalog.advance(&drive).await?) })
                .await?;
        }
        control.check()?;
        store.check_profile_review(local.clone()).await?;
        let profiles = catalog.profiles(None).await?;
        let more = has_more(&catalog, &profiles).await?;
        let review = Review {
            path,
            key: paths.key.clone(),
            scope,
            revision: state.revision,
            local,
            files: state.files,
            total: state.profiles,
            profiles,
            after: None,
            more,
        };
        review.check(&state)?;
        Ok(review)
    }
    .await;
    catalog
        .close()
        .await
        .context("Could not finish saving profile discovery")?;
    result
}

async fn has_more(catalog: &Discovery, profiles: &[Profile]) -> anyhow::Result<bool> {
    Ok(profiles.len() == 50
        && !catalog
            .profiles(profiles.last().map(Profile::cursor))
            .await?
            .is_empty())
}
