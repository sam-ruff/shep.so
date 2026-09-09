//! Reviewed device enrollment. Remote history is copied, never a device database.
mod apply;
mod store;
mod transfer;
use super::changed;
use crate::{api::MobileProfile, database::Database};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shep_mail_core::model::Account;
use shep_profile_core::{
    Action, Change,
    drive::catalog::{Discovery, Scope, Snapshot},
    history::{Binding, Command as HistoryCommand, Reply, Worker},
};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Preferences {
    pub values: BTreeMap<String, Value>,
    pub revisions: BTreeMap<String, u64>,
}
const SETTINGS: [&str; 8] = [
    "appearance",
    "left_swipe",
    "right_swipe",
    "preview_lines",
    "sender_pictures",
    "unified_inbox",
    "reply_display",
    "tooltips",
];
impl Preferences {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.values.len() == SETTINGS.len() && self.revisions.len() == SETTINGS.len(),
            "Refresh the device preferences before reviewing this profile."
        );
        for key in SETTINGS {
            ensure!(
                self.values.contains_key(key)
                    && self
                        .revisions
                        .get(key)
                        .is_some_and(|v| *v <= 9_007_199_254_740_991),
                "Invalid device preference revision. Reopen the profile."
            );
            // The wire codec owns validation of these portable values.
            let setting = serde_json::from_value(serde_json::json!(key))?;
            let operation = shep_profile_core::Operation {
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
                extra: Default::default(),
                changes: vec![Change {
                    action: Action::Setting {
                        key: setting,
                        value: self.values[key].clone(),
                    },
                    extra: Default::default(),
                }],
            };
            operation.encode()?;
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Review {
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
    pub baseline: Preferences,
    pub settings_receipt: Option<Value>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Row {
    position: u64,
    target: String,
    kind: String,
    account: Option<Account>,
    local: Option<Account>,
    value: Value,
    reason: Option<String>,
    available: bool,
    // Frozen opaque local IDs, never exported as a shared identity.
    local_id: Option<String>,
    slot: Option<String>,
    shared: Option<Uuid>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
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
        preferences: Preferences,
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
    Settings {
        id: Uuid,
    },
    ConfirmSettings {
        id: Uuid,
        applied: Vec<String>,
        kept: Vec<String>,
    },
    Cancel {
        id: Uuid,
    },
}
impl Command {
    pub fn reads(&self) -> bool {
        matches!(
            self,
            Self::Current | Self::Rows { .. } | Self::Settings { .. }
        )
    }
}
fn read(db: &Connection, key: &str, id: Uuid) -> Result<Review> {
    let raw: String = db.query_row("SELECT review FROM profile_enrollments WHERE id=? AND scope=?", params![id.to_string(), key], |r| r.get(0)).optional()?.context("This enrollment belongs to another Google account or is no longer available. Reopen Profiles and sync.")?;
    Ok(serde_json::from_str(&raw)?)
}
fn write(db: &Connection, review: &Review) -> Result<()> {
    db.execute(
        "UPDATE profile_enrollments SET phase=?,review=? WHERE id=?",
        params![
            review.phase,
            serde_json::to_string(review)?,
            review.id.to_string()
        ],
    )?;
    Ok(())
}
fn integer(value: u64) -> Result<i64> {
    value
        .try_into()
        .context("Invalid enrollment cursor. Reopen the review.")
}
async fn worker(db: &Database, binding: Binding) -> Result<Worker> {
    let mut directory = db.path.as_os_str().to_owned();
    directory.push(".published-profiles");
    Worker::open(
        std::path::PathBuf::from(directory).join(format!("{}.sqlite", binding.storage_key()?)),
        binding,
    )
    .await
    .map_err(Into::into)
}
async fn source(db: &Database, key: &str, id: Uuid, catalog: &Discovery) -> Result<Snapshot> {
    let key = key.to_owned();
    let original: Snapshot = db
        .read(move |db| {
            read(db, &key, id)?;
            let raw: String = db.query_row(
                "SELECT source FROM profile_enrollments WHERE id=?",
                [id.to_string()],
                |r| r.get(0),
            )?;
            Ok(serde_json::from_str(&raw)?)
        })
        .await?;
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
pub(super) async fn run(
    profile: &MobileProfile,
    scope: Scope,
    catalog: &Discovery,
    command: Command,
) -> Result<Value> {
    let db = &profile.database;
    let key = scope.storage_key()?;
    Ok(match command {
        Command::Current => db.read(move |db| {
            let id: Option<String> = db.query_row("SELECT id FROM profile_enrollments WHERE scope=? AND phase!='cancelled' ORDER BY seq DESC LIMIT 1", [&key], |r| r.get(0)).optional()?;
            Ok(serde_json::to_value(id.map(|id| read(db, &key, Uuid::parse_str(&id)?)).transpose()?)?)
        }).await?,
        Command::Rows { id, after } => db.read(move |db| store::rows(db, &key, id, after)).await?,
        Command::Prepare { id, profile, generation, revision, preferences } => {
            preferences.validate()?;
            let snapshot = catalog.snapshot(profile, generation, revision).await?;
            serde_json::to_value(db.write(move |db| store::prepare(db, &key, id, snapshot, preferences)).await?)?
        }
        Command::Step { id } => {
            let result = step(profile, &key, id, catalog).await;
            if let Err(error) = &result {
                let key = key.clone(); let message = error.to_string();
                db.write(move |db| { let mut review = read(db, &key, id)?; review.error = Some(message); write(db, &review) }).await?;
            }
            serde_json::to_value(result?)?
        }
        Command::Choose { id, position, selected } => serde_json::to_value(db.write(move |db| store::choose(db, &key, id, position, selected)).await?)?,
        Command::Approve { id, accounts, settings } => {
            source(db, &key, id, catalog).await?;
            let lookup = key.clone();
            let review = db.read(move |db| read(db, &lookup, id)).await?;
            if review.phase == "review" {
                let history = worker(db, review.binding.clone()).await?;
                let state = history.request(HistoryCommand::State).await;
                let closed = history.close().await;
                let Reply::State(state) = state? else { return Err(changed()); };
                closed?;
                ensure!(state.revision == review.history_revision && state.initialized && !state.removed,
                    "Local profile history changed. Cancel this review and prepare it again.");
            }
            serde_json::to_value(db.write(move |db| store::approve(db, &key, id, accounts, settings)).await?)?
        }
        Command::Settings { id } => db.read(move |db| apply::settings(db, &key, id)).await?,
        Command::ConfirmSettings { id, applied, kept } => serde_json::to_value(db.write(move |db| apply::confirm(db, &key, id, applied, kept)).await?)?,
        Command::Cancel { id } => serde_json::to_value(db.write(move |db| {
            let mut review = read(db, &key, id)?;
            ensure!(matches!(review.phase.as_str(), "copying" | "draining" | "fields" | "planning" | "review" | "cancelled"), "This enrollment has approved changes. Pause and resume it to retain its receipts.");
            review.phase = "cancelled".into(); write(db, &review)?; Ok(review)
        }).await?)?,
    })
}
async fn step(profile: &MobileProfile, key: &str, id: Uuid, catalog: &Discovery) -> Result<Review> {
    let db = &profile.database;
    let lookup = key.to_owned();
    let review = db.read(move |db| read(db, &lookup, id)).await?;
    if matches!(
        review.phase.as_str(),
        "copying" | "draining" | "fields" | "planning"
    ) {
        let snapshot = source(db, key, id, catalog).await?;
        let history = worker(db, review.binding.clone()).await?;
        let result = transfer::step(db, key, review, snapshot, catalog, &history).await;
        let closed = history.close().await;
        let result = result?;
        closed?;
        return Ok(result);
    }
    if review.phase == "applying" {
        return apply::step(profile, key, review).await;
    }
    ensure!(
        matches!(review.phase.as_str(), "settings" | "complete"),
        "Review the selected profile before applying it."
    );
    Ok(review)
}
#[cfg(test)]
mod tests;
