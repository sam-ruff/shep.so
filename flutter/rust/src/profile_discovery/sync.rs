//! Ongoing device reconciliation for the eight portable preferences. The ledger
//! records every local edit, remote application and checked decision durably
//! before the shared history or the platform preference store sees it, so a
//! lost acknowledgment retries the same operation rather than minting another.
mod cycle;
mod ledger;
mod reviews;
use super::{
    Remote, changed,
    enrollment::{Preferences, SETTINGS, worker},
};
use crate::{api::MobileProfile, database::Database};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shep_profile_core::{
    Action, Change,
    drive::catalog::{Discovery, Phase, Scope, Snapshot},
    history::{Binding, Command as HistoryCommand, Record, Reply, Worker},
};
use std::collections::BTreeMap;
use uuid::Uuid;

/// Bounded work per foreground cycle; the next cycle resumes remaining work.
const BATCH: usize = 32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Basis {
    /// Current common shared version, absent while the profile has no value.
    pub operation: Option<Uuid>,
    /// Scalar of that version; null is an explicit reset to the mobile default.
    pub value: Value,
    /// Device revision acknowledged with this basis. Absent without proof of the
    /// original write, in which case a differing remote value needs a review.
    pub native_revision: Option<u64>,
    /// Device revision last seen for this field; a later revision is local intent.
    pub observed: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Report {
    pub admitted: u64,
    pub imported: u64,
    pub applied: u64,
    pub published: u64,
    pub deferred: u64,
    pub remaining: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Subscription {
    pub id: Uuid,
    pub binding: Binding,
    pub name: Option<String>,
    pub origin: String,
    pub enabled: bool,
    pub fields: BTreeMap<String, bool>,
    pub bases: BTreeMap<String, Basis>,
    pub cursor: u64,
    pub source_device: Option<Uuid>,
    pub history_revision: u64,
    pub revision: u64,
    pub last: Option<Report>,
    pub error: Option<String>,
}
impl Subscription {
    fn field_enabled(&self, field: &str) -> bool {
        self.enabled && self.fields.get(field).copied().unwrap_or(true)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Review {
    pub id: Uuid,
    pub field: String,
    pub kind: String,
    pub local: Value,
    pub native_revision: u64,
    pub history_revision: u64,
    pub versions: Vec<Uuid>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Choice {
    Local,
    Shared { operation: Uuid },
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
    Current,
    Subscribe {
        enrollment: Uuid,
        snapshot: Preferences,
    },
    Configure {
        expected_revision: u64,
        enabled: Option<bool>,
        field: Option<String>,
        selected: Option<bool>,
    },
    Cycle {
        snapshot: Preferences,
    },
    Application,
    ConfirmApplication {
        id: Uuid,
        applied: Vec<String>,
        kept: Vec<String>,
        revisions: BTreeMap<String, u64>,
    },
    Reviews,
    ReviewVersions {
        id: Uuid,
        after: Option<Uuid>,
    },
    Decide {
        id: Uuid,
        choice: Choice,
        seen: u64,
        snapshot: Preferences,
    },
}
impl Command {
    pub fn reads(&self) -> bool {
        matches!(
            self,
            Self::Current | Self::Application | Self::Reviews | Self::ReviewVersions { .. }
        )
    }
}

/// Verified remote access for one cycle. Production binds the session's Drive
/// transport and durable catalog; tests substitute an in-memory second device.
#[async_trait::async_trait]
pub(crate) trait Source: Send + Sync {
    /// Advance discovery by bounded read-only steps; true once complete.
    async fn pull(&self) -> Result<bool>;
    async fn snapshot(&self, profile: Uuid, generation: Uuid) -> Result<Snapshot>;
    async fn source_device(&self, source: Snapshot) -> Result<Uuid>;
    async fn export(&self, source: Snapshot, after: u64) -> Result<Option<Record>>;
    async fn publish(&self, worker: &Worker) -> Result<Option<Uuid>>;
}
pub(super) struct Live<'a> {
    pub remote: &'a dyn Remote,
    pub catalog: &'a Discovery,
}
#[async_trait::async_trait]
impl Source for Live<'_> {
    async fn pull(&self) -> Result<bool> {
        let state = self.catalog.state().await?;
        if state.phase == Phase::Complete && state.error.is_none() {
            self.catalog.refresh(state.revision, false).await?;
        } else if state.error.is_some() {
            self.catalog.retry(state.revision).await?;
        }
        for _ in 0..BATCH {
            let state = self.remote.advance(self.catalog).await?;
            if state.phase == Phase::Complete && state.error.is_none() {
                return Ok(true);
            }
        }
        Ok(false)
    }
    async fn snapshot(&self, profile: Uuid, generation: Uuid) -> Result<Snapshot> {
        Ok(self.catalog.latest_snapshot(profile, generation).await?)
    }
    async fn source_device(&self, source: Snapshot) -> Result<Uuid> {
        Ok(self.catalog.source_device(source).await?)
    }
    async fn export(&self, source: Snapshot, after: u64) -> Result<Option<Record>> {
        Ok(self.catalog.export_record(source, after).await?)
    }
    async fn publish(&self, worker: &Worker) -> Result<Option<Uuid>> {
        self.remote.publish(worker, self.catalog).await
    }
}

fn target(field: &str) -> String {
    format!("setting:{field}")
}
fn scalar(field: &str, change: &Change) -> Result<Value> {
    let expected = target(field);
    ensure!(
        shep_profile_core::history::target(&change.action) == expected,
        "The shared preference identity changed. Refresh its review."
    );
    Ok(match &change.action {
        Action::Setting { value, .. } => value.clone(),
        Action::SettingRemoved { .. } => Value::Null,
        _ => anyhow::bail!("This preference needs a newer Shep version."),
    })
}
/// Replace only the native value; opaque extensions of the reviewed shared
/// version stay with the field so the shared worker accepts the replacement.
fn setting_change(field: &str, value: &Value, source: Option<&Change>) -> Result<Change> {
    let key = serde_json::from_value(serde_json::json!(field))?;
    Ok(Change {
        action: if value.is_null() {
            Action::SettingRemoved { key }
        } else {
            Action::Setting {
                key,
                value: value.clone(),
            }
        },
        extra: source.map(|c| c.extra.clone()).unwrap_or_default(),
    })
}
fn state(reply: Reply) -> Result<shep_profile_core::history::State> {
    match reply {
        Reply::State(state) => Ok(state),
        _ => Err(changed()),
    }
}
async fn versions(worker: &Worker, field: &str) -> Result<Vec<Uuid>> {
    let mut all = Vec::new();
    let mut after = None;
    loop {
        let Reply::Versions(page) = worker
            .request(HistoryCommand::Versions {
                target: target(field),
                after,
            })
            .await?
        else {
            return Err(changed());
        };
        if page.is_empty() {
            break;
        }
        ensure!(
            all.len() + page.len() <= shep_profile_core::MAX_PARENTS,
            "Too many versions of this preference need review. Update Shep before resolving them."
        );
        after = page.last().map(|v| v.operation);
        all.extend(page.into_iter().map(|v| v.operation));
    }
    Ok(all)
}
async fn value(worker: &Worker, field: &str, operation: Uuid) -> Result<Change> {
    match worker
        .request(HistoryCommand::Value {
            target: target(field),
            operation,
        })
        .await?
    {
        Reply::Value(change) => Ok(change),
        _ => Err(changed()),
    }
}
fn read(db: &Connection, key: &str) -> Result<Option<Subscription>> {
    let raw: Option<String> = db
        .query_row(
            "SELECT state FROM profile_subscriptions WHERE scope=?",
            [key],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into))
        .transpose()
}
fn require(db: &Connection, key: &str) -> Result<Subscription> {
    read(db, key)?.context(
        "This device is not keeping a profile in sync with this Google account. Apply a profile first.",
    )
}
fn write(db: &Connection, subscription: &Subscription) -> Result<()> {
    db.execute(
        "UPDATE profile_subscriptions SET state=? WHERE id=?",
        params![
            serde_json::to_string(subscription)?,
            subscription.id.to_string()
        ],
    )?;
    Ok(())
}
fn integer(value: u64) -> Result<i64> {
    value
        .try_into()
        .context("Invalid profile sync cursor. Reopen Profiles and sync.")
}
/// The platform preference store retains one application receipt, shared with
/// enrollment. Neither may stage a device write while the other is unconfirmed.
fn ensure_no_pending_enrollment(db: &Connection) -> Result<()> {
    let pending: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM profile_enrollments WHERE phase NOT IN ('complete','cancelled'))", [], |r| r.get(0))?;
    ensure!(
        !pending,
        "Finish or cancel the pending profile enrollment before syncing preferences."
    );
    Ok(())
}
pub(super) fn pending_application(db: &Connection) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM profile_sync_applications WHERE receipt IS NULL)",
        [],
        |r| r.get(0),
    )?)
}
fn status(db: &Connection, key: &str) -> Result<Value> {
    let Some(subscription) = read(db, key)? else {
        return Ok(Value::Null);
    };
    let id = subscription.id.to_string();
    let count =
        |sql: &str| -> Result<u64> { Ok(db.query_row(sql, [&id], |r| r.get::<_, i64>(0))? as u64) };
    let staged =
        count("SELECT COUNT(*) FROM profile_sync_edits WHERE subscription=? AND state='staged'")?;
    let deferred =
        count("SELECT COUNT(*) FROM profile_sync_edits WHERE subscription=? AND state='deferred'")?;
    let reviews = count("SELECT COUNT(*) FROM profile_sync_reviews WHERE subscription=?")?;
    let applications = count(
        "SELECT COUNT(*) FROM profile_sync_applications WHERE subscription=? AND receipt IS NULL",
    )?;
    let unproven = subscription
        .bases
        .values()
        .filter(|b| b.native_revision.is_none())
        .count();
    let mut value = serde_json::to_value(&subscription)?;
    value["staged"] = staged.into();
    value["deferred"] = deferred.into();
    value["reviews"] = reviews.into();
    value["applications"] = applications.into();
    value["unproven"] = (unproven as u64).into();
    Ok(value)
}
pub(super) async fn run(
    profile: &MobileProfile,
    scope: Scope,
    source: &dyn Source,
    command: Command,
) -> Result<Value> {
    let db = &profile.database;
    let key = scope.storage_key()?;
    Ok(match command {
        Command::Current => db.read(move |db| status(db, &key)).await?,
        Command::Subscribe {
            enrollment,
            snapshot,
        } => {
            snapshot.validate()?;
            ledger::subscribe(db, &key, enrollment, &snapshot).await?;
            ledger::bind_source(db, &key, source).await?;
            db.read(move |db| status(db, &key)).await?
        }
        Command::Configure {
            expected_revision,
            enabled,
            field,
            selected,
        } => {
            let lookup = key.clone();
            db.write(move |db| {
                let mut subscription = require(db, &lookup)?;
                ensure!(
                    subscription.revision == expected_revision,
                    "Profile sync settings changed. Reopen Preferences before changing them."
                );
                if let Some(enabled) = enabled {
                    subscription.enabled = enabled;
                }
                if let Some(field) = field {
                    ensure!(
                        SETTINGS.contains(&field.as_str()),
                        "This preference cannot be synced by this version of Shep."
                    );
                    subscription.fields.insert(
                        field,
                        selected.context("Choose whether to sync this preference.")?,
                    );
                }
                subscription.revision += 1;
                write(db, &subscription)?;
                Ok(())
            })
            .await?;
            db.read(move |db| status(db, &key)).await?
        }
        Command::Cycle { snapshot } => {
            snapshot.validate()?;
            let result = cycle::run(profile, &key, source, snapshot).await;
            let lookup = key.clone();
            let outcome = result
                .as_ref()
                .map(Clone::clone)
                .map_err(ToString::to_string);
            db.write(move |db| {
                let mut subscription = require(db, &lookup)?;
                match &outcome {
                    Ok(report) => {
                        subscription.last = Some(report.clone());
                        subscription.error = None;
                    }
                    Err(message) => subscription.error = Some(message.clone()),
                }
                write(db, &subscription)
            })
            .await?;
            result?;
            db.read(move |db| status(db, &key)).await?
        }
        Command::Application => db.read(move |db| ledger::application(db, &key)).await?,
        Command::ConfirmApplication {
            id,
            applied,
            kept,
            revisions,
        } => {
            db.write(move |db| ledger::confirm(db, &key, id, applied, kept, revisions))
                .await?
        }
        Command::Reviews => db.read(move |db| reviews::list(db, &key)).await?,
        Command::ReviewVersions { id, after } => {
            reviews::versions_page(profile, &key, id, after).await?
        }
        Command::Decide {
            id,
            choice,
            seen,
            snapshot,
        } => {
            snapshot.validate()?;
            reviews::decide(profile, &key, id, choice, seen, snapshot).await?
        }
    })
}

#[cfg(test)]
mod tests;
