//! Sign-in discovery never publishes local data. Only one completed profile in
//! an untouched workspace may be applied without the manual import review.
use super::{
    control::Control, drive::Session, enrollment::Snapshot, paths::Paths, setup::Discovery,
};
use crate::store::Store;

pub(crate) fn eligible(local: &Snapshot) -> bool {
    local.available
        && local.enrollment.selection.is_none()
        && local.enrollment.options.discover_on_login
}

pub(crate) fn automatic_candidate(review: &Discovery) -> bool {
    eligible(review.local())
        && review.local().empty_workspace
        && (review.local().enrollment.options.accounts
            || review.local().enrollment.options.settings)
        && review.profile_count() == 1
        && !review.has_more()
        && review.profiles().len() == 1
        && review.profiles().iter().all(|p| {
            p.initialized
                && !p.removed
                && p.waiting == 0
                && p.conflicts == 0
                && !p.name_conflict
                && p.name.is_some()
        })
}

pub(crate) async fn join(
    store: &Store,
    session: &Session,
    paths: &Paths,
    discovery: &Discovery,
    control: &Control,
) -> anyhow::Result<super::commands::Update> {
    anyhow::ensure!(
        automatic_candidate(discovery),
        "Choose and review a shared profile first."
    );
    let mut review = super::join::prepare(
        store,
        session,
        paths,
        paths.journal().await?,
        discovery,
        &discovery.profiles()[0].cursor(),
        control,
    )
    .await?;
    review.automatic = true;
    let (name, accounts, settings) = (review.name().to_owned(), review.accounts, review.settings);
    let snapshot = super::join::accept(store, paths, review, control).await?;
    Ok(super::commands::Update::AutoJoined {
        snapshot: std::sync::Arc::new(snapshot),
        name,
        accounts,
        settings,
    })
}
