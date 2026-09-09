//! Explicit local choices. The engine serializes these with owned history steps.
use super::*;
use crate::{
    profiles::{discovery::Grant, preferences::SUPPORTED, publication},
    store::{Store, profile_enrollment},
};
use anyhow::{Context, Result, ensure};
use shep_profile_core::{
    Action,
    drive::catalog::Scope,
    history::{Command as HistoryCommand, Reply, Worker},
};
use std::path::Path;

#[derive(Clone, Debug)]
pub enum Source {
    Enrollment(Uuid),
    Publication(Uuid),
}
#[derive(Clone, Debug)]
pub enum Command {
    Current,
    Review {
        profile: String,
        key: SettingKey,
    },
    ReviewPage {
        profile: String,
        id: Uuid,
        after: Option<Uuid>,
    },
    Resolve {
        profile: String,
        id: Uuid,
        choice: Option<super::resolution::Choice>,
    },
    CancelReview {
        profile: String,
        id: Uuid,
    },
    Prepare(Source),
    Enable {
        profile: String,
        revision: u64,
        enabled: bool,
    },
    Field {
        profile: String,
        revision: u64,
        key: SettingKey,
        enabled: bool,
    },
    Check {
        profile: String,
    },
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct Observation {
    pub profile: Option<String>,
    pub review: Option<super::resolution::Page>,
    #[serde(skip)]
    pub applied: Option<std::sync::Arc<crate::store::PreferenceSnapshot>>,
    pub subscription: Option<Subscription>,
    pub fields: Vec<Field>,
    pub phase: Option<String>,
    pub queued: Option<u64>,
}
struct Prepared {
    binding: Binding,
    name: String,
    fields: BTreeMap<SettingKey, Option<Change>>,
    baseline: BTreeMap<SettingKey, u64>,
    local_intent: BTreeSet<SettingKey>,
    history_revision: Option<u64>,
}
fn prepared(db: &rusqlite::Connection, scope: &str, source: Source) -> Result<Prepared> {
    let result = match source {
        Source::Publication(id) => {
            let review = crate::store::profiles::read(db, scope, id)?;
            ensure!(
                review.phase == publication::Phase::Complete,
                "Finish publishing this reviewed profile before choosing ongoing sync."
            );
            Prepared {
                binding: review.binding,
                name: review.name,
                fields: review
                    .settings
                    .into_iter()
                    .map(|(key, value)| {
                        (
                            key,
                            Some(Change {
                                action: Action::Setting { key, value },
                                extra: Default::default(),
                            }),
                        )
                    })
                    .collect(),
                baseline: review.preference_revisions,
                local_intent: Default::default(),
                history_revision: None,
            }
        }
        Source::Enrollment(id) => {
            let review = profile_enrollment::read(db, scope, id)?;
            ensure!(
                review.phase == "complete" && review.include_settings,
                "Finish applying selected preferences before choosing ongoing sync."
            );
            let baseline = review
                .settings_receipt
                .as_ref()
                .and_then(|r| r.get("local_revisions"))
                .cloned()
                .map(serde_json::from_value)
                .transpose()?
                .unwrap_or_default();
            let mut fields = BTreeMap::new();
            let mut local_intent = BTreeSet::new();
            let mut query=db.prepare("SELECT f.change,r.receipt FROM profile_enrollment_rows r JOIN profile_enrollment_fields f ON f.enrollment=r.enrollment AND f.target=r.target WHERE r.enrollment=? AND r.kind='setting' AND r.choice='apply' AND json_extract(r.details,'$.available')=1 ORDER BY r.target LIMIT 50")?;
            let mut rows = query.query([id.to_string()])?;
            while let Some(row) = rows.next()? {
                let change: Change = serde_json::from_str(&row.get::<_, String>(0)?)?;
                let key = match &change.action {
                    Action::Setting { key, .. } | Action::SettingRemoved { key } => *key,
                    _ => anyhow::bail!("The reviewed preference has an invalid record."),
                };
                if SUPPORTED.contains(&key) {
                    if row.get::<_, Option<String>>(1)?.as_deref() != Some("applied") {
                        local_intent.insert(key);
                    }
                    fields.insert(key, Some(change));
                }
            }
            Prepared {
                binding: review.binding,
                name: review.name.unwrap_or_else(|| "Shared profile".into()),
                fields,
                baseline,
                local_intent,
                history_revision: Some(review.history_revision),
            }
        }
    };
    ensure!(
        !result.fields.is_empty(),
        "This review has no selected supported preferences. Review the preferences to share before enabling sync."
    );
    Ok(result)
}

pub async fn prepare(
    store: &Store,
    root: &Path,
    scope: Scope,
    source: Source,
) -> Result<Subscription> {
    let scope = scope.storage_key()?;
    let review = store.run(move |db| prepared(db, &scope, source)).await?;
    let key = review.binding.storage_key()?;
    let path = root.join("histories").join(format!("{key}.sqlite"));
    ensure!(
        tokio::fs::try_exists(&path).await?,
        "The reviewed profile history is missing. Recover it before enabling sync."
    );
    let history = Worker::open(path, review.binding.clone()).await?;
    let result=async {
        let Reply::State(state)=history.request(HistoryCommand::State).await? else {anyhow::bail!("Could not read the reviewed profile history.")};
        ensure!(state.initialized && !state.removed,"This profile is incomplete or removed. Review its recovery before enabling sync.");
        let existing_key=key.clone();
        let existing=store.run(move |db| {
            let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM profile_sync WHERE profile=?)",[&existing_key],|row|row.get(0))?;
            if exists {crate::store::profile_sync::read(db,&existing_key).map(Some)} else {Ok(None)}
        }).await?;
        if let Some(existing)=existing {
            ensure!(existing.device==state.device && state.revision>=existing.history_revision,"The enrolled history was replaced or rolled back. Recover it before continuing sync.");
            return Ok(existing);
        }
        ensure!(review.history_revision.is_none_or(|revision|revision==state.revision),"The shared history changed after enrollment. Review it again before enabling sync.");
        let Reply::Fields(fields)=history.request(HistoryCommand::Fields {after:Some("setting:".into())}).await? else {anyhow::bail!("Could not inspect the reviewed preferences.")};
        for (key,change) in &review.fields {
            let expected=change.as_ref().context("The reviewed preference has no value.")?;
            let target=shep_profile_core::history::target(&expected.action);
            let field=fields.iter().find(|field|field.target==target).context("This shared preference changed. Review its current value before enabling sync.")?;
            ensure!(field.versions==1 && !field.conflict,"This shared preference has conflicting versions. Review them before enabling sync.");
            let Reply::Versions(versions)=history.request(HistoryCommand::Versions {target:target.clone(),after:None}).await? else {anyhow::bail!("Could not read the reviewed preference version.")};
            ensure!(versions.len()==1,"The preference versions changed. Review them again.");
            let Reply::Value(actual)=history.request(HistoryCommand::Value {target,operation:versions[0].operation}).await? else {anyhow::bail!("Could not read the reviewed preference value.")};
            ensure!(actual==*expected && SUPPORTED.contains(key),"This preference changed after the review. Review its current value before enabling sync.");
        }
        store.profile_sync_seed(Seed {binding:review.binding,device:state.device,name:review.name,history_revision:state.revision,fields:review.fields,local_intent:review.local_intent,baseline:Some(review.baseline)}).await
    }.await;
    let closed = history.close().await;
    let result = result?;
    closed?;
    store.put("profile_sync_selection", key).await?;
    Ok(result)
}

pub fn check_grant(
    grant: &Grant,
    prefs: &crate::model::Preferences,
    subscription: &Subscription,
) -> Result<()> {
    grant.check(prefs)?;
    ensure!(
        grant.principal() == subscription.binding.principal,
        "This profile belongs to another Google account. Reconnect its account before resuming sync."
    );
    Ok(())
}
