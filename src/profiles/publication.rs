//! Desktop publication keeps its review in the mail owner's database and exact
//! operations in an independently owned history. No account or credential writes.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shep_profile_core::{Action, Change, Operation, SettingKey, history::Binding};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Observation {
    pub next_id: Option<Uuid>,
    pub review: Option<Review>,
    pub rows: Vec<AccountRow>,
    pub after: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Specification {
    pub name: String,
    pub include_accounts: bool,
    pub settings: BTreeMap<SettingKey, Value>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Preparing,
    Review,
    Staging,
    Uploading,
    Complete,
    Cancelled,
}
impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Review => "review",
            Self::Staging => "staging",
            Self::Uploading => "uploading",
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Review {
    pub id: Uuid,
    pub binding: Binding,
    pub name: String,
    pub accounts: u64,
    pub prepared: u64,
    pub settings: BTreeMap<SettingKey, Value>,
    #[serde(default)]
    pub preference_revisions: BTreeMap<SettingKey, u64>,
    pub total: u64,
    pub staged: u64,
    pub uploaded: u64,
    pub phase: Phase,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct AccountRow {
    pub position: u64,
    pub account: crate::model::Account,
}
#[derive(Clone, Debug)]
pub enum Command {
    Current,
    Prepare {
        id: Uuid,
        specification: Specification,
    },
    PrepareStep {
        id: Uuid,
    },
    Accounts {
        id: Uuid,
        after: u64,
    },
    Approve {
        id: Uuid,
        settings: BTreeMap<SettingKey, Value>,
    },
    Step {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
    },
}
impl Command {
    pub fn needs_discovery(&self) -> bool {
        !matches!(
            self,
            Self::Current | Self::Accounts { .. } | Self::Cancel { .. }
        )
    }
}
pub(crate) fn change(action: Action) -> Change {
    Change {
        action,
        extra: Default::default(),
    }
}
pub(crate) fn validate(spec: &Specification) -> Result<Vec<Change>> {
    ensure!(
        spec.settings
            .keys()
            .all(|key| super::preferences::SUPPORTED.contains(key)),
        "This preference is not available for desktop publication yet."
    );
    let mut changes = vec![change(Action::ProfileName {
        name: spec.name.clone(),
    })];
    changes.extend(spec.settings.iter().map(|(key, value)| {
        change(Action::Setting {
            key: *key,
            value: value.clone(),
        })
    }));
    Operation {
        format: shep_profile_core::FORMAT.into(),
        major: 1,
        minor: 0,
        requires: vec!["causal-v1".into(), "settings-v1".into()],
        namespace: "so.shep.validation".into(),
        profile: Uuid::from_u128(1),
        generation: Uuid::from_u128(2),
        device: Uuid::from_u128(3),
        operation: Uuid::from_u128(4),
        parents: vec![],
        changes: changes.clone(),
        extra: Default::default(),
    }
    .encode()?;
    Ok(changes)
}
pub(crate) fn integer(value: u64) -> Result<i64> {
    value
        .try_into()
        .context("Profile cursor is invalid. Reopen the review.")
}

pub(crate) async fn run(
    store: &crate::store::Store,
    scope: shep_profile_core::drive::catalog::Scope,
    root: &std::path::Path,
    catalog: &shep_profile_core::drive::catalog::Discovery,
    drive: Option<&shep_profile_core::drive::Drive>,
    command: Command,
) -> Result<Option<Review>> {
    let key = scope.storage_key()?;
    if command.needs_discovery() {
        let state = catalog.state().await?;
        ensure!(
            state.phase == shep_profile_core::drive::catalog::Phase::Complete
                && state.error.is_none(),
            "Finish or retry discovery before publishing a profile."
        );
    }
    Ok(match command {
        Command::Current => store.publication_current(key).await?,
        Command::Prepare { id, specification } => {
            Some(store.publication_prepare(scope, id, specification).await?)
        }
        Command::PrepareStep { id } => Some(store.publication_prepare_step(key, id).await?),
        Command::Accounts { id, .. } => Some(store.publication_review(key, id).await?),
        Command::Approve { id, settings } => {
            Some(store.publication_approve(key, id, settings).await?)
        }
        Command::Cancel { id } => Some(store.publication_cancel(key, id).await?),
        Command::Step { id } => Some(step(store, key, id, root, catalog, drive).await?),
    })
}
async fn step(
    store: &crate::store::Store,
    scope: String,
    id: Uuid,
    root: &std::path::Path,
    catalog: &shep_profile_core::drive::catalog::Discovery,
    drive: Option<&shep_profile_core::drive::Drive>,
) -> Result<Review> {
    let review = store.publication_review(scope.clone(), id).await?;
    ensure!(
        matches!(
            review.phase,
            Phase::Staging | Phase::Uploading | Phase::Complete
        ),
        "Review this profile before publishing it."
    );
    if review.phase == Phase::Complete {
        return Ok(review);
    }
    let worker = shep_profile_core::history::Worker::open(
        root.join(format!("{}.sqlite", review.binding.storage_key()?)),
        review.binding.clone(),
    )
    .await?;
    let result = match review.phase {
        Phase::Staging => stage(store, &scope, &review, &worker).await,
        _ => match drive {
            Some(drive) => upload(store, &scope, &review, &worker, catalog, drive).await,
            None => Err(anyhow::anyhow!(
                "Reconnect Google before continuing this publication."
            )),
        },
    };
    let closed = worker.close().await;
    let result = result.and_then(|review| {
        closed?;
        Ok(review)
    });
    if let Err(error) = &result {
        let message = error.to_string();
        store
            .run(move |db| {
                let tx = db.transaction()?;
                let mut current = crate::store::profiles::read(&tx, &scope, id)?;
                current.error = Some(message);
                crate::store::profiles::write(&tx, &current)?;
                tx.commit()?;
                Ok(())
            })
            .await?;
    }
    result
}
async fn stage(
    store: &crate::store::Store,
    scope: &str,
    review: &Review,
    worker: &shep_profile_core::history::Worker,
) -> Result<Review> {
    use rusqlite::params;
    use shep_profile_core::history::{Command as HistoryCommand, LocalEdit, Reply};
    let id = review.id;
    let position = review.staged;
    let (operation,changes,request)=store.run(move |db| Ok(db.query_row(
        "SELECT operation,changes,request FROM profile_publication_rows WHERE publication=? AND position=?",
        params![id.to_string(),integer(position)?],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?)))?)).await?;
    let request = if let Some(saved) = request {
        saved
    } else {
        let Reply::State(state) = worker.request(HistoryCommand::State).await? else {
            anyhow::bail!("Reopen the saved publication before continuing.");
        };
        let request = serde_json::to_string(&LocalEdit {
            operation: Uuid::parse_str(&operation)?,
            expected_revision: state.revision,
            changes: serde_json::from_str(&changes)?,
            resolutions: vec![],
        })?;
        let saved = request.clone();
        store.run(move |db| {
            db.execute("UPDATE profile_publication_rows SET request=? WHERE publication=? AND position=? AND request IS NULL",params![saved,id.to_string(),integer(position)?])?;
            Ok(())
        }).await?;
        request
    };
    // The exact request is durable before crossing into another database. A lost
    // observer or receipt retries the same operation and expected revision.
    worker
        .request(HistoryCommand::Edit {
            edit: serde_json::from_str(&request)?,
        })
        .await?;
    let scope = scope.to_owned();
    store
        .run(move |db| {
            let tx = db.transaction()?;
            let mut current = crate::store::profiles::read(&tx, &scope, id)?;
            ensure!(
                current.phase == Phase::Staging && current.staged == position,
                "Publication changed. Reopen its saved progress."
            );
            current.staged += 1;
            current.error = None;
            if current.staged == current.total {
                current.phase = Phase::Uploading;
            }
            crate::store::profiles::write(&tx, &current)?;
            tx.commit()?;
            Ok(current)
        })
        .await
}
async fn upload(
    store: &crate::store::Store,
    scope: &str,
    review: &Review,
    worker: &shep_profile_core::history::Worker,
    catalog: &shep_profile_core::drive::catalog::Discovery,
    drive: &shep_profile_core::drive::Drive,
) -> Result<Review> {
    use shep_profile_core::history::{Command as HistoryCommand, Reply};
    drive.upload_next_tracked(worker, catalog).await?;
    let Reply::State(state) = worker.request(HistoryCommand::State).await? else {
        anyhow::bail!("Reopen the saved publication before continuing.");
    };
    ensure!(
        state.initialized && state.operations == review.total && state.queued <= review.total,
        "The prepared profile is incomplete. Keep its publication and retry."
    );
    let scope = scope.to_owned();
    let id = review.id;
    store
        .run(move |db| {
            let tx = db.transaction()?;
            let mut current = crate::store::profiles::read(&tx, &scope, id)?;
            ensure!(
                current.phase == Phase::Uploading,
                "Publication changed. Reopen its saved progress."
            );
            current.uploaded = current.total - state.queued;
            current.error = None;
            if state.queued == 0 {
                current.phase = Phase::Complete;
            }
            crate::store::profiles::write(&tx, &current)?;
            tx.commit()?;
            Ok(current)
        })
        .await
}

#[cfg(test)]
mod tests;
