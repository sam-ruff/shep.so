//! Durable group actions: a frozen review is staged from the captured
//! selection into `group_jobs`/`group_items`, then executed one owned step at
//! a time through the existing per-message mutation and receipt code. The
//! journal keeps metadata and physical identities only, never MIME or secrets.
use crate::api::MobileProfile;
use crate::operations::{Mutation, stored_account, stored_mail};
use crate::paging::Scope;
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shep_mail_core::model::Protocol;

/// History page size and independently bounded active admission.
pub const HISTORY_JOBS: i64 = 20;
/// Items are staged, listed and retired in pages of this size.
pub const PAGE: usize = 50;
pub(crate) const ACTIVE_CAPACITY_QUERY: &str = "SELECT COUNT(*) FROM (SELECT 1 FROM group_jobs INDEXED BY group_job_state WHERE state IN ('staging','review','running','undoing','paused') LIMIT 21)";
pub(crate) const ATTENTION_COUNT_QUERY: &str = "SELECT COUNT(*) FROM group_items INDEXED BY group_item_attention WHERE state IN ('failed','uncertain','undo_failed','undo_uncertain')";
pub(crate) const ATTENTION_TARGET_QUERY: &str = "SELECT job FROM group_items INDEXED BY group_item_attention WHERE state IN ('failed','uncertain','undo_failed','undo_uncertain') LIMIT 1";
pub(crate) const NEXT_ITEM_QUERY: &str = "SELECT j.id,j.state,COALESCE(CASE WHEN j.state='undoing' THEN j.undone ELSE j.approved END,0),j.fields,i.position,i.mail,i.account,i.folder,i.remote_id,i.unread,i.starred,i.lineage,i.receipt FROM group_jobs j INDEXED BY group_job_state CROSS JOIN group_items i ON i.job=j.id AND i.position=(SELECT position FROM group_items INDEXED BY group_item_state WHERE job=j.id AND state=CASE j.state WHEN 'running' THEN 'pending' ELSE 'undoing' END ORDER BY position LIMIT 1) WHERE j.state IN ('running','undoing') ORDER BY j.seq LIMIT 1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Archive,
    Delete,
    Spam,
    Move { folder: String },
    Read,
    Unread,
    Flag,
    Unflag,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Fields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unread: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starred: Option<bool>,
}
impl Fields {
    fn is_empty(&self) -> bool {
        self.folder.is_none() && self.unread.is_none() && self.starred.is_none()
    }
}
impl Action {
    fn logical_role(&self) -> Option<crate::destinations::Role> {
        use crate::destinations::Role;
        match self {
            Self::Archive => Some(Role::Archive),
            Self::Delete => Some(Role::Trash),
            Self::Spam => Some(Role::Spam),
            _ => None,
        }
    }
    pub fn fields(&self) -> Fields {
        match self {
            Action::Archive => Fields {
                folder: Some("Archive".into()),
                ..Default::default()
            },
            Action::Delete => Fields {
                folder: Some("Trash".into()),
                ..Default::default()
            },
            Action::Spam => Fields {
                folder: Some("Spam".into()),
                ..Default::default()
            },
            Action::Move { folder } => Fields {
                folder: Some(if folder.eq_ignore_ascii_case("Inbox") {
                    "INBOX".into()
                } else {
                    folder.clone()
                }),
                ..Default::default()
            },
            Action::Read => Fields {
                unread: Some(false),
                ..Default::default()
            },
            Action::Unread => Fields {
                unread: Some(true),
                ..Default::default()
            },
            Action::Flag => Fields {
                starred: Some(true),
                ..Default::default()
            },
            Action::Unflag => Fields {
                starred: Some(false),
                ..Default::default()
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    /// Freeze the captured selection at `expected` and stage its exact
    /// membership under the new job `id`. The job waits in review.
    Prepare {
        id: String,
        selection: String,
        expected: u64,
        action: Action,
        scope: Scope,
    },
    Approve {
        id: String,
    },
    Inspect {
        id: String,
    },
    Decline {
        id: String,
    },
    /// Execute at most one owned step. Provider steps need the account's
    /// credential; without it the reply names the account and nothing changes.
    Step {
        #[serde(default)]
        password: Option<SecretString>,
        #[serde(default)]
        credential_slot: Option<String>,
    },
    Pause {
        id: String,
    },
    Resume {
        id: String,
    },
    Undo {
        id: String,
    },
    Retry {
        id: String,
        position: i64,
    },
    Accept {
        id: String,
        position: i64,
    },
    History {
        #[serde(default)]
        before: Option<i64>,
        #[serde(default)]
        tracked: Vec<String>,
    },
    Items {
        id: String,
        #[serde(default)]
        after: Option<i64>,
    },
    Remove {
        id: String,
    },
}

fn token(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "Invalid group identity. Select the messages again."
    );
    Ok(())
}
fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
fn job_state(db: &Connection, id: &str) -> Result<(String, bool)> {
    token(id)?;
    db.query_row(
        "SELECT state,undone IS NOT NULL FROM group_jobs WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .context("This group action is no longer in History.")
}
fn touch(db: &Connection, id: &str) -> Result<()> {
    db.execute(
        "UPDATE group_jobs SET revision=revision+1 WHERE id=?1",
        [id],
    )?;
    Ok(())
}

fn admit_active(db: &Connection) -> Result<()> {
    let count: i64 = db.query_row(ACTIVE_CAPACITY_QUERY, [], |r| r.get(0))?;
    ensure!(
        count < HISTORY_JOBS,
        "There are already {HISTORY_JOBS} active group actions. Finish or remove an open review first."
    );
    Ok(())
}
fn runnable_items(db: &Connection, id: &str) -> Result<i64> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM group_items WHERE job=?1 AND state IN ('pending','sending','undoing','reversing')",
        [id],
        |r| r.get(0),
    )?)
}
/// A job with no queued or in-flight step is finished; failed and uncertain
/// items stay visible in History for explicit decisions.
fn settle(db: &Connection, id: &str) -> Result<()> {
    let (state, _) = job_state(db, id)?;
    if matches!(state.as_str(), "running" | "undoing" | "paused") && runnable_items(db, id)? == 0 {
        db.execute("UPDATE group_jobs SET state='finished' WHERE id=?1", [id])?;
    }
    Ok(())
}

/// Exclusive ownership at open proves no earlier process can still confirm a
/// claimed step. Those steps become uncertain and pause their group; nothing
/// is repeated. Interrupted staging and abandoned reviews are retired.
pub(crate) fn restart(db: &Connection) -> Result<()> {
    db.execute("UPDATE group_items SET state='uncertain',reason='Shep stopped before the server confirmed this step. Check the folder, then accept the current state or refresh.' WHERE state='sending'",[])?;
    db.execute("UPDATE group_items SET state='undo_uncertain',reason='Shep stopped before the server confirmed this Undo step. Check the folder, then accept the current state or refresh.' WHERE state='reversing'",[])?;
    db.execute("UPDATE group_jobs SET state='paused' WHERE state IN ('running','undoing') AND EXISTS(SELECT 1 FROM group_items WHERE job=group_jobs.id AND state IN ('uncertain','undo_uncertain') AND attempt IS NOT NULL)",[])?;
    db.execute(
        "UPDATE group_jobs SET state='interrupted',error='Preparation was interrupted before the review completed. Select the messages again.' WHERE state='staging'",
        [],
    )?;
    db.execute(
        "UPDATE group_jobs SET state='cancelled' WHERE state='review'",
        [],
    )?;
    for _ in 0..200 {
        if !sweep_one(db)? {
            break;
        }
    }
    Ok(())
}

/// Retire one retired job's rows in one bounded transaction. Approved work,
/// receipts and decisions are never touched here.
fn sweep_one(db: &Connection) -> Result<bool> {
    let candidate: Option<String> = db
        .query_row(
            "SELECT id FROM group_jobs WHERE state IN ('cancelled','interrupted') ORDER BY seq LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let Some(id) = candidate else {
        return Ok(false);
    };
    retire_page(db, &id)?;
    Ok(true)
}
fn retire_page(db: &Connection, id: &str) -> Result<bool> {
    let deleted = db.execute(
        "DELETE FROM group_items WHERE (job,position) IN (SELECT job,position FROM group_items WHERE job=?1 ORDER BY position LIMIT ?2)",
        params![id, PAGE as i64],
    )?;
    if deleted < PAGE {
        db.execute("DELETE FROM group_jobs WHERE id=?1", [id])?;
        return Ok(true);
    }
    Ok(false)
}

/// Account removal: cancel this account's queued, failed and uncertain work
/// and abandon reviews that froze it. Receipts of completed steps remain.
pub(crate) fn fence_account(db: &Connection, account: &str) -> Result<()> {
    db.execute(&format!("UPDATE group_items SET state='cancelled',attempt=NULL,reason='Account removed from this device' WHERE account=?1 AND state IN {}",crate::accounts::ACTIVE_GROUP_ITEMS),[account])?;
    db.execute("UPDATE group_jobs SET state='cancelled' WHERE state IN ('review','staging') AND EXISTS(SELECT 1 FROM group_items WHERE job=group_jobs.id AND account=?1)",[account])?;
    let jobs = db
        .prepare("SELECT DISTINCT job FROM group_items WHERE account=?1 AND state='cancelled'")?
        .query_map([account], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for job in jobs {
        settle(db, &job)?;
        touch(db, &job)?;
    }
    Ok(())
}

pub(crate) async fn run(profile: &MobileProfile, command: Command) -> Result<Value> {
    let db = &profile.database;
    match command {
        Command::Prepare {
            id,
            selection,
            expected,
            action,
            scope,
        } => prepare(profile, id, selection, expected, action, scope).await,
        Command::Inspect { id } => {
            token(&id)?;
            db.read(move |db| {
                let tx = db.unchecked_transaction()?;
                let value = summary(&tx, &id)?;
                tx.commit()?;
                Ok(value)
            })
            .await
        }
        Command::Approve { id } => {
            db.write(move |db| {
                let tx = db.transaction()?;
                let (state, _) = job_state(&tx, &id)?;
                ensure!(state == "review", "This review is no longer open. Select the messages again.");
                let removed: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM group_items i WHERE i.job=?1 AND NOT EXISTS(SELECT 1 FROM accounts a WHERE a.id=i.account))",
                    [&id],
                    |r| r.get(0),
                )?;
                if removed {
                    tx.execute("UPDATE group_jobs SET state='cancelled' WHERE id=?1", [&id])?;
                    tx.commit()?;
                    anyhow::bail!("An account in this selection was removed. Select the messages again.");
                }
                tx.execute("UPDATE group_clock SET revision=revision+1 WHERE id=1", [])?;
                tx.execute("UPDATE group_jobs SET state='running',approved=(SELECT revision FROM group_clock WHERE id=1),revision=revision+1 WHERE id=?1",[&id])?;
                settle(&tx, &id)?;
                let value = summary(&tx, &id)?;
                tx.commit()?;
                Ok(value)
            })
            .await
        }
        Command::Decline { id } => {
            token(&id)?;
            loop {
                let job = id.clone();
                let finished = db
                    .write(move |db| {
                        let tx = db.transaction()?;
                        let Some((state, _)) = job_state(&tx, &job).ok() else {
                            return Ok(true);
                        };
                        ensure!(
                            matches!(state.as_str(), "review" | "staging" | "cancelled" | "interrupted"),
                            "This group action was approved. Use Undo in History instead."
                        );
                        tx.execute("UPDATE group_jobs SET state='cancelled' WHERE id=?1", [&job])?;
                        let done = retire_page(&tx, &job)?;
                        tx.commit()?;
                        Ok(done)
                    })
                    .await?;
                if finished {
                    break;
                }
            }
            Ok(json!({"declined":true}))
        }
        Command::Step {
            password,
            credential_slot,
        } => step(profile, password, credential_slot).await,
        Command::Pause { id } => {
            db.write(move |db| {
                let tx = db.transaction()?;
                let (state, _) = job_state(&tx, &id)?;
                ensure!(matches!(state.as_str(), "running" | "undoing"), "This group action is not running.");
                tx.execute("UPDATE group_jobs SET state='paused' WHERE id=?1", [&id])?;
                touch(&tx, &id)?;
                let value = summary(&tx, &id)?;
                tx.commit()?;
                Ok(value)
            })
            .await
        }
        Command::Resume { id } => {
            db.write(move |db| {
                let tx = db.transaction()?;
                let (state, undo) = job_state(&tx, &id)?;
                ensure!(state == "paused", "This group action is not paused.");
                tx.execute(
                    "UPDATE group_jobs SET state=?2 WHERE id=?1",
                    params![id, if undo { "undoing" } else { "running" }],
                )?;
                settle(&tx, &id)?;
                touch(&tx, &id)?;
                let value = summary(&tx, &id)?;
                tx.commit()?;
                Ok(value)
            })
            .await
        }
        Command::Undo { id } => {
            db.write(move |db| {
                let tx = db.transaction()?;
                let (state, undo) = job_state(&tx, &id)?;
                if undo {
                    return summary(&tx, &id);
                }
                ensure!(
                    matches!(state.as_str(), "running" | "paused" | "finished"),
                    "This group action cannot be undone from its current state."
                );
                if state == "finished" {
                    admit_active(&tx)?;
                }
                // Unsent forward steps are cancelled before any provider call;
                // acknowledged steps queue their inverse. Failed, skipped and
                // uncertain items keep their state and are never repeated.
                tx.execute("UPDATE group_clock SET revision=revision+1 WHERE id=1", [])?;
                tx.execute("UPDATE group_items SET state='cancelled',reason='Cancelled before sending' WHERE job=?1 AND state='pending'",[&id])?;
                tx.execute("UPDATE group_items SET state='undoing',reason=NULL WHERE job=?1 AND state='done'",[&id])?;
                tx.execute("UPDATE group_jobs SET state='undoing',undone=(SELECT revision FROM group_clock WHERE id=1),revision=revision+1 WHERE id=?1",[&id])?;
                settle(&tx, &id)?;
                let value = summary(&tx, &id)?;
                tx.commit()?;
                Ok(value)
            })
            .await
        }
        Command::Retry { id, position } => {
            db.write(move |db| {
                let tx = db.transaction()?;
                let (state, undo) = job_state(&tx, &id)?;
                let item: String = tx
                    .query_row(
                        "SELECT state FROM group_items WHERE job=?1 AND position=?2",
                        params![id, position],
                        |r| r.get(0),
                    )
                    .context("This message is no longer part of the group action.")?;
                let next = match item.as_str() {
                    "failed" if !undo => "pending",
                    "undo_failed" if undo => "undoing",
                    "uncertain" | "undo_uncertain" => anyhow::bail!(
                        "The server result is unknown. Refresh and check the folder, then accept the current state; Shep never repeats an unconfirmed step."
                    ),
                    _ => anyhow::bail!("Only a failed step can be retried."),
                };
                if state == "finished" {
                    admit_active(&tx)?;
                }
                tx.execute(
                    "UPDATE group_items SET state=?3,reason=NULL,attempt=NULL WHERE job=?1 AND position=?2",
                    params![id, position, next],
                )?;
                if state == "finished" {
                    tx.execute(
                        "UPDATE group_jobs SET state=?2 WHERE id=?1",
                        params![id, if undo { "undoing" } else { "running" }],
                    )?;
                }
                touch(&tx, &id)?;
                let value = summary(&tx, &id)?;
                tx.commit()?;
                Ok(value)
            })
            .await
        }
        Command::Accept { id, position } => {
            db.write(move |db| {
                let tx = db.transaction()?;
                job_state(&tx, &id)?;
                let item: String = tx
                    .query_row(
                        "SELECT state FROM group_items WHERE job=?1 AND position=?2",
                        params![id, position],
                        |r| r.get(0),
                    )
                    .context("This message is no longer part of the group action.")?;
                ensure!(
                    matches!(item.as_str(), "uncertain" | "undo_uncertain"),
                    "Only an unconfirmed step can be accepted."
                );
                // Retires the local intent only. The cache and the provider
                // outcome are not classified; the message keeps whatever a
                // later refresh shows.
                tx.execute(
                    "UPDATE group_items SET state='accepted',reason='Current state accepted without a server confirmation' WHERE job=?1 AND position=?2",
                    params![id, position],
                )?;
                touch(&tx, &id)?;
                let value = summary(&tx, &id)?;
                tx.commit()?;
                Ok(value)
            })
            .await
        }
        Command::History { before, tracked } => db.read(move |db| {
            ensure!(tracked.len() <= 20, "Observe at most 20 active groups.");
            for id in &tracked { token(id)?; }
            let tx = db.unchecked_transaction()?;
            let value = history(&tx, before, &tracked)?;
            tx.commit()?;
            Ok(value)
        }).await,
        Command::Items { id, after } => db.read(move |db| {
            let tx=db.unchecked_transaction()?;
            let value=items(&tx,&id,after)?;
            tx.commit()?;
            Ok(value)
        }).await,
        Command::Remove { id } => {
            token(&id)?;
            loop {
                let job = id.clone();
                let finished = db
                    .write(move |db| {
                        let tx = db.transaction()?;
                        let Some((state, _)) = job_state(&tx, &job).ok() else {
                            return Ok(true);
                        };
                        ensure!(
                            matches!(state.as_str(), "finished" | "cancelled" | "interrupted"),
                            "Pause and finish or undo this group action before removing it from History."
                        );
                        let done = retire_page(&tx, &job)?;
                        tx.commit()?;
                        Ok(done)
                    })
                    .await?;
                if finished {
                    break;
                }
            }
            Ok(json!({"removed":true}))
        }
    }
}

async fn prepare(
    profile: &MobileProfile,
    id: String,
    selection: String,
    expected: u64,
    action: Action,
    scope: Scope,
) -> Result<Value> {
    token(&id)?;
    let db = &profile.database;
    let fields = action.fields();
    let job = id.clone();
    let action_json = serde_json::to_string(&action)?;
    let fields_json = serde_json::to_string(&fields)?;
    let mut scope = scope;
    scope.projection.clear();
    let scope_json = serde_json::to_string(&scope)?;
    db.write(move |db| {
        let tx = db.transaction()?;
        for _ in 0..4 {
            if !sweep_one(&tx)? {
                break;
            }
        }
        admit_active(&tx)?;
        tx.execute("INSERT INTO group_jobs(id,action,fields,state,scope,created) VALUES(?1,?2,?3,'staging',?4,?5)",params![job,action_json,fields_json,scope_json,now()])?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    let staged = stage(profile, &id, &selection, expected).await;
    let job = id.clone();
    match staged {
        Ok(()) => {
            db.write(move |db| {
                let tx = db.transaction()?;
                tx.execute("UPDATE group_jobs SET state='review',total=(SELECT COUNT(*) FROM group_items WHERE job=?1) WHERE id=?1 AND state='staging'",[&job])?;
                let value = summary(&tx, &job)?;
                tx.commit()?;
                Ok(value)
            })
            .await
        }
        Err(error) => {
            let text = error.to_string();
            let _ = db
                .write(move |db| {
                    db.execute("UPDATE group_jobs SET state='interrupted',error=?2 WHERE id=?1 AND state='staging'",params![job,text])?;
                    Ok(())
                })
                .await;
            Err(error)
        }
    }
}

/// Freeze the capture, then copy its exact membership in pages of 50 with
/// each message's current physical identity. Missing messages stay in the
/// frozen group as skipped rows so the review count is exact.
async fn stage(profile: &MobileProfile, id: &str, selection: &str, expected: u64) -> Result<()> {
    let db = &profile.database;
    let frozen = format!("{id}-frozen");
    let (target, source) = (frozen.clone(), selection.to_owned());
    db.selection(move |db| {
        crate::selection::run(
            db,
            crate::selection::Command::Freeze {
                id: source,
                expected,
                target: target.clone(),
            },
            vec![],
        )
    })
    .await
    .context("The selection changed while preparing this action. Review it again.")?;
    let mut after: Option<u64> = None;
    let result = loop {
        let page_id = frozen.clone();
        let page = match db
            .selection(move |db| {
                crate::selection::run(
                    db,
                    crate::selection::Command::Page {
                        id: page_id,
                        expected: 0,
                        after,
                    },
                    vec![],
                )
            })
            .await
        {
            Ok(page) => page,
            Err(error) => break Err(error),
        };
        let rows = page["rows"].as_array().cloned().unwrap_or_default();
        let job = id.to_owned();
        if let Err(error) = db
            .write(move |db| {
                let tx = db.transaction()?;
                let (state, _) = job_state(&tx, &job)?;
                ensure!(state == "staging", "This review was cancelled.");
                let action: Action = serde_json::from_str(&tx.query_row("SELECT action FROM group_jobs WHERE id=?1", [&job], |row|row.get::<_,String>(0))?)?;
                let mut accounts = std::collections::BTreeSet::new();
                for row in rows {
                    let position = row["position"].as_i64().context("Invalid selection page.")?;
                    let mail_id = row["id"].as_str().context("Invalid selection page.")?;
                    let (account, folder, unread, starred) = (
                        row["account"].as_str().unwrap_or_default(),
                        row["folder"].as_str().unwrap_or_default(),
                        row["unread"].as_bool().unwrap_or_default(),
                        row["starred"].as_bool().unwrap_or_default(),
                    );
                    let current = stored_mail(&tx, mail_id).ok();
                    match current {
                        Some(mail) => {
                            let lineage: String = tx.query_row("SELECT token FROM mail_lineage WHERE id=?1", [&mail.id], |row|row.get(0))?;
                            let changed = tx.execute(
                                "INSERT INTO group_items(job,position,mail,account,folder,remote_id,unread,starred,state,lineage) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending',?9)",
                                params![job,position,mail.id,mail.account_id,mail.folder,mail.remote_id,mail.unread,mail.starred,lineage],
                            )?;
                            if let Some(role)=action.logical_role() && accounts.insert(mail.account_id.clone()) {
                                crate::destinations::admit(&tx,"group",&job,&mail.account_id,role)?;
                            }
                            changed
                        },
                        None => tx.execute(
                            "INSERT INTO group_items(job,position,mail,account,folder,remote_id,unread,starred,state,reason) VALUES(?1,?2,?3,?4,?5,'',?6,?7,'skipped','Message is no longer cached')",
                            params![job,position,mail_id,account,folder,unread,starred],
                        )?,
                    };
                }
                tx.commit()?;
                Ok(())
            })
            .await
        {
            break Err(error);
        }
        match page["next_after"].as_u64() {
            Some(next) => after = Some(next),
            None => break Ok(()),
        }
    };
    let _ = db
        .selection(move |db| {
            crate::selection::run(
                db,
                crate::selection::Command::Release { id: frozen },
                vec![],
            )
        })
        .await;
    result
}

fn counts(db: &Connection, id: &str) -> Result<Value> {
    let mut statement =
        db.prepare("SELECT state,COUNT(*) FROM group_items WHERE job=?1 GROUP BY state")?;
    let mut counts = serde_json::Map::new();
    for row in statement.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (state, n) = row?;
        counts.insert(state, n.into());
    }
    Ok(Value::Object(counts))
}
fn summary(db: &Connection, id: &str) -> Result<Value> {
    let (seq, action, fields, state, scope, created, total, revision, error, undo): (
        i64,
        String,
        String,
        String,
        String,
        i64,
        i64,
        i64,
        Option<String>,
        bool,
    ) = db.query_row(
        "SELECT seq,action,fields,state,scope,created,total,revision,error,undone IS NOT NULL FROM group_jobs WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?)),
    )?;
    let groups = db.prepare("SELECT account,folder,COUNT(*),SUM(unread),SUM(starred) FROM group_items WHERE job=?1 GROUP BY account,folder ORDER BY account,folder")?
        .query_map([id],|r|Ok(json!({"account":r.get::<_,String>(0)?,"folder":r.get::<_,String>(1)?,"total":r.get::<_,i64>(2)?,"unread":r.get::<_,i64>(3)?,"starred":r.get::<_,i64>(4)?})))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let total = if state == "staging" {
        db.query_row("SELECT COUNT(*) FROM group_items WHERE job=?1", [id], |r| {
            r.get::<_, i64>(0)
        })?
    } else {
        total
    };
    Ok(json!({
        "id":id,"seq":seq,"action":serde_json::from_str::<Value>(&action)?,"fields":serde_json::from_str::<Value>(&fields)?,
        "state":state,"scope":serde_json::from_str::<Value>(&scope)?,"created":created,"total":total,
        "revision":revision,"error":error,"undo":undo,"counts":counts(db,id)?,"groups":groups
    }))
}
fn history(db: &Connection, before: Option<i64>, tracked: &[String]) -> Result<Value> {
    let mut ids = db
        .prepare("SELECT id,seq FROM group_jobs INDEXED BY group_history_cursor WHERE state IN ('staging','review','running','undoing','paused','finished') AND seq<?1 ORDER BY seq DESC LIMIT 21")?
        .query_map([before.unwrap_or(i64::MAX)], |r| Ok((r.get::<_, String>(0)?,r.get::<_, i64>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = ids.len() > HISTORY_JOBS as usize;
    ids.truncate(HISTORY_JOBS as usize);
    let newer = db.prepare("SELECT seq FROM group_jobs INDEXED BY group_history_cursor WHERE state IN ('staging','review','running','undoing','paused','finished') AND seq>?1 ORDER BY seq ASC LIMIT 21")?
        .query_map([ids.first().map_or(before.map_or(i64::MAX,|cursor|cursor.saturating_sub(1)), |(_,seq)|*seq)], |r| r.get::<_,i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let jobs = ids
        .iter()
        .map(|(id, _)| summary(db, id))
        .collect::<Result<Vec<_>>>()?;
    let active = db.prepare("SELECT id FROM group_jobs INDEXED BY group_job_state WHERE state IN ('running','undoing','paused') ORDER BY seq LIMIT 20")?
        .query_map([], |r| r.get::<_,String>(0))?
        .map(|r| summary(db, &r?)).collect::<Result<Vec<_>>>()?;
    let attention: i64 = db.query_row(ATTENTION_COUNT_QUERY, [], |r| r.get(0))?;
    let target: Option<String> = db
        .query_row(ATTENTION_TARGET_QUERY, [], |r| r.get(0))
        .optional()?;
    let attention_job = target.map(|id| summary(db, &id)).transpose()?;
    let mut observed = Vec::with_capacity(tracked.len());
    for id in tracked {
        let exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM group_jobs WHERE id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if exists {
            observed.push(summary(db, id)?);
        }
    }
    let runnable: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_jobs j INDEXED BY group_job_state WHERE j.state IN ('running','undoing') AND EXISTS(SELECT 1 FROM group_items WHERE job=j.id AND state=CASE j.state WHEN 'running' THEN 'pending' ELSE 'undoing' END))",
        [],
        |r| r.get(0),
    )?;
    Ok(
        json!({"jobs":jobs,"runnable":runnable,"active":active,"attention":attention,"attention_job":attention_job,"tracked":observed,
        "next_before":if more {ids.last().map(|(_,seq)|*seq)} else {None},
        "has_previous":!newer.is_empty(),"previous_before":if newer.len()>20 {newer.last().copied()} else {None}}),
    )
}
fn items(db: &Connection, id: &str, after: Option<i64>) -> Result<Value> {
    job_state(db, id)?;
    let rows = db.prepare("SELECT i.position,i.mail,i.account,i.folder,i.unread,i.starred,i.state,i.fields,i.reason,i.receipt,m.subject,m.sender FROM group_items i LEFT JOIN mail m ON m.id=i.mail WHERE i.job=?1 AND i.position>?2 ORDER BY i.position LIMIT ?3")?
        .query_map(params![id,after.unwrap_or(-1),PAGE as i64],|r|Ok(json!({
            "position":r.get::<_,i64>(0)?,"mail":r.get::<_,String>(1)?,"account":r.get::<_,String>(2)?,"folder":r.get::<_,String>(3)?,
            "unread":r.get::<_,bool>(4)?,"starred":r.get::<_,bool>(5)?,"state":r.get::<_,String>(6)?,
            "fields":r.get::<_,Option<String>>(7)?.map(|f|serde_json::from_str::<Value>(&f)).transpose().unwrap_or_default(),
            "reason":r.get::<_,Option<String>>(8)?,
            "receipt":r.get::<_,Option<String>>(9)?.map(|f|serde_json::from_str::<Value>(&f)).transpose().unwrap_or_default(),
            "subject":r.get::<_,Option<String>>(10)?,"sender":r.get::<_,Option<String>>(11)?
        })))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let next_after = (rows.len() == PAGE
        && db.query_row(
            "SELECT EXISTS(SELECT 1 FROM group_items WHERE job=?1 AND position>?2)",
            params![
                id,
                rows.last()
                    .and_then(|r| r["position"].as_i64())
                    .unwrap_or(-1)
            ],
            |r| r.get::<_, bool>(0),
        )?)
    .then(|| {
        rows.last()
            .map(|r| r["position"].clone())
            .unwrap_or(Value::Null)
    });
    let newer = db.prepare("SELECT position FROM group_items WHERE job=?1 AND position<?2 ORDER BY position DESC LIMIT 51")?
        .query_map(params![id, rows.first().and_then(|r|r["position"].as_i64()).unwrap_or(after.unwrap_or(-1).saturating_add(1))],|r|r.get::<_,i64>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(
        json!({"id":id,"rows":rows,"next_after":next_after,"has_previous":!newer.is_empty(),"previous_after":newer.get(50).copied()}),
    )
}

#[derive(Clone, Deserialize, Serialize)]
struct Identity {
    folder: String,
    remote_id: String,
    unread: bool,
    starred: bool,
    #[serde(default)]
    lineage: Option<String>,
    #[serde(default)]
    encoding: Option<shep_mail_core::folders::NameEncoding>,
}

#[derive(Clone)]
pub(crate) struct DispatchClaim {
    job: String,
    position: i64,
    attempt: String,
    account: String,
    inverse: bool,
    approved: i64,
    fields: Fields,
    source: Identity,
}

pub(crate) enum Dispatch {
    Send(Fields),
    Stop { outcome: &'static str, idle: bool },
}

pub(crate) fn dispatch(
    db: &Connection,
    claim: &DispatchClaim,
    current: &shep_mail_core::model::Mail,
) -> Result<Dispatch> {
    let saved: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT state,attempt FROM group_items WHERE job=?1 AND position=?2",
            params![claim.job, claim.position],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((state, attempt)) = saved else {
        return Ok(Dispatch::Stop {
            outcome: "cancelled",
            idle: true,
        });
    };
    ensure!(
        state
            == (if claim.inverse {
                "reversing"
            } else {
                "sending"
            })
            && attempt.as_deref() == Some(claim.attempt.as_str()),
        "This group step lost its exact claim before dispatch."
    );
    let (job_state, undo) = job_state(db, &claim.job)?;
    if !claim.inverse && undo {
        finish_item(
            db,
            &claim.job,
            claim.position,
            "cancelled",
            Some("Cancelled before provider dispatch"),
            None,
        )?;
        return Ok(Dispatch::Stop {
            outcome: "cancelled",
            idle: false,
        });
    }
    if job_state == "paused" {
        db.execute(
            "UPDATE group_items SET state=?3,attempt=NULL,fields=NULL WHERE job=?1 AND position=?2",
            params![
                claim.job,
                claim.position,
                if claim.inverse { "undoing" } else { "pending" }
            ],
        )?;
        touch(db, &claim.job)?;
        return Ok(Dispatch::Stop {
            outcome: "deferred",
            idle: true,
        });
    }
    let continuity = if let Some(lineage) = claim.source.lineage.as_deref() {
        crate::operations::observed_lineage_matches(db, &current.id, lineage)?
    } else {
        same_folder(&current.folder, &claim.source.folder)
            && current.remote_id == claim.source.remote_id
    };
    let binding = crate::destinations::get(db, "group", &claim.job, &claim.account)?
        .map_or(Ok(()), |destination| {
            crate::destinations::binding(db, &destination)
        });
    let reason = if current.account_id != claim.account || !continuity {
        Some("The source changed before provider dispatch; skipped")
    } else if binding.is_err() {
        Some("The account connection changed after review; skipped")
    } else {
        None
    };
    if let Some(reason) = reason {
        let outcome = if claim.inverse {
            "undo_skipped"
        } else {
            "skipped"
        };
        finish_item(db, &claim.job, claim.position, outcome, Some(reason), None)?;
        return Ok(Dispatch::Stop {
            outcome,
            idle: false,
        });
    }
    let mut fields = claim.fields.clone();
    for field in ["folder", "unread", "starred"] {
        let newer: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM mail_intents WHERE mail=?1 AND field=?2 AND revision>?3)",
            params![current.id, field, claim.approved],
            |row| row.get(0),
        )?;
        match field {
            "folder"
                if newer
                    || fields
                        .folder
                        .as_deref()
                        .is_some_and(|folder| same_folder(folder, &current.folder)) =>
            {
                fields.folder = None
            }
            "unread" if newer || fields.unread == Some(current.unread) => fields.unread = None,
            "starred" if newer || fields.starred == Some(current.starred) => fields.starred = None,
            _ => {}
        }
    }
    if fields.is_empty() {
        let outcome = if claim.inverse {
            "undo_skipped"
        } else {
            "skipped"
        };
        finish_item(
            db,
            &claim.job,
            claim.position,
            outcome,
            Some("A newer choice or current state replaced these fields; skipped"),
            None,
        )?;
        return Ok(Dispatch::Stop {
            outcome,
            idle: false,
        });
    }
    db.execute(
        "UPDATE group_items SET fields=?3 WHERE job=?1 AND position=?2",
        params![claim.job, claim.position, serde_json::to_string(&fields)?],
    )?;
    touch(db, &claim.job)?;
    Ok(Dispatch::Send(fields))
}
#[derive(Clone, Deserialize, Serialize)]
struct Receipt {
    dispatch: Identity,
    applied: Fields,
    #[serde(default)]
    after: Option<Identity>,
    #[serde(default)]
    warning: Option<String>,
}

struct Claim {
    job: String,
    position: i64,
    mail: String,
    inverse: bool,
    fields: Fields,
    frozen: Identity,
    account: String,
    receipt: Option<Receipt>,
    approved: i64,
}
enum Next {
    Idle,
    Skip {
        job: String,
        position: i64,
        inverse: bool,
        reason: String,
    },
    Claim(Box<Claim>),
}

fn same_folder(a: &str, b: &str) -> bool {
    let normalise = |f: &str| {
        if f.eq_ignore_ascii_case("Inbox") {
            "INBOX".to_owned()
        } else {
            f.to_owned()
        }
    };
    normalise(a) == normalise(b)
}

/// Decide the next step from the durable journal and the current cache
/// without changing either. Newer per-field intent, changed physical identity
/// and already-applied values become distinct skipped outcomes.
struct Queued {
    job: String,
    job_state: String,
    approved: i64,
    job_fields: String,
    position: i64,
    mail: String,
    account: String,
    frozen: Identity,
    receipt: Option<String>,
}
fn next(db: &Connection) -> Result<Next> {
    let row = db
        .query_row(NEXT_ITEM_QUERY, [], |r| {
            Ok(Queued {
                job: r.get(0)?,
                job_state: r.get(1)?,
                approved: r.get(2)?,
                job_fields: r.get(3)?,
                position: r.get(4)?,
                mail: r.get(5)?,
                account: r.get(6)?,
                frozen: Identity {
                    folder: r.get(7)?,
                    remote_id: r.get(8)?,
                    unread: r.get(9)?,
                    starred: r.get(10)?,
                    lineage: r.get(11)?,
                    encoding: None,
                },
                receipt: r.get(12)?,
            })
        })
        .optional()?;
    let Some(Queued {
        job,
        job_state,
        approved,
        job_fields,
        position,
        mail,
        account,
        frozen,
        receipt,
    }) = row
    else {
        return Ok(Next::Idle);
    };
    let Identity {
        folder,
        remote_id,
        unread,
        starred,
        lineage,
        encoding,
    } = frozen;
    let inverse = job_state == "undoing";
    let skip = |reason: &str| {
        Ok(Next::Skip {
            job: job.clone(),
            position,
            inverse,
            reason: reason.to_owned(),
        })
    };
    let receipt: Option<Receipt> = receipt.map(|r| serde_json::from_str(&r)).transpose()?;
    if stored_account(db, &account).is_err() {
        return skip("Account removed from this device");
    }
    let Ok(current) = stored_mail(db, &mail) else {
        return skip("Message is no longer cached");
    };
    let (moved, resolvable): (bool, bool) = db.query_row(
        "SELECT moved,EXISTS(SELECT 1 FROM move_receipts WHERE id=?1) FROM mail WHERE id=?1",
        [&current.id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let expected = if inverse {
        match receipt.as_ref().and_then(|r| r.after.clone()) {
            Some(after) => after,
            None => return skip("No saved receipt identifies the moved message"),
        }
    } else {
        Identity {
            folder: folder.clone(),
            remote_id: remote_id.clone(),
            unread,
            starred,
            lineage: lineage.clone(),
            encoding,
        }
    };
    // An acknowledged move whose destination UID is still unresolved keeps its
    // saved receipt; the shared mutation path recovers that identity before
    // the inverse MOVE, so only the folder is compared here.
    let continuity = if let Some(lineage) = expected.lineage.as_deref() {
        crate::operations::observed_lineage_matches(db, &current.id, lineage)?
            && (!moved || resolvable)
    } else if inverse && moved && resolvable {
        same_folder(&current.folder, &expected.folder)
    } else {
        !moved
            && same_folder(&current.folder, &expected.folder)
            && current.remote_id == expected.remote_id
    };
    if current.account_id != account || !continuity {
        return skip(if inverse {
            "Changed since the group ran; Undo skipped"
        } else {
            "Changed since the review; skipped"
        });
    }
    let mut fields: Fields = if inverse {
        let Some(receipt) = receipt.as_ref() else {
            return skip("No saved receipt to reverse");
        };
        Fields {
            folder: receipt
                .applied
                .folder
                .as_ref()
                .map(|_| receipt.dispatch.folder.clone()),
            unread: receipt.applied.unread.map(|_| receipt.dispatch.unread),
            starred: receipt.applied.starred.map(|_| receipt.dispatch.starred),
        }
    } else {
        serde_json::from_str(&job_fields)?
    };
    let mut statement =
        db.prepare("SELECT field FROM mail_intents WHERE mail=?1 AND revision>?2")?;
    let newer = statement
        .query_map(params![current.id, approved], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let had_fields = !fields.is_empty();
    for field in newer {
        match field.as_str() {
            "folder" => fields.folder = None,
            "unread" => fields.unread = None,
            "starred" => fields.starred = None,
            _ => {}
        }
    }
    if had_fields && fields.is_empty() {
        return skip("A newer change owns this message; skipped");
    }
    if fields
        .folder
        .as_deref()
        .is_some_and(|f| same_folder(f, &current.folder))
    {
        fields.folder = None;
    }
    if fields.unread.is_some_and(|u| u == current.unread) {
        fields.unread = None;
    }
    if fields.starred.is_some_and(|s| s == current.starred) {
        fields.starred = None;
    }
    if fields.is_empty() {
        return skip(if inverse {
            "Already restored"
        } else {
            "Already up to date"
        });
    }
    Ok(Next::Claim(Box::new(Claim {
        job,
        position,
        mail: current.id.clone(),
        inverse,
        fields,
        frozen: Identity {
            folder: current.folder.clone(),
            remote_id: current.remote_id.clone(),
            unread: current.unread,
            starred: current.starred,
            lineage,
            encoding,
        },
        account,
        receipt,
        approved,
    })))
}

fn finish_item(
    db: &Connection,
    job: &str,
    position: i64,
    state: &str,
    reason: Option<&str>,
    receipt: Option<&Receipt>,
) -> Result<()> {
    // A step acknowledged after Undo was requested joins the inverse queue
    // with its actual receipt instead of staying applied.
    let (_, undo) = job_state(db, job)?;
    let state = if state == "done" && undo {
        "undoing"
    } else {
        state
    };
    db.execute(
        "UPDATE group_items SET state=?3,reason=?4,receipt=COALESCE(?5,receipt),attempt=NULL WHERE job=?1 AND position=?2",
        params![job, position, state, reason, receipt.map(serde_json::to_string).transpose()?],
    )?;
    if matches!(state, "uncertain" | "undo_uncertain") {
        db.execute(
            "UPDATE group_jobs SET state='paused' WHERE id=?1 AND state IN ('running','undoing')",
            [job],
        )?;
    }
    settle(db, job)?;
    touch(db, job)?;
    Ok(())
}

async fn step(
    profile: &MobileProfile,
    password: Option<SecretString>,
    credential_slot: Option<String>,
) -> Result<Value> {
    let _owner = profile
        .operations
        .groups
        .clone()
        .try_lock_owned()
        .context("A group action step is already in progress.")?;
    let db = &profile.database;
    let decision = db.read(next).await?;
    let mut claim = match decision {
        Next::Idle => {
            // Settle groups whose last step already finished.
            db.write(|db| {
                let tx = db.transaction()?;
                let ids = tx
                    .prepare("SELECT id FROM group_jobs WHERE state IN ('running','undoing')")?
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for id in ids {
                    settle(&tx, &id)?;
                    touch(&tx, &id)?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
            return Ok(json!({"idle":true}));
        }
        Next::Skip {
            job,
            position,
            inverse,
            reason,
        } => {
            let state = if inverse { "undo_skipped" } else { "skipped" };
            let (id, text) = (job.clone(), reason.clone());
            db.write(move |db| {
                let tx = db.transaction()?;
                let current: String = tx.query_row(
                    "SELECT state FROM group_items WHERE job=?1 AND position=?2",
                    params![id, position],
                    |r| r.get(0),
                )?;
                ensure!(
                    matches!(current.as_str(), "pending" | "undoing"),
                    "This step changed while it was being decided."
                );
                finish_item(&tx, &id, position, state, Some(&text), None)?;
                tx.commit()?;
                Ok(())
            })
            .await?;
            return Ok(
                json!({"stepped":true,"job":job,"position":position,"outcome":state,"reason":reason}),
            );
        }
        Next::Claim(claim) => claim,
    };
    let account = {
        let id = claim.account.clone();
        db.read(move |db| stored_account(db, &id)).await?
    };
    let remote =
        account.protocol == Protocol::Imap && !claim.frozen.remote_id.starts_with("local-");
    let mut prepared_destination = None;
    if !claim.inverse {
        let job = claim.job.clone();
        let action: Action = db
            .read(move |db| {
                Ok(serde_json::from_str(&db.query_row(
                    "SELECT action FROM group_jobs WHERE id=?1",
                    [job],
                    |row| row.get::<_, String>(0),
                )?)?)
            })
            .await?;
        if action.logical_role().is_some() {
            let job = claim.job.clone();
            let source = claim.account.clone();
            let destination = db
                .read(move |db| crate::destinations::get(db, "group", &job, &source))
                .await?;
            let resolution = if let Some(destination) = destination {
                if !remote {
                    db.write(move |db| crate::destinations::local(db, &destination))
                        .await
                        .map(crate::destinations::Resolution::Ready)?
                } else if let Some(cached) = crate::destinations::cached(db, &destination).await? {
                    cached
                } else {
                    let Some(password) = password.clone() else {
                        return Ok(
                            json!({"requires_credentials":account.id,"job":claim.job,"position":claim.position}),
                        );
                    };
                    let (_account, _slot) =
                        profile.operations.destination_capacity(&account.id).await?;
                    let id = account.id.clone();
                    let slot = credential_slot.clone();
                    db.read(move |db| crate::connections::check_binding(db, &id, slot.as_deref()))
                        .await?;
                    crate::destinations::provider(profile, destination, account.clone(), password)
                        .await?
                }
            } else {
                crate::destinations::Resolution::Rejected { message:"This older group has no saved destination or source proof. Undo queued work and review it again.".into() }
            };
            match resolution {
                crate::destinations::Resolution::Ready(target) => {
                    claim.fields.folder = Some(target.name.clone());
                    claim.frozen.encoding = Some(target.encoding);
                    prepared_destination = Some(target);
                }
                crate::destinations::Resolution::Obsolete => return Ok(json!({"idle":true})),
                crate::destinations::Resolution::Waiting { message, .. }
                | crate::destinations::Resolution::Rejected { message } => {
                    let job = claim.job.clone();
                    let warning = message.clone();
                    db.write(move |db| {
                        db.execute("UPDATE group_jobs SET state='paused',error=?2,revision=revision+1 WHERE id=?1 AND state='running' AND undone IS NULL",params![job,warning])?;
                        Ok(())
                    }).await?;
                    return Ok(json!({"idle":true,"destination_error":message}));
                }
            }
        }
    } else if let Some(encoding) = claim
        .receipt
        .as_ref()
        .and_then(|receipt| receipt.dispatch.encoding)
        && let Some(folder) = claim.fields.folder.clone()
    {
        let mut target = shep_mail_core::folders::Mailbox::flat(folder);
        target.encoding = encoding;
        prepared_destination = Some(target);
    }
    if remote && password.is_none() {
        return Ok(
            json!({"requires_credentials":account.id,"job":claim.job,"position":claim.position}),
        );
    }
    let (job, position, mail, inverse, mut fields, frozen) = (
        claim.job.clone(),
        claim.position,
        claim.mail.clone(),
        claim.inverse,
        claim.fields.clone(),
        claim.frozen.clone(),
    );
    let claimed_fields = fields.clone();
    let attempt = uuid::Uuid::new_v4().to_string();
    let attempt_id = attempt.clone();
    let claimed_job = job.clone();
    db.write(move |db| {
        let tx = db.transaction()?;
        let current: String = tx.query_row(
            "SELECT state FROM group_items WHERE job=?1 AND position=?2",
            params![claimed_job, position],
            |r| r.get(0),
        )?;
        ensure!(
            current == if inverse { "undoing" } else { "pending" },
            "This step changed while it was being claimed."
        );
        tx.execute(
            "UPDATE group_items SET state=?3,attempt=?4,fields=?5 WHERE job=?1 AND position=?2",
            params![
                claimed_job,
                position,
                if inverse { "reversing" } else { "sending" },
                attempt_id,
                serde_json::to_string(&claimed_fields)?
            ],
        )?;
        touch(&tx, &claimed_job)?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    let outcome = crate::operations::mutate(
        profile,
        Mutation {
            action_id: format!(
                "group:{job}:{position}:{}",
                if inverse { "undo" } else { "forward" }
            ),
            parent_action: None,
            observed_lineage: None,
            require_observation: false,
            credential_slot,
            id: mail.clone(),
            password,
            folder: fields.folder.clone(),
            unread: fields.unread,
            starred: fields.starred,
            intent: false,
            report: false,
            logical_role: None,
            prepared_destination,
            group_claim: Some(DispatchClaim {
                job: job.clone(),
                position,
                attempt: attempt.clone(),
                account: claim.account.clone(),
                inverse,
                approved: claim.approved,
                fields: fields.clone(),
                source: frozen.clone(),
            }),
        },
    )
    .await;
    if let Ok(value) = &outcome {
        if let Some(outcome) = value.get("group_outcome").and_then(Value::as_str) {
            return Ok(
                json!({"stepped":value["idle"]!=true,"idle":value["idle"]==true,"job":job,"position":position,"outcome":outcome}),
            );
        }
        if let Some(accepted) = value
            .get("accepted_fields")
            .filter(|value| !value.is_null())
        {
            fields = serde_json::from_value(accepted.clone())?;
        }
    }
    let (state, reason, warning) = match &outcome {
        Ok(value) if value.get("requires_credentials").is_some() => (
            if inverse { "undo_failed" } else { "failed" },
            Some("The account credential was not available. Retry after reconnecting.".to_owned()),
            None,
        ),
        Ok(value) => (
            if inverse { "undone" } else { "done" },
            None,
            value
                .get("warning")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ),
        Err(error) => {
            let text = error.to_string();
            let flags_rejected = error
                .chain()
                .any(|cause| cause.is::<shep_mail_core::mail_actions::FlagsRejected>());
            let move_refused = shep_mail_core::mail_actions::classify_move_failure(error)
                == shep_mail_core::mail_actions::MoveFailure::Refused;
            let provider_started = !text.contains("no provider operation was started");
            let ambiguous = provider_started
                && ((fields.folder.is_some() && !move_refused)
                    || ((fields.unread.is_some() || fields.starred.is_some()) && !flags_rejected));
            if ambiguous {
                (
                    if inverse {
                        "undo_uncertain"
                    } else {
                        "uncertain"
                    },
                    Some(format!("{text} Shep will not repeat this move.")),
                    None,
                )
            } else {
                (
                    if inverse { "undo_failed" } else { "failed" },
                    Some(text),
                    None,
                )
            }
        }
    };
    let (job_id, mail_id, state_text, reason_text, saved_warning) = (
        job.clone(),
        mail.clone(),
        state.to_owned(),
        reason.clone(),
        warning.clone(),
    );
    let previous = claim.receipt.clone();
    db.write(move |db| {
        let warning = saved_warning;
        let tx = db.transaction()?;
        let (current, held): (String, Option<String>) = tx.query_row(
            "SELECT state,attempt FROM group_items WHERE job=?1 AND position=?2",
            params![job_id, position],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            held.as_deref() == Some(attempt.as_str())
                && current == if inverse { "reversing" } else { "sending" },
            "This step lost its claim before its receipt was saved."
        );
        let after = stored_mail(&tx, &mail_id)
            .ok()
            .map(|m| {
                let lineage = if frozen.lineage.is_some() {
                    tx.query_row(
                        "SELECT token FROM mail_lineage WHERE id=?1",
                        [&m.id],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?
                } else {
                    None
                };
                Ok::<_, anyhow::Error>(Identity {
                    folder: m.folder,
                    remote_id: m.remote_id,
                    unread: m.unread,
                    starred: m.starred,
                    lineage,
                    encoding: frozen.encoding,
                })
            })
            .transpose()?;
        let receipt = if inverse {
            previous.map(|mut r| {
                r.warning = warning.clone();
                r.after = after.clone();
                r
            })
        } else {
            Some(Receipt {
                dispatch: frozen.clone(),
                applied: fields.clone(),
                after: after.clone(),
                warning: warning.clone(),
            })
        };
        let text = reason_text.as_deref().or(warning.as_deref());
        finish_item(&tx, &job_id, position, &state_text, text, receipt.as_ref())?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(
        json!({"stepped":true,"job":job,"position":position,"outcome":state,"reason":reason.or(warning),"mail":mail}),
    )
}
