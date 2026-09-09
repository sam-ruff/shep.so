//! Reviewed desktop enrollment copies original records into an independent history.
mod transfer;
use crate::{
    model::Account,
    store::{Store, profile_enrollment as sql},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shep_profile_core::{
    drive::catalog::{Discovery, Scope, Snapshot},
    history::{Binding, Command as HistoryCommand, Reply, Worker},
};
use std::path::Path;
use uuid::Uuid;
fn changed() -> anyhow::Error {
    anyhow::anyhow!("Profile history changed. Reopen its review.")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Review {
    pub id: Uuid,
    pub binding: Binding,
    pub name: Option<String>,
    pub phase: String,
    pub copied: u64,
    pub total: u64,
    pub cursor: u64,
    pub history_revision: u64,
    pub field_after: Option<String>,
    pub rows: u64,
    pub applied: u64,
    pub kept: u64,
    pub include_accounts: bool,
    pub include_settings: bool,
    pub baseline: super::preference_state::PreferenceState,
    pub settings_receipt: Option<Value>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Row {
    pub position: u64,
    pub target: String,
    pub kind: String,
    pub account: Option<Account>,
    pub local: Option<Account>,
    pub value: Value,
    pub reason: Option<String>,
    pub available: bool,
    // Frozen opaque local IDs, never exported as a shared identity.
    pub local_id: Option<String>,
    pub existing_id: Option<String>,
    pub removal_epoch: u64,
    pub mapping: Option<String>,
    pub new_account: bool,
    #[serde(default)]
    pub selected: bool,
    #[serde(default)]
    pub receipt: Option<String>,
    pub shared: Option<Uuid>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Observation {
    pub next_id: Option<Uuid>,
    pub review: Option<Review>,
    pub rows: Vec<Row>,
    pub after: u64,
    // Device settings and connection snapshots never enter the portable observation.
    #[serde(skip)]
    pub local: Option<Local>,
}
#[derive(Clone, Debug)]
pub struct Local {
    pub preferences: crate::store::PreferenceSnapshot,
    pub accounts: Vec<Account>,
    pub connections_revision: u64,
    pub reconnect: std::collections::HashSet<String>,
}
#[derive(Clone, Debug)]
pub enum Command {
    Current,
    Rows {
        id: Uuid,
        after: u64,
    },
    Prepare {
        id: Uuid,
        profile: Uuid,
        generation: Uuid,
        revision: u64,
    },
    Step {
        id: Uuid,
    },
    Choose {
        id: Uuid,
        position: u64,
        selected: bool,
    },
    Approve {
        id: Uuid,
        accounts: bool,
        settings: bool,
    },
    Cancel {
        id: Uuid,
    },
}
async fn source(db: &Store, key: &str, id: Uuid, catalog: &Discovery) -> Result<Snapshot> {
    let key = key.to_owned();
    let original = db.run(move |db| sql::source(db, &key, id)).await?;
    let current = catalog
        .snapshot(
            original.profile.profile,
            original.profile.generation,
            original.profile.revision,
        )
        .await?;
    ensure!(
        current.binding == original.binding && current.profile == original.profile,
        "This profile changed. Cancel the review and prepare it again."
    );
    Ok(original)
}
pub(crate) async fn run(
    db: &Store,
    scope: Scope,
    root: &Path,
    catalog: &Discovery,
    command: Command,
) -> Result<Option<Review>> {
    let key = scope.storage_key()?;
    Ok(match command {
        Command::Current => db.run(move |db| sql::current(db, &key)).await?,
        Command::Rows { id, .. } => Some(db.run(move |db| sql::read(db, &key, id)).await?),
        Command::Prepare {
            id,
            profile,
            generation,
            revision,
        } => {
            let snapshot = catalog.snapshot(profile, generation, revision).await?;
            Some(
                db.run(move |db| sql::prepare(db, &key, id, snapshot))
                    .await?,
            )
        }
        Command::Step { id } => {
            let result = step(db, &key, id, root, catalog).await;
            if let Err(error) = &result {
                let key = key.clone();
                let message = error.to_string();
                db.run(move |db| {
                    let mut review = sql::read(db, &key, id)?;
                    review.error = Some(message);
                    sql::write(db, &review)
                })
                .await?;
            }
            Some(result?)
        }
        Command::Choose {
            id,
            position,
            selected,
        } => Some(
            db.run(move |db| sql::choose(db, &key, id, position, selected))
                .await?,
        ),
        Command::Approve {
            id,
            accounts,
            settings,
        } => {
            source(db, &key, id, catalog).await?;
            let lookup = key.clone();
            let review = db.run(move |db| sql::read(db, &lookup, id)).await?;
            if review.phase == "review" {
                let history = Worker::open(
                    root.join(format!("{}.sqlite", review.binding.storage_key()?)),
                    review.binding.clone(),
                )
                .await?;
                let state = history.request(HistoryCommand::State).await;
                let closed = history.close().await;
                let Reply::State(state) = state? else {
                    return Err(changed());
                };
                closed?;
                ensure!(
                    state.revision == review.history_revision
                        && state.initialized
                        && !state.removed,
                    "Local profile history changed. Cancel this review and prepare it again."
                );
            }
            Some(
                db.run(move |db| sql::approve(db, &key, id, accounts, settings))
                    .await?,
            )
        }
        Command::Cancel { id } => Some(db.run(move |db| sql::cancel(db, &key, id)).await?),
    })
}
async fn step(db: &Store, key: &str, id: Uuid, root: &Path, catalog: &Discovery) -> Result<Review> {
    let lookup = key.to_owned();
    let review = db.run(move |db| sql::read(db, &lookup, id)).await?;
    if matches!(
        review.phase.as_str(),
        "copying" | "draining" | "fields" | "planning"
    ) {
        let snapshot = source(db, key, id, catalog).await?;
        let history = Worker::open(
            root.join(format!("{}.sqlite", review.binding.storage_key()?)),
            review.binding.clone(),
        )
        .await?;
        let result = transfer::step(db, key, review, snapshot, catalog, &history).await;
        let closed = history.close().await;
        let result = result?;
        closed?;
        return Ok(result);
    }
    let key = key.to_owned();
    db.run(move |db| sql::apply_step(db, &key, id)).await
}

#[cfg(all(test, feature = "test-support"))]
mod tests;
