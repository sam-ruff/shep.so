//! First-device setup. Own each async pass until upload receipts are durable;
//! cancellation of a UI observer is not cancellation of an admitted write.
use super::{drive, enrollment::*, journal, replica::*, *};
use crate::store::Store;
use shep_profile_core::history;

/// A complete discovery review, scoped to the verified Google identity and the
/// local setup as it stood before listing. Private fields prevent UI callers
/// manufacturing an empty result or replacing the reviewed local revision.
#[derive(Clone, Debug)]
pub struct Discovery {
    evidence: Evidence,
    local: Snapshot,
}
#[derive(Clone, Debug)]
enum Evidence {
    Files(journal::Scan),
    Catalog(Box<super::catalog::Review>),
}
impl Discovery {
    pub fn records(&self) -> u64 {
        match &self.evidence {
            Evidence::Files(scan) => scan.files(),
            Evidence::Catalog(review) => review.files,
        }
    }
    pub(crate) fn from_catalog(review: super::catalog::Review) -> Self {
        Self {
            local: review.local.clone(),
            evidence: Evidence::Catalog(Box::new(review)),
        }
    }
    pub fn profiles(&self) -> &[shep_profile_core::drive::catalog::Profile] {
        match &self.evidence {
            Evidence::Catalog(review) => &review.profiles,
            _ => &[],
        }
    }
    pub fn profile_count(&self) -> u64 {
        match &self.evidence {
            Evidence::Catalog(review) => review.total,
            _ => 0,
        }
    }
    pub fn has_more(&self) -> bool {
        matches!(&self.evidence,Evidence::Catalog(review) if review.more)
    }
    pub fn after(&self) -> Option<&str> {
        match &self.evidence {
            Evidence::Catalog(review) => review.after.as_deref(),
            _ => None,
        }
    }
    pub(crate) async fn page(&self, store: &Store, after: Option<String>) -> anyhow::Result<Self> {
        let Evidence::Catalog(review) = &self.evidence else {
            anyhow::bail!("Discover named profiles first.")
        };
        Ok(Self::from_catalog(review.page(store, after).await?))
    }
    pub(crate) async fn validate(
        &self,
        session: &drive::Session,
        journal: &journal::Journal,
    ) -> anyhow::Result<()> {
        match &self.evidence {
            Evidence::Files(scan) => {
                anyhow::ensure!(
                    scan.binding() == session.binding() && scan.profile().is_none(),
                    "Google changed after discovery. Review the current account first."
                );
                journal.scan_entries(scan, None).await?;
                Ok(())
            }
            Evidence::Catalog(review) => review.validate(session).await,
        }
    }
    pub fn local(&self) -> &Snapshot {
        &self.local
    }
}

pub async fn discover(
    store: &Store,
    session: &drive::Session,
    journal: &journal::Journal,
) -> anyhow::Result<Discovery> {
    discover_controlled(store, session, journal, &control::Control::default()).await
}
pub async fn discover_controlled(
    store: &Store,
    session: &drive::Session,
    journal: &journal::Journal,
    control: &control::Control,
) -> anyhow::Result<Discovery> {
    control.check()?;
    let local = store.profile_enrollment().await?;
    anyhow::ensure!(
        local.available && local.google_identity == session.binding().identity(),
        "Google changed. Reconnect and discover profiles again."
    );
    let mut scan = match journal.resume_scan(session.binding(), None).await? {
        Some(scan) if !scan.complete() => scan,
        _ => journal.begin_scan(session.binding().clone(), None).await?,
    };
    while !scan.complete() {
        let page = control.read(session.page_for(&scan)).await?;
        scan = journal.append_page(&scan, page).await?;
    }
    // Validate local intent again before presenting the completed review.
    store.check_profile_review(local.clone()).await?;
    Ok(Discovery {
        evidence: Evidence::Files(scan),
        local,
    })
}

/// Only an explicit Create review may choose fresh shared identities. This is
/// separate from resume: an existing or missing remote generation cannot become
/// an implicit replacement. The store commits all seed IDs and values together.
pub async fn create(
    store: &Store,
    session: &drive::Session,
    journal: &journal::Journal,
    reviewed: Discovery,
    name: String,
    options: Options,
) -> anyhow::Result<Snapshot> {
    reviewed.validate(session, journal).await?;
    store
        .begin_profile_enrollment(
            reviewed.local,
            Selection {
                binding: history::Binding {
                    namespace: session.binding().namespace().into(),
                    principal: session.binding().identity().into(),
                    profile: Uuid::new_v4(),
                    generation: Uuid::new_v4(),
                },
                name,
                origin: Origin::Create,
                ready: false,
            },
            options,
        )
        .await
}

/// Materialize the exact saved seed into the owning shared history worker.
/// Persist each edit's expected revision before admission so lost acknowledgments
/// and process restarts replay the identical request, including across chunks.
pub async fn prepare(
    store: &Store,
    replica: &mut Replica,
    expected: &Snapshot,
) -> anyhow::Result<()> {
    prepare_controlled(store, replica, expected, &control::Control::default()).await
}
async fn prepare_controlled(
    store: &Store,
    replica: &mut Replica,
    expected: &Snapshot,
    control: &control::Control,
) -> anyhow::Result<()> {
    control.check()?;
    let selection = expected
        .enrollment
        .selection
        .as_ref()
        .context("Choose a shared profile first.")?;
    anyhow::ensure!(
        &selection.binding == replica.binding(),
        "The saved setup belongs to another history."
    );
    let seed = store.profile_seed(expected.clone()).await?;
    seed.validate(selection)?;
    for chunk in &seed.chunks {
        check_categories(expected.enrollment.options, chunk)?;
    }
    for chunk in seed.chunks {
        control.check()?;
        let revision = replica.state().await?.revision;
        let chunk = store
            .checkpoint_profile_seed(expected.clone(), chunk.operation, revision)
            .await?;
        replica
            .edit(history::LocalEdit {
                operation: chunk.operation,
                expected_revision: chunk
                    .expected_revision
                    .context("The setup edit was not checkpointed")?,
                changes: chunk.changes,
                resolutions: vec![],
            })
            .await?;
    }
    Ok(())
}

fn check_categories(options: Options, chunk: &SeedChunk) -> anyhow::Result<()> {
    use shep_profile_core::Action;
    anyhow::ensure!(
        options.enabled,
        "Profile sync was turned off. The saved setup is kept."
    );
    for change in &chunk.changes {
        let allowed = match &change.action {
            Action::AccountConnection { .. } | Action::AccountName { .. } => options.accounts,
            Action::Setting { .. } => options.settings,
            _ => true,
        };
        anyhow::ensure!(
            allowed,
            "A saved setup category was turned off. Re-enable its original categories to resume this pending copy."
        );
    }
    Ok(())
}

/// Finish first publication. Every write is preceded by a local intent check;
/// the in-progress write is still acknowledged before a subsequent check fails.
/// A newer local edit is never applied back from the frozen first-device seed.
pub async fn publish(
    store: &Store,
    replica: &mut Replica,
    session: &drive::Session,
    expected: Snapshot,
    when: i64,
) -> anyhow::Result<Snapshot> {
    publish_controlled(
        store,
        replica,
        session,
        expected,
        when,
        &control::Control::default(),
    )
    .await
}
pub async fn publish_controlled(
    store: &Store,
    replica: &mut Replica,
    session: &drive::Session,
    expected: Snapshot,
    when: i64,
    control: &control::Control,
) -> anyhow::Result<Snapshot> {
    prepare_controlled(store, replica, &expected, control).await?;
    let mut pulled = replica.pull_controlled(session, control).await?;
    anyhow::ensure!(
        !pulled.state().removed && pulled.state().conflicts == 0,
        "This profile changed on another device. Review its changes before completing setup."
    );
    let mut acknowledged = false;
    let result = async {
        loop {
            control.check()?;
            store.check_profile_review(expected.clone()).await?;
            // publish_next includes reserved-ID recovery and both durable receipts.
            if !replica
                .publish_next(session, &mut pulled, PublishIntent::ReviewedNewProfile)
                .await?
            {
                break;
            }
            acknowledged = true;
        }
        store.profile_sync_succeeded(expected, when).await
    }
    .await;
    if acknowledged {
        result.context("A profile upload was saved to Drive. Setup remains pending; keep its saved records and resume after reviewing the current choices")
    } else {
        result
    }
}

#[cfg(test)]
mod tests;
