//! Frozen first-publication reviews. Device credentials and mail never enter rows.
use super::{Remote, changed};
use crate::database::Database;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use shep_mail_core::{model::Account, profiles::export_account};
use shep_profile_core::{
    Action, Change, Operation, SettingKey,
    drive::catalog::{Discovery, Scope},
    history::{Binding, Command as HistoryCommand, LocalEdit, Reply, Worker},
};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Specification {
    pub name: String,
    pub include_accounts: bool,
    pub settings: BTreeMap<SettingKey, Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Review {
    pub id: Uuid,
    pub binding: Binding,
    pub name: String,
    pub accounts: u64,
    pub settings: u64,
    pub setting_values: BTreeMap<SettingKey, Value>,
    pub total: u64,
    pub staged: u64,
    pub uploaded: u64,
    pub phase: String,
    pub error: Option<String>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Command {
    Current,
    Accounts {
        id: Uuid,
        after: u64,
    },
    Prepare {
        id: Uuid,
        specification: Specification,
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
    pub fn reads(&self) -> bool {
        matches!(self, Self::Current | Self::Accounts { .. })
    }
    pub fn needs_discovery(&self) -> bool {
        matches!(
            self,
            Self::Prepare { .. } | Self::Approve { .. } | Self::Step { .. }
        )
    }
}
fn change(action: Action) -> Change {
    Change {
        action,
        extra: Default::default(),
    }
}
fn read(db: &Connection, scope: &str, id: Uuid) -> Result<Review> {
    let data:String=db.query_row("SELECT review FROM profile_publications WHERE id=? AND scope=?",params![id.to_string(),scope],|r|r.get(0)).optional()?.context("This profile publication is not available for this Google account. Reopen Profiles and sync.")?;
    Ok(serde_json::from_str(&data)?)
}
fn write(db: &Connection, review: &Review) -> Result<()> {
    db.execute(
        "UPDATE profile_publications SET review=?,phase=? WHERE id=?",
        params![
            serde_json::to_string(review)?,
            review.phase,
            review.id.to_string()
        ],
    )?;
    Ok(())
}
pub(super) fn account_fingerprint(db: &Connection) -> Result<String> {
    let mut digest = Sha256::new();
    let mut statement = db.prepare("SELECT id,settings FROM accounts ORDER BY id")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        for i in 0..2 {
            let v: String = row.get(i)?;
            digest.update((v.len() as u64).to_le_bytes());
            digest.update(v);
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn validate(spec: &Specification) -> Result<Vec<Change>> {
    ensure!(
        spec.settings.len() <= 16,
        "Too many settings were selected. Update Shep and retry."
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
    let operation = Operation {
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
    };
    operation.encode()?;
    Ok(changes)
}
fn prepare(db: &mut Connection, scope: Scope, id: Uuid, spec: Specification) -> Result<Review> {
    ensure!(!id.is_nil(), "Choose a valid profile publication identity.");
    let settings = validate(&spec)?;
    let key = scope.storage_key()?;
    let encoded = serde_json::to_string(&spec)?;
    let tx = db.transaction()?;
    let old: Option<(String, String)> = tx
        .query_row(
            "SELECT specification,scope FROM profile_publications WHERE id=?",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((previous, old_scope)) = old {
        ensure!(
            previous == encoded && old_scope == key,
            "This saved profile review differs from the original request. Reopen Profiles and sync."
        );
        return read(&tx, &key, id);
    }
    let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM profile_publications WHERE scope=? AND phase NOT IN ('complete','cancelled'))",[&key],|r|r.get(0))?;
    ensure!(
        !pending,
        "A profile is already being prepared or published. Resume that publication first."
    );
    let binding = Binding {
        namespace: scope.namespace,
        principal: scope.principal,
        profile: id,
        generation: Uuid::new_v4(),
    };
    let mut review = Review {
        id,
        binding,
        name: spec.name.clone(),
        accounts: 0,
        settings: spec.settings.len() as u64,
        setting_values: spec.settings.clone(),
        total: 0,
        staged: 0,
        uploaded: 0,
        phase: "review".into(),
        error: None,
    };
    tx.execute("INSERT INTO profile_publications(id,scope,specification,baseline,phase,review) VALUES(?,?,?,?,?,?)",params![id.to_string(),key,encoded,if spec.include_accounts{account_fingerprint(&tx)?}else{String::new()},review.phase,serde_json::to_string(&review)?])?;
    let mut position = 0u64;
    let mut insert = |changes: Vec<Change>,
                      local: Option<&str>,
                      shared: Option<Uuid>,
                      account: Option<&str>|
     -> Result<()> {
        tx.execute("INSERT INTO profile_publication_rows(publication,position,operation,changes,local_id,shared_id,account) VALUES(?,?,?,?,?,?,?)",params![id.to_string(),integer(position)?,Uuid::new_v4().to_string(),serde_json::to_string(&changes)?,local,shared.map(|v|v.to_string()),account])?;
        position += 1;
        Ok(())
    };
    insert(
        vec![change(Action::ProfileSetup { complete: false })],
        None,
        None,
        None,
    )?;
    insert(settings, None, None, None)?;
    if spec.include_accounts {
        let mut statement = tx.prepare("SELECT id,settings FROM accounts ORDER BY id")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let local: String = row.get(0)?;
            let json: String = row.get(1)?;
            let account: Account = serde_json::from_str(&json)?;
            ensure!(
                account.id == local,
                "A saved account identity is inconsistent. Repair it before publishing."
            );
            let shared = Uuid::parse_str(&local).unwrap_or_else(|_| Uuid::new_v4());
            insert(
                export_account(&account, shared)?,
                Some(&local),
                Some(shared),
                Some(&json),
            )?;
            review.accounts += 1;
        }
    }
    insert(
        vec![change(Action::ProfileSetup { complete: true })],
        None,
        None,
        None,
    )?;
    review.total = position;
    write(&tx, &review)?;
    tx.commit()?;
    Ok(review)
}
fn approve(
    db: &mut Connection,
    scope: &str,
    id: Uuid,
    settings: BTreeMap<SettingKey, Value>,
) -> Result<Review> {
    let tx = db.transaction()?;
    let mut review = read(&tx, scope, id)?;
    if review.phase != "review" {
        ensure!(
            review.phase != "cancelled",
            "This review was cancelled. Prepare another profile."
        );
        return Ok(review);
    }
    let (spec, baseline): (String, String) = tx.query_row(
        "SELECT specification,baseline FROM profile_publications WHERE id=?",
        [id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let spec: Specification = serde_json::from_str(&spec)?;
    ensure!(
        spec.settings == settings,
        "Preferences changed while this review was open. Cancel it and prepare a new review."
    );
    ensure!(
        !spec.include_accounts || baseline == account_fingerprint(&tx)?,
        "Accounts changed while this review was open. Cancel it and prepare a new review."
    );
    let profile = review.binding.storage_key()?;
    tx.execute("INSERT INTO profile_account_mappings(profile,local_id,shared_id) SELECT ?,local_id,shared_id FROM profile_publication_rows WHERE publication=? AND local_id IS NOT NULL",params![profile,id.to_string()])?;
    review.phase = "staging".into();
    write(&tx, &review)?;
    tx.commit()?;
    Ok(review)
}
async fn step(
    db: &Database,
    scope: String,
    id: Uuid,
    remote: &dyn Remote,
    catalog: &Discovery,
) -> Result<Review> {
    let key = scope.clone();
    let review = db.read(move |db| read(db, &key, id)).await?;
    ensure!(
        matches!(review.phase.as_str(), "staging" | "uploading" | "complete"),
        "Review this profile before publishing it."
    );
    if review.phase == "complete" {
        return Ok(review);
    }
    let mut directory = db.path.as_os_str().to_owned();
    directory.push(".published-profiles");
    let path = std::path::PathBuf::from(directory)
        .join(format!("{}.sqlite", review.binding.storage_key()?));
    let worker = Worker::open(path, review.binding.clone()).await?;
    let result = if review.phase == "staging" {
        stage_next(db, &scope, &review, &worker).await
    } else {
        publish_next(db, &scope, &review, &worker, remote, catalog).await
    };
    let closed = worker.close().await;
    match result {
        Err(e) => {
            let key = scope;
            let message = e.to_string();
            db.write(move |db| {
                let mut current = read(db, &key, id)?;
                current.error = Some(message);
                write(db, &current)
            })
            .await?;
            Err(e)
        }
        Ok(result) => {
            closed?;
            Ok(result)
        }
    }
}
async fn stage_next(
    db: &Database,
    scope: &str,
    review: &Review,
    worker: &Worker,
) -> Result<Review> {
    let id = review.id;
    let position = review.staged;
    let (operation, changes, request) = db.read(move |db| {
        Ok(db.query_row(
            "SELECT operation,changes,request FROM profile_publication_rows WHERE publication=? AND position=?",
            params![id.to_string(), integer(position)?],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)),
        )?)
    }).await?;
    let request = if let Some(saved) = request {
        saved
    } else {
        let Reply::State(state) = worker.request(HistoryCommand::State).await? else {
            return Err(changed());
        };
        let request = serde_json::to_string(&LocalEdit {
            operation: Uuid::parse_str(&operation)?,
            expected_revision: state.revision,
            changes: serde_json::from_str(&changes)?,
            resolutions: vec![],
        })?;
        let saved = request.clone();
        db.write(move |db| {
            db.execute(
                "UPDATE profile_publication_rows SET request=? WHERE publication=? AND position=? AND request IS NULL",
                params![saved, id.to_string(), integer(position)?],
            )?;
            Ok(())
        }).await?;
        request
    };
    // Persist the exact request before editing the independent history database.
    // A lost mail-cache receipt must retry that request, including its revision.
    worker
        .request(HistoryCommand::Edit {
            edit: serde_json::from_str(&request)?,
        })
        .await?;
    let key = scope.to_owned();
    db.write(move |db| {
        let mut current = read(db, &key, id)?;
        ensure!(
            current.phase == "staging" && current.staged == position,
            "Profile publication changed. Reopen its saved progress."
        );
        current.staged += 1;
        current.error = None;
        if current.staged == current.total {
            current.phase = "uploading".into();
        }
        write(db, &current)?;
        Ok(current)
    })
    .await
}
async fn publish_next(
    db: &Database,
    scope: &str,
    review: &Review,
    worker: &Worker,
    remote: &dyn Remote,
    catalog: &Discovery,
) -> Result<Review> {
    remote.publish(worker, catalog).await?;
    let Reply::State(state) = worker.request(HistoryCommand::State).await? else {
        return Err(changed());
    };
    ensure!(
        state.initialized && state.operations == review.total,
        "The prepared profile is incomplete. Keep its publication and retry."
    );
    let key = scope.to_owned();
    let id = review.id;
    db.write(move |db| {
        let mut current = read(db, &key, id)?;
        ensure!(
            state.queued <= current.total,
            "Profile upload progress is inconsistent. Reopen it before continuing."
        );
        current.uploaded = current.total - state.queued;
        current.error = None;
        if state.queued == 0 {
            current.phase = "complete".into();
        }
        write(db, &current)?;
        Ok(current)
    })
    .await
}
fn current(db: &Connection, key: &str) -> Result<Value> {
    let id: Option<String> = db.query_row(
        "SELECT id FROM profile_publications WHERE scope=? AND phase!='cancelled' ORDER BY seq DESC LIMIT 1",
        [key], |r| r.get(0),
    ).optional()?;
    Ok(serde_json::to_value(
        id.map(|id| read(db, key, Uuid::parse_str(&id)?))
            .transpose()?,
    )?)
}
fn accounts(db: &Connection, key: &str, id: Uuid, after: u64) -> Result<Value> {
    read(db, key, id)?;
    let mut query = db.prepare(
        "SELECT position,account FROM profile_publication_rows WHERE publication=? AND position>? AND account IS NOT NULL ORDER BY position LIMIT 50",
    )?;
    let rows = query
        .query_map(params![id.to_string(), integer(after)?], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Value::Array(
        rows.into_iter()
            .map(|(position, account)| {
                Ok(serde_json::json!({
                    "position": u64::try_from(position)?,
                    "account": serde_json::from_str::<Value>(&account)?,
                }))
            })
            .collect::<Result<Vec<_>>>()?,
    ))
}
pub(super) async fn run(
    db: &Database,
    scope: Scope,
    remote: &dyn Remote,
    catalog: &Discovery,
    command: Command,
) -> Result<Value> {
    let key = scope.storage_key()?;
    Ok(match command {
        Command::Current => db.read(move |db| current(db, &key)).await?,
        Command::Accounts { id, after } => db.read(move |db| accounts(db, &key, id, after)).await?,
        Command::Prepare { id, specification } => {
            serde_json::to_value(db.write(move |db| prepare(db, scope, id, specification)).await?)?
        }
        Command::Approve { id, settings } => {
            serde_json::to_value(db.write(move |db| approve(db, &key, id, settings)).await?)?
        }
        Command::Step { id } => serde_json::to_value(step(db, key, id, remote, catalog).await?)?,
        Command::Cancel { id } => serde_json::to_value(db.write(move |db| {
            let mut review = read(db, &key, id)?;
            ensure!(
                matches!(review.phase.as_str(), "review" | "cancelled"),
                "This profile already has approved work. Pause publication to keep its progress."
            );
            review.phase = "cancelled".into();
            write(db, &review)?;
            Ok(review)
        }).await?)?,
    })
}
#[cfg(test)]
mod tests;

fn integer(value: u64) -> Result<i64> {
    value
        .try_into()
        .context("Profile cursor is invalid. Reopen the review.")
}
