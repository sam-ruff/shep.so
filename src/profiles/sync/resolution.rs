//! Checked, paged reviews of one preference. All history access runs under the
//! engine's existing owner; the mail Store retains reviews and exact decisions.
use super::*;
use crate::store::Store;
use anyhow::{Context, Result, ensure};
use shep_profile_core::history::{Command as HistoryCommand, Reply, Worker};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Review {
    pub id: Uuid,
    pub profile: String,
    pub key: SettingKey,
    pub device: Uuid,
    pub subscription_revision: u64,
    pub history_revision: u64,
    pub local_revision: u64,
    pub local: serde_json::Value,
    pub total: u64,
    pub seen: u64,
    pub phase: String,
    pub request: Option<PendingEdit>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {
    pub operation: Uuid,
    pub device: Uuid,
    pub change: Change,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Page {
    pub review: Option<Review>,
    pub versions: Vec<Version>,
    pub after: Option<Uuid>,
    pub more: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum Choice {
    Local,
    Version(Uuid),
}

async fn history(root: &Path, sub: &Subscription) -> Result<Worker> {
    let path = root
        .join("histories")
        .join(format!("{}.sqlite", sub.binding.storage_key()?));
    ensure!(
        tokio::fs::try_exists(&path).await?,
        "The enrolled history is missing. Recover it before resolving preferences."
    );
    let history = Worker::open(path, sub.binding.clone()).await?;
    let checked = async {
        let Reply::State(state) = history.request(HistoryCommand::State).await? else { anyhow::bail!("Could not inspect the profile history.") };
        ensure!(state.device == sub.device && state.revision >= sub.history_revision && state.initialized && !state.removed,
            "The profile history is incomplete, replaced or removed. Recover it before resolving preferences.");
        Ok(())
    }.await;
    if let Err(error) = checked {
        let _ = history.close().await;
        return Err(error);
    }
    Ok(history)
}

/// A failed old edit can be retired only after the same immutable request is
/// replayed. Changed/Conflict prove it was not accepted: History checks an
/// existing operation's exact request *before* those conditions.
pub async fn begin(store: &Store, root: &Path, profile: String, key: SettingKey) -> Result<Page> {
    let sub = store.profile_sync_subscription(profile.clone()).await?;
    let history = history(root, &sub).await?;
    let result = async {
        store.profile_resolution_can_begin(profile.clone()).await?;
        if let Some(edit) = store
            .profile_sync_pending_edit(profile.clone(), key)
            .await?
        {
            match history
                .request(HistoryCommand::Edit {
                    edit: edit.request.clone(),
                })
                .await
            {
                Ok(Reply::State(_)) => {
                    let revision = super::runner::receipt_revision(&history, &edit).await?;
                    store.profile_sync_edit_saved(edit, revision).await?;
                }
                Err(
                    shep_profile_core::history::Error::Changed
                    | shep_profile_core::history::Error::Conflict,
                ) => {
                    store.profile_sync_reject_edit(edit).await?;
                }
                Err(error) => return Err(error.into()),
                _ => anyhow::bail!("Could not inspect the pending preference change."),
            }
        }
        let Reply::State(state) = history.request(HistoryCommand::State).await? else {
            anyhow::bail!("Could not read the profile history.")
        };
        let id = store
            .profile_resolution_begin(profile.clone(), key, state.revision)
            .await?;
        let target =
            shep_profile_core::history::target(&shep_profile_core::Action::SettingRemoved { key });
        let mut after = None;
        loop {
            let Reply::Versions(versions) = history
                .request(HistoryCommand::Versions {
                    target: target.clone(),
                    after,
                })
                .await?
            else {
                anyhow::bail!("Could not inspect the preference versions.")
            };
            if versions.is_empty() {
                break;
            }
            let mut page = Vec::with_capacity(versions.len());
            for version in versions {
                let Reply::Value(change) = history
                    .request(HistoryCommand::Value {
                        target: target.clone(),
                        operation: version.operation,
                    })
                    .await?
                else {
                    anyhow::bail!("Could not inspect the preference value.")
                };
                page.push(Version {
                    operation: version.operation,
                    device: version.device,
                    change,
                });
            }
            after = page.last().map(|v| v.operation);
            store.profile_resolution_append(id, page).await?;
        }
        store.profile_resolution_ready(id).await?;
        store.profile_resolution_page(profile, id, None).await
    }
    .await;
    let closed = history.close().await;
    let page = result?;
    closed?;
    Ok(page)
}

/// Stage once in the mail DB, then replay exactly after storage/worker failure.
/// Choosing a version is explicit; uploads still require authenticated scanning.
pub async fn save(
    store: &Store,
    root: &Path,
    profile: String,
    id: Uuid,
    choice: Option<Choice>,
) -> Result<crate::store::PreferenceSnapshot> {
    let sub = store.profile_sync_subscription(profile.clone()).await?;
    let history = history(root, &sub).await?;
    let result = async {
        let review = store
            .profile_resolution_stage(profile.clone(), id, choice)
            .await?;
        let edit = review
            .request
            .clone()
            .context("This review has no saved decision. Reopen it before retrying.")?;
        match history
            .request(HistoryCommand::Edit {
                edit: edit.request.clone(),
            })
            .await
        {
            Ok(Reply::State(state)) => {
                let receipt = super::runner::receipt_revision(&history, &edit).await?;
                store
                    .profile_resolution_finish(profile, id, edit, receipt, state.revision)
                    .await
            }
            Err(
                error @ (shep_profile_core::history::Error::Changed
                | shep_profile_core::history::Error::Conflict),
            ) => {
                store
                    .profile_resolution_stale(profile, id, error.to_string())
                    .await?;
                Err(error.into())
            }
            Err(error) => Err(error.into()),
            _ => anyhow::bail!("Could not save the preference decision."),
        }
    }
    .await;
    let closed = history.close().await;
    let snapshot = result?;
    closed?;
    Ok(snapshot)
}

#[cfg(all(test, feature = "test-support"))]
mod tests;
