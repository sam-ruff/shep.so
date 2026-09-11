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

/// History keeps this many groups; older finished groups retire first.
pub const HISTORY_JOBS: i64 = 20;
/// Items are staged, listed and retired in pages of this size.
pub const PAGE: usize = 50;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Archive,
    Delete,
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
    History,
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
                ensure!(!undo, "This group action is already being undone.");
                ensure!(
                    matches!(state.as_str(), "running" | "paused" | "finished"),
                    "This group action cannot be undone from its current state."
                );
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
        Command::History => db.read(history).await,
        Command::Items { id, after } => db.read(move |db| items(db, &id, after)).await,
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
        let jobs: i64 = tx.query_row("SELECT COUNT(*) FROM group_jobs", [], |r| r.get(0))?;
        if jobs >= HISTORY_JOBS {
            let oldest: Option<String> = tx.query_row("SELECT id FROM group_jobs WHERE state='finished' ORDER BY seq LIMIT 1",[],|r|r.get(0)).optional()?;
            if let Some(oldest) = oldest {
                while !retire_page(&tx, &oldest)? {}
            }
        }
        let jobs: i64 = tx.query_row("SELECT COUNT(*) FROM group_jobs", [], |r| r.get(0))?;
        ensure!(jobs < HISTORY_JOBS, "History keeps {HISTORY_JOBS} group actions. Finish or remove older ones first.");
        tx.execute("INSERT INTO group_jobs(id,action,fields,state,scope,created) VALUES(?1,?2,?3,'staging',?4,?5)",params![job,action_json,fields_json,scope_json,now()])?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    let staged = stage(profile, &id, &selection, expected).await;
    let job = id.clone();
    let release = selection.clone();
    let _ = db
        .selection(move |db| {
            crate::selection::run(
                db,
                crate::selection::Command::Release { id: release },
                vec![],
            )
        })
        .await;
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
                        Some(mail) => tx.execute(
                            "INSERT INTO group_items(job,position,mail,account,folder,remote_id,unread,starred,state) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending')",
                            params![job,position,mail.id,mail.account_id,mail.folder,mail.remote_id,mail.unread,mail.starred],
                        )?,
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
fn history(db: &Connection) -> Result<Value> {
    let ids = db
        .prepare("SELECT id FROM group_jobs WHERE state NOT IN ('cancelled','interrupted') ORDER BY seq DESC LIMIT ?1")?
        .query_map([HISTORY_JOBS], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let jobs = ids
        .iter()
        .map(|id| summary(db, id))
        .collect::<Result<Vec<_>>>()?;
    let runnable: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_jobs j WHERE (j.state='running' AND EXISTS(SELECT 1 FROM group_items WHERE job=j.id AND state='pending')) OR (j.state='undoing' AND EXISTS(SELECT 1 FROM group_items WHERE job=j.id AND state='undoing')))",
        [],
        |r| r.get(0),
    )?;
    Ok(json!({"jobs":jobs,"runnable":runnable}))
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
    let next_after = (rows.len() == PAGE).then(|| {
        rows.last()
            .map(|r| r["position"].clone())
            .unwrap_or(Value::Null)
    });
    Ok(json!({"rows":rows,"next_after":next_after}))
}

#[derive(Clone, Deserialize, Serialize)]
struct Identity {
    folder: String,
    remote_id: String,
    unread: bool,
    starred: bool,
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
    let row = db.query_row(
        "SELECT j.id,j.state,COALESCE(CASE WHEN j.state='undoing' THEN j.undone ELSE j.approved END,0),j.fields,i.position,i.mail,i.account,i.folder,i.remote_id,i.unread,i.starred,i.receipt FROM group_jobs j JOIN group_items i ON i.job=j.id WHERE (j.state='running' AND i.state='pending') OR (j.state='undoing' AND i.state='undoing') ORDER BY j.seq,i.position LIMIT 1",
        [],
        |r| Ok(Queued {
            job: r.get(0)?,
            job_state: r.get(1)?,
            approved: r.get(2)?,
            job_fields: r.get(3)?,
            position: r.get(4)?,
            mail: r.get(5)?,
            account: r.get(6)?,
            frozen: Identity { folder: r.get(7)?, remote_id: r.get(8)?, unread: r.get(9)?, starred: r.get(10)? },
            receipt: r.get(11)?,
        }),
    ).optional()?;
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
        }
    };
    // An acknowledged move whose destination UID is still unresolved keeps its
    // saved receipt; the shared mutation path recovers that identity before
    // the inverse MOVE, so only the folder is compared here.
    let continuity = if inverse && moved && resolvable {
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
        },
        account,
        receipt,
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
    let claim = match decision {
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
    if remote && password.is_none() {
        return Ok(
            json!({"requires_credentials":account.id,"job":claim.job,"position":claim.position}),
        );
    }
    let (job, position, mail, inverse, fields, frozen) = (
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
            credential_slot,
            id: mail.clone(),
            password,
            folder: fields.folder.clone(),
            unread: fields.unread,
            starred: fields.starred,
            intent: false,
        },
    )
    .await;
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
            let ambiguous = fields.folder.is_some()
                && (text.contains("did not confirm")
                    || text.contains("timed out")
                    || text.contains("acknowledged"));
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
        let after = stored_mail(&tx, &mail_id).ok().map(|m| Identity {
            folder: m.folder,
            remote_id: m.remote_id,
            unread: m.unread,
            starred: m.starred,
        });
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
