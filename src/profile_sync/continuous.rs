//! One cancellable continuous pass. Local intent is admitted before pulling;
//! only the exclusive history owner can attest values for cache application.
use super::{
    control::Control,
    drive::Session,
    enrollment::Options,
    incremental,
    replica::{PublishIntent, Pulled, Replica},
    state,
};
use crate::store::Store;
use shep_profile_core::{Change, history};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default)]
pub struct Report {
    pub applied: usize,
    pub review: usize,
    pub published: usize,
    pub remaining: bool,
    /// The password vault pass that followed, when password sync is involved.
    pub passwords: Option<super::vault::Report>,
}
pub(crate) struct Observed {
    pub(super) binding: history::Binding,
    pub(super) revision: u64,
    pub(super) fields: Vec<(history::Field, Option<Change>)>,
}
impl Observed {
    fn cursor(&self) -> Option<String> {
        self.fields.last().map(|(field, _)| field.target.clone())
    }
    pub(crate) fn into_parts(
        self,
    ) -> (history::Binding, u64, Vec<(history::Field, Option<Change>)>) {
        (self.binding, self.revision, self.fields)
    }
}

pub(crate) async fn run(
    store: &Store,
    replica: &mut Replica,
    session: &Session,
    catalog: &incremental::Location,
    control: &Control,
) -> anyhow::Result<Report> {
    let mut report = Report::default();
    let excluded = admit(store, replica, control).await?;
    if store
        .capture_profile_change_except(excluded.clone())
        .await?
        .is_some()
    {
        report.remaining = true;
        return Ok(report);
    }
    let mut pulled = replica.pull_catalog(session, catalog, control).await?;
    reconcile(store, replica, session, control, &mut pulled, &mut report).await?;
    report.remaining = replica.state().await?.queued != 0;
    report.review = report.review.max(excluded.len());
    Ok(report)
}

async fn admit(
    store: &Store,
    replica: &mut Replica,
    control: &Control,
) -> anyhow::Result<BTreeSet<String>> {
    let mut excluded = BTreeSet::new();
    // Yield after bounded admission rather than holding the owner indefinitely
    // when another source is continuously editing. The next pass resumes it.
    for _ in 0..32 {
        control.check()?;
        let Some(pending) = store
            .capture_profile_change_except(excluded.clone())
            .await?
        else {
            break;
        };
        match replica.admit_local(pending.clone()).await {
            Ok(receipt) => store.acknowledge_profile_change(receipt).await?,
            Err(error)
                if matches!(
                    error.downcast_ref::<history::Error>(),
                    Some(
                        history::Error::Changed
                            | history::Error::Conflict
                            | history::Error::Removed
                    )
                ) =>
            {
                excluded.insert(pending.target());
                store
                    .defer_profile_change(replica.binding().clone(), pending)
                    .await?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(excluded)
}

async fn reconcile(
    store: &Store,
    replica: &mut Replica,
    session: &Session,
    control: &Control,
    pulled: &mut Pulled,
    report: &mut Report,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        pulled.remote_records() > 0,
        "The shared profile is missing from Drive. Local accounts and changes have been kept; check the connected Google account."
    );
    anyhow::ensure!(
        pulled.state().initialized && !pulled.state().removed,
        "This shared profile is incomplete or removed. Local accounts have been kept for review."
    );
    let mut after = None;
    loop {
        control.check()?;
        let observed = replica.observe(after).await?;
        after = observed.cursor();
        if after.is_none() {
            break;
        }
        let result = store.apply_profile_observation(observed).await?;
        report.applied += result.applied;
        report.review += result.review;
    }
    for _ in 0..32 {
        control.check()?;
        let snapshot = store.profile_enrollment().await?;
        let options = snapshot.enrollment.options;
        if !options.enabled || !replica.next_allowed(options).await? {
            break;
        }
        if !replica
            .publish_next(session, pulled, PublishIntent::ExistingProfile)
            .await?
        {
            break;
        }
        report.published += 1;
    }
    Ok(())
}

impl Replica {
    pub(crate) async fn next_allowed(&self, options: Options) -> anyhow::Result<bool> {
        let Some(changes) = self.next_upload_changes().await? else {
            return Ok(false);
        };
        Ok(changes.iter().all(|c| state::allowed(c, options)))
    }
}
