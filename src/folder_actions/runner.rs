//! Serial execution under the account coordinator. Provider timeouts live in
//! the adapter; a database commit is always observed through completion.
use super::*;
use crate::store::{FolderLease, Store};
use std::sync::Arc;

pub async fn run(
    store: &Store,
    lease: &FolderLease,
    mut connection: Option<&mut dyn Connection>,
    stopping: &crate::lifecycle::Signal,
    progress: Option<&tokio::sync::watch::Sender<Arc<Job>>>,
) -> anyhow::Result<Job> {
    let mut current = store.recover_folder_change(lease).await?;
    anyhow::ensure!(
        !current.review.imap || connection.is_some(),
        "Connect to the IMAP server before applying this folder change."
    );
    // A prior acknowledgment is cache work, never an instruction to repeat a
    // RENAME/DELETE. An unconfirmed write stays available for explicit review.
    for position in current
        .steps
        .iter()
        .filter(|step| step.status == Status::Acknowledged)
        .map(|step| step.position)
        .collect::<Vec<_>>()
    {
        if matches!(current.steps[position].step, Step::Rename { .. })
            && let Some(connection) = connection.as_deref_mut()
        {
            current.confirm_renamed_catalog(&connection.catalog().await?)?;
        }
        current = store.commit_folder_step(lease, position).await?;
        publish(progress, &current);
    }
    loop {
        if current.closed
            || stopping.get()
            || current
                .steps
                .iter()
                .any(|step| matches!(step.status, Status::Uncertain | Status::Rejected))
        {
            return Ok(current);
        }
        let catalog = if let Some(connection) = connection.as_deref_mut() {
            connection.catalog().await?
        } else {
            store
                .current_folder_catalog(current.review.account.clone())
                .await?
        };
        let absent = current.preflight(&catalog)?;
        let Some(step) = store.claim_folder_step(lease).await? else {
            return Ok(current);
        };
        current.steps[step.position].status = Status::Running;
        publish(progress, &current);
        let local = matches!(step.step, Step::Forget { .. })
            || matches!(&step.step, Step::Delete { source } if absent.contains(source));
        let outcome = if !local && let Some(connection) = connection.as_deref_mut() {
            connection.apply(&step.step).await
        } else {
            Outcome::Applied
        };
        current = store
            .record_folder_outcome(lease, step.position, outcome)
            .await?;
        publish(progress, &current);
        if current.steps[step.position].status == Status::Acknowledged {
            if matches!(step.step, Step::Rename { .. })
                && let Some(connection) = connection.as_deref_mut()
            {
                current.confirm_renamed_catalog(&connection.catalog().await?)?;
            }
            current = store.commit_folder_step(lease, step.position).await?;
            publish(progress, &current);
        }
    }
}

fn publish(progress: Option<&tokio::sync::watch::Sender<Arc<Job>>>, job: &Job) {
    if let Some(progress) = progress {
        progress.send_replace(Arc::new(job.clone()));
    }
}
