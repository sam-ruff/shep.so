//! Durable group actions: a frozen review is staged from the captured
//! selection into `group_jobs`/`group_items`, then executed one owned step at
//! a time through the existing per-message mutation and receipt code. The
//! journal keeps metadata and physical identities only, never MIME or secrets.
use crate::api::MobileProfile;
use crate::operations::{stored_account, stored_mail};
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
pub(crate) const ATTENTION_COUNT_QUERY: &str = "SELECT COUNT(*) FROM group_items INDEXED BY group_item_attention WHERE state IN ('failed','uncertain','undo_failed','undo_uncertain','repair','undo_repair')";
pub(crate) const ATTENTION_TARGET_QUERY: &str = "SELECT job FROM group_items INDEXED BY group_item_attention WHERE state IN ('failed','uncertain','undo_failed','undo_uncertain','repair','undo_repair') LIMIT 1";
pub(crate) const NEXT_ITEM_QUERY: &str = "SELECT j.id,j.state,COALESCE(j.approved,0),j.fields,i.position,i.mail,i.account,i.folder,i.remote_id,i.unread,i.starred,i.receipt,i.lineage FROM group_jobs j INDEXED BY group_job_state CROSS JOIN group_items i ON i.job=j.id AND i.position=(SELECT position FROM group_items INDEXED BY group_item_state WHERE job=j.id AND state=CASE j.state WHEN 'running' THEN 'pending' ELSE 'undoing' END ORDER BY position LIMIT 1) WHERE j.state IN ('running','undoing') ORDER BY j.seq LIMIT 1";

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
        "SELECT COUNT(*) FROM group_items WHERE job=?1 AND state IN ('pending','sending','undoing','reversing','repair','undo_repair')",
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

/// Exclusive ownership at open permits receipt repair, never mutation replay.
pub(crate) fn restart(db: &Connection) -> Result<()> {
    db.execute("UPDATE group_items SET state='repair',reason='A saved acknowledgement needs local repair before another group step.' WHERE state='sending' AND EXISTS(SELECT 1 FROM individual_mail_action_receipts r WHERE r.action=group_items.attempt)",[])?;
    db.execute("UPDATE group_items SET state='undo_repair',reason='A saved Undo acknowledgement needs local repair before another group step.' WHERE state='reversing' AND EXISTS(SELECT 1 FROM individual_mail_action_receipts r WHERE r.action=group_items.attempt)",[])?;
    // A child that never left queued or waiting was not sent; requeue its item.
    db.execute("UPDATE individual_mail_actions SET status='cancelled',error=NULL WHERE group_job IS NOT NULL AND status IN ('queued','waiting') AND id IN (SELECT attempt FROM group_items WHERE state IN ('sending','reversing'))",[])?;
    db.execute("UPDATE group_items SET state=CASE state WHEN 'sending' THEN 'pending' ELSE 'undoing' END,attempt=NULL WHERE state IN ('sending','reversing') AND EXISTS(SELECT 1 FROM individual_mail_actions a WHERE a.id=group_items.attempt AND a.status='cancelled' AND NOT EXISTS(SELECT 1 FROM individual_mail_action_receipts r WHERE r.action=a.id))",[])?;
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
        let tx = db.unchecked_transaction()?;
        let swept = sweep_one(&tx)?;
        tx.commit()?;
        if !swept {
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
    if crate::operations::group::retire(db, id)? {
        return Ok(false);
    }
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
                tx.execute("UPDATE individual_mail_actions SET status='cancelled',error=NULL WHERE group_job=?1 AND status IN ('queued','waiting')",[&id])?;
                tx.execute("UPDATE group_items SET state=CASE state WHEN 'sending' THEN 'pending' ELSE 'undoing' END,attempt=NULL WHERE job=?1 AND state IN ('sending','reversing') AND EXISTS(SELECT 1 FROM individual_mail_actions a WHERE a.id=group_items.attempt AND a.group_job=?1 AND a.status='cancelled')",[&id])?;
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
                tx.execute("UPDATE individual_mail_actions SET status='cancelled',error=NULL WHERE group_job=?1 AND group_inverse=0 AND status IN ('queued','waiting')",[&id])?;
                tx.execute("UPDATE group_items SET state='cancelled',attempt=NULL,reason='Cancelled before sending' WHERE job=?1 AND state='sending' AND EXISTS(SELECT 1 FROM individual_mail_actions a WHERE a.id=group_items.attempt AND a.status='cancelled')",[&id])?;
                tx.execute("UPDATE group_items SET state='cancelled',reason='Cancelled before sending' WHERE job=?1 AND state='pending'",[&id])?;
                tx.execute("UPDATE group_items SET state='undoing',reason=NULL WHERE job=?1 AND state='done'",[&id])?;
                // An accepted unsaved acknowledgement is applied through the
                // checked repair first, then reversed with its proven identity.
                tx.execute("UPDATE group_items SET state='repair',reason='Undo saves this acknowledged change locally before reversing it.' WHERE job=?1 AND state='accepted' AND EXISTS(SELECT 1 FROM individual_mail_actions a WHERE a.id=group_items.attempt AND a.group_job=?1 AND a.group_inverse=0 AND a.status='repair')",[&id])?;
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
                    "repair" | "undo_repair" => {
                        tx.execute("UPDATE group_jobs SET state=CASE WHEN undone IS NOT NULL THEN 'undoing' ELSE 'running' END,revision=revision+1 WHERE id=?1",[&id])?;
                        let value=summary(&tx,&id)?;
                        tx.commit()?;
                        return Ok(value);
                    }
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
                let (_, undo) = job_state(&tx, &id)?;
                let item: String = tx
                    .query_row(
                        "SELECT state FROM group_items WHERE job=?1 AND position=?2",
                        params![id, position],
                        |r| r.get(0),
                    )
                    .context("This message is no longer part of the group action.")?;
                // Retires the local intent only. The cache and the provider
                // outcome are not classified; the message keeps whatever a
                // later refresh shows. Saved acknowledgements stay recorded.
                let reason = match item.as_str() {
                    "uncertain" | "undo_uncertain" => "Current state accepted without a server confirmation",
                    "repair" if undo => "Accepted without Undo. This message stays where the server put it; refresh the folder to see it.",
                    "repair" | "undo_repair" => "Current state accepted. The server acknowledgement is kept, but this device did not save it; refresh the folder to see the result.",
                    _ => anyhow::bail!("Only an unconfirmed or unsaved step can be accepted."),
                };
                tx.execute(
                    "UPDATE group_items SET state='accepted',reason=?3 WHERE job=?1 AND position=?2",
                    params![id, position, reason],
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
        sweep_one(&tx)?;
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
                for row in rows {
                    let position = row["position"].as_i64().context("Invalid selection page.")?;
                    let mail_id = row["id"].as_str().context("Invalid selection page.")?;
                    let (account, folder, unread, starred) = (
                        row["account"].as_str().unwrap_or_default(),
                        row["folder"].as_str().unwrap_or_default(),
                        row["unread"].as_bool().unwrap_or_default(),
                        row["starred"].as_bool().unwrap_or_default(),
                    );
                    let lineage = row["lineage"].as_str();
                    let current = stored_mail(&tx, mail_id).ok();
                    let current = match (current, lineage) {
                        (Some(mail), Some(proof)) if crate::operations::observed_lineage_matches(&tx, &mail.id, proof)? => Some(mail),
                        _ => None,
                    };
                    match current {
                        Some(mail) => tx.execute(
                            "INSERT INTO group_items(job,position,mail,account,folder,remote_id,unread,starred,state,lineage,connection) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'pending',?9,?10)",
                            params![job,position,mail.id,mail.account_id,mail.folder,mail.remote_id,mail.unread,mail.starred,lineage,crate::operations::group::connection(&tx,&mail.account_id)?],
                        )?,
                        None => tx.execute(
                            "INSERT INTO group_items(job,position,mail,account,folder,remote_id,unread,starred,state,reason,lineage) VALUES(?1,?2,?3,?4,?5,'',?6,?7,'skipped','Message changed since the selection was captured',?8)",
                            params![job,position,mail_id,account,folder,unread,starred,lineage],
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
    let runnable: bool = crate::operations::group::pending(db)?.is_some()
        || db.query_row(
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
pub(crate) struct Identity {
    pub folder: String,
    pub remote_id: String,
    pub unread: bool,
    pub starred: bool,
    #[serde(default)]
    pub lineage: Option<String>,
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
    /// No provider work was started; the step stays retryable.
    Defer {
        job: String,
        position: i64,
        inverse: bool,
        reason: String,
    },
    Claim(Box<Claim>),
}

/// A started, acknowledged-but-unsaved or unknown change to this field.
fn unsettled(db: &Connection, mail: &str, field: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM individual_mail_actions INDEXED BY individual_mail_action_unsettled WHERE mail=?1 AND status IN ('running','repair','uncertain') AND json_extract(COALESCE(accepted_fields,fields),?2) IS NOT NULL)",
        params![mail, format!("$.{field}")],
        |row| row.get(0),
    )?)
}

/// A cached flag proves the server value only when its last cache completion
/// is known and no change to it is unsettled.
fn cache_proves(db: &Connection, mail: &str, field: &str) -> Result<bool> {
    let legacy: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM mail_intents WHERE mail=?1 AND field=?2 AND legacy_revision>applied_revision)",
        params![mail, field],
        |row| row.get(0),
    )?;
    Ok(!legacy && !unsettled(db, mail, field)?)
}

/// Why the cached folder and UID cannot be trusted yet: a move is in flight
/// or its pending-move row is unsaved. A complete listing that still shows the
/// source clears that row, so an unconfirmed move never blocks indefinitely.
fn move_blocker(db: &Connection, mail: &str) -> Result<Option<&'static str>> {
    let (pending, running): (bool, bool) = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pending_moves WHERE id=?1),EXISTS(SELECT 1 FROM individual_mail_actions INDEXED BY individual_mail_action_unsettled WHERE mail=?1 AND status IN ('running','repair','uncertain') AND status='running' AND json_extract(COALESCE(accepted_fields,fields),'$.folder') IS NOT NULL)",
        [mail],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if !pending && !running {
        return Ok(None);
    }
    let group: Option<bool> = db
        .query_row(
            "SELECT group_job IS NOT NULL FROM individual_mail_actions INDEXED BY individual_mail_action_unsettled WHERE mail=?1 AND status IN ('running','repair','uncertain') AND json_extract(COALESCE(accepted_fields,fields),'$.folder') IS NOT NULL ORDER BY status='running' DESC,created DESC LIMIT 1",
            [mail],
            |row| row.get(0),
        )
        .optional()?;
    Ok(Some(match group {
        Some(true) => {
            "An earlier group step that moves this message is not saved locally yet. Check History or refresh its folders, then retry."
        }
        Some(false) => {
            "An earlier move of this message is not confirmed or saved locally yet. Check Activity or refresh its folders, then retry."
        }
        None => {
            "An earlier move of this message is not saved locally yet. Refresh its folders, then retry."
        }
    }))
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

/// Pending group approvals own fields before their first provider step. The
/// active-job index bounds this lookup independently of completed history.
pub(crate) fn field_owned(
    db: &Connection,
    job: &str,
    mail: &str,
    field: &str,
    approved: i64,
) -> Result<bool> {
    ensure!(
        ["folder", "unread", "starred"].contains(&field),
        "Invalid group field."
    );
    let individual: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM mail_intents WHERE mail=?1 AND field=?2 AND ((revision>?3 AND revision!=COALESCE((SELECT undone FROM group_jobs WHERE id=?4),-1)) OR (applied_revision>?3 AND applied_revision!=COALESCE((SELECT undone FROM group_jobs WHERE id=?4),-1)) OR legacy_revision>?3))",
        params![mail, field, approved,job],
        |row| row.get(0),
    )?;
    if individual {
        return Ok(false);
    }
    no_newer_group(db, job, mail, field, approved)
}

pub(crate) fn no_newer_group(
    db: &Connection,
    job: &str,
    mail: &str,
    field: &str,
    approved: i64,
) -> Result<bool> {
    ensure!(
        ["folder", "unread", "starred"].contains(&field),
        "Invalid group field."
    );
    let path = format!("$.{field}");
    let newer: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM group_jobs j INDEXED BY group_job_state CROSS JOIN group_items i INDEXED BY group_item_origin WHERE j.state IN ('running','undoing','paused') AND j.id!=?1 AND j.approved>?4 AND i.job=j.id AND i.lineage IN (SELECT token FROM mail_lineage WHERE id=?2 UNION SELECT a.source FROM mail_lineage_aliases a JOIN mail_lineage l ON l.token=a.target WHERE l.id=?2) AND json_extract(j.fields,?3) IS NOT NULL AND i.state NOT IN ('cancelled','accepted','undo_skipped','undone'))",
        params![job,mail,path,approved], |row|row.get(0))?;
    Ok(!newer)
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
                    lineage: r.get(12)?,
                },
                receipt: r.get(11)?,
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
    let Some(origin) = lineage.as_deref() else {
        return skip("This older review has no captured source proof. Select the messages again");
    };
    let connection: Option<String> = db.query_row(
        "SELECT connection FROM group_items WHERE job=?1 AND position=?2",
        params![job, position],
        |row| row.get(0),
    )?;
    if connection.as_deref() != Some(crate::operations::group::connection(db, &account)?.as_str()) {
        return skip("Connection changed since the review. Select the messages again");
    }
    let Ok(current) = stored_mail(db, &mail) else {
        return skip("Message is no longer cached");
    };
    if !crate::operations::observed_lineage_matches(db, &current.id, origin)? {
        return skip("Message changed since the selection was captured");
    }
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
    let had_fields = !fields.is_empty();
    for (field, requested) in [
        ("folder", fields.folder.is_some()),
        ("unread", fields.unread.is_some()),
        ("starred", fields.starred.is_some()),
    ] {
        if !requested || field_owned(db, &job, &current.id, field, approved)? {
            continue;
        }
        match field {
            "folder" => fields.folder = None,
            "unread" => fields.unread = None,
            "starred" => fields.starred = None,
            _ => {}
        }
    }
    if had_fields && fields.is_empty() {
        return skip("A newer change owns this message; skipped");
    }
    if let Some(reason) = move_blocker(db, &current.id)? {
        return Ok(Next::Defer {
            job,
            position,
            inverse,
            reason: reason.into(),
        });
    }
    if fields
        .folder
        .as_deref()
        .is_some_and(|f| same_folder(f, &current.folder))
    {
        fields.folder = None;
    }
    if fields.unread.is_some_and(|u| u == current.unread)
        && cache_proves(db, &current.id, "unread")?
    {
        fields.unread = None;
    }
    if fields.starred.is_some_and(|s| s == current.starred)
        && cache_proves(db, &current.id, "starred")?
    {
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
        },
        account,
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

fn reserve_applied_choice(db: &Connection, job: &str, position: i64) -> Result<()> {
    let (mail,lineage,fields,approved):(String,Option<String>,String,i64)=db.query_row("SELECT i.mail,i.lineage,j.fields,j.approved FROM group_items i JOIN group_jobs j ON j.id=i.job WHERE i.job=?1 AND i.position=?2",params![job,position],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
    let Some(lineage) = lineage else {
        return Ok(());
    };
    let Ok(current) = stored_mail(db, &mail) else {
        return Ok(());
    };
    if !crate::operations::observed_lineage_matches(db, &current.id, &lineage)? {
        return Ok(());
    }
    let fields: Fields = serde_json::from_str(&fields)?;
    for (field, applied) in [
        (
            "folder",
            fields
                .folder
                .as_deref()
                .is_some_and(|folder| same_folder(folder, &current.folder)),
        ),
        ("unread", fields.unread == Some(current.unread)),
        ("starred", fields.starred == Some(current.starred)),
    ] {
        let proven = if field == "folder" {
            move_blocker(db, &current.id)?.is_none()
        } else {
            cache_proves(db, &current.id, field)?
        };
        if applied && proven && field_owned(db, job, &current.id, field, approved)? {
            db.execute("INSERT INTO mail_intents(mail,field,revision) VALUES(?1,?2,?3) ON CONFLICT(mail,field) DO UPDATE SET revision=excluded.revision WHERE mail_intents.revision<=excluded.revision",params![current.id,field,approved])?;
            crate::operations::record_applied(db, &current.id, field, approved)?;
        }
    }
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
    if let Some(attempt) = db.read(crate::operations::group::pending).await? {
        return repair_attempt(profile, attempt, password, credential_slot).await;
    }
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
            return close_unclaimed(db, job, position, inverse, reason, state).await;
        }
        Next::Defer {
            job,
            position,
            inverse,
            reason,
        } => {
            let state = if inverse { "undo_failed" } else { "failed" };
            return close_unclaimed(db, job, position, inverse, reason, state).await;
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
    let claimed_mail = mail.clone();
    let lineage = frozen
        .lineage
        .clone()
        .context("This review has no captured source proof.")?;
    let approved = claim.approved;
    let slot = credential_slot.clone();
    let admitted = db
        .write(move |db| {
            let tx = db.transaction()?;
            let (state, _) = job_state(&tx, &claimed_job)?;
            if state != if inverse { "undoing" } else { "running" } {
                return Ok(false);
            }
            let current: String = tx.query_row(
                "SELECT state FROM group_items WHERE job=?1 AND position=?2",
                params![claimed_job, position],
                |r| r.get(0),
            )?;
            ensure!(
                current == if inverse { "undoing" } else { "pending" },
                "This step changed while it was being claimed."
            );
            let accepted = crate::operations::group::admit(
                &tx,
                crate::operations::group::Admission {
                    job: &claimed_job,
                    position,
                    inverse,
                    attempt: &attempt_id,
                    mail: &claimed_mail,
                    lineage: &lineage,
                    approved,
                    fields: &serde_json::to_value(&claimed_fields)?,
                    credential_slot: slot.as_deref(),
                },
            )?;
            tx.execute(
                "UPDATE group_items SET state=?3,attempt=?4,fields=?5 WHERE job=?1 AND position=?2",
                params![
                    claimed_job,
                    position,
                    if inverse { "reversing" } else { "sending" },
                    attempt_id,
                    serde_json::to_string(&accepted)?
                ],
            )?;
            touch(&tx, &claimed_job)?;
            tx.commit()?;
            Ok(true)
        })
        .await?;
    if !admitted {
        return Ok(json!({"idle":true,"stepped":false}));
    }
    let outcome = crate::operations::group::dispatch(profile, attempt.clone(), password).await;
    let failure = outcome.err().map(|error| error.to_string());
    let saved_attempt = attempt.clone();
    let result = db
        .write(move |db| finish_attempt(db, &saved_attempt, failure.as_deref()))
        .await?;
    let reason_job = job.clone();
    let reason: Option<String> = db
        .read(move |db| {
            Ok(db.query_row(
                "SELECT reason FROM group_items WHERE job=?1 AND position=?2",
                params![reason_job, position],
                |row| row.get(0),
            )?)
        })
        .await?;
    Ok(
        json!({"stepped":true,"job":job,"position":position,"outcome":result,"reason":reason,"mail":mail}),
    )
}

/// Close a step that needs no provider call: a skip, or a retryable failure
/// whose reason says what to do first.
async fn close_unclaimed(
    db: &crate::database::Database,
    job: String,
    position: i64,
    inverse: bool,
    reason: String,
    state: &'static str,
) -> Result<Value> {
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
        if !inverse && text == "Already up to date" {
            reserve_applied_choice(&tx, &id, position)?;
        }
        finish_item(&tx, &id, position, state, Some(&text), None)?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(json!({"stepped":true,"job":job,"position":position,"outcome":state,"reason":reason}))
}

async fn repair_attempt(
    profile: &MobileProfile,
    attempt: String,
    password: Option<SecretString>,
    credential_slot: Option<String>,
) -> Result<Value> {
    let lookup = attempt.clone();
    let saved = profile
        .database
        .read(move |db| crate::operations::group::snapshot(db, &lookup))
        .await?;
    let lookup = attempt.clone();
    let repaired = if saved.status != "repair" {
        // Only a saved acknowledgement needs local work; other results are
        // recorded on the item as they stand.
        Ok(crate::operations::ReceiptApplication::Complete)
    } else {
        profile
            .database
            .write(move |db| crate::operations::group::repair(db, &lookup))
            .await
    };
    let mut failure = None;
    match repaired {
        Ok(crate::operations::ReceiptApplication::NeedsInspection) => {
            let Some(password) = password else {
                let lookup = attempt.clone();
                let account: String = profile
                    .database
                    .read(move |db| {
                        Ok(db.query_row(
                            "SELECT account FROM individual_mail_actions WHERE id=?1",
                            [lookup],
                            |row| row.get(0),
                        )?)
                    })
                    .await?;
                return Ok(
                    json!({"requires_credentials":account,"job":saved.job,"position":saved.position}),
                );
            };
            if let Err(error) = crate::operations::group::inspect(
                profile,
                attempt.clone(),
                password,
                credential_slot,
            )
            .await
            {
                failure = Some(error.to_string());
            }
            if failure.is_none() {
                let lookup = attempt.clone();
                if let Err(error) = profile
                    .database
                    .write(move |db| crate::operations::group::repair(db, &lookup))
                    .await
                {
                    failure = Some(error.to_string());
                }
            }
        }
        Err(error) => failure = Some(error.to_string()),
        _ => {}
    }
    let result = profile
        .database
        .write(move |db| finish_attempt(db, &attempt, failure.as_deref()))
        .await?;
    let blocked = matches!(result.as_str(), "repair" | "undo_repair");
    Ok(
        json!({"stepped":!blocked,"idle":blocked,"job":saved.job,"position":saved.position,"outcome":result,"repaired":!blocked}),
    )
}

fn finish_attempt(db: &mut Connection, attempt: &str, failure: Option<&str>) -> Result<String> {
    let tx = db.transaction()?;
    let saved = crate::operations::group::snapshot(&tx, attempt)?;
    let (current, held, previous): (String, Option<String>, Option<String>) = tx.query_row(
        "SELECT state,attempt,receipt FROM group_items WHERE job=?1 AND position=?2",
        params![saved.job, saved.position],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if current == "cancelled" && saved.status == "cancelled" {
        return Ok(current);
    }
    if saved.status == "cancelled"
        && held.is_none()
        && matches!(current.as_str(), "pending" | "undoing")
    {
        return Ok("paused".into());
    }
    ensure!(
        held.as_deref() == Some(attempt),
        "This group attempt no longer owns its item."
    );
    let state = match saved.status.as_str() {
        "succeeded" => {
            if saved.inverse {
                "undone"
            } else {
                "done"
            }
        }
        "repair" => {
            if saved.inverse {
                "undo_repair"
            } else {
                "repair"
            }
        }
        "uncertain" | "running" => {
            if saved.inverse {
                "undo_uncertain"
            } else {
                "uncertain"
            }
        }
        "cancelled" => {
            if saved.inverse {
                "undo_skipped"
            } else {
                "skipped"
            }
        }
        _ => {
            if saved.inverse {
                "undo_failed"
            } else {
                "failed"
            }
        }
    };
    // Child errors are fixed public text; provider replies never reach here.
    let child_error: Option<String> = tx.query_row(
        "SELECT error FROM individual_mail_actions WHERE id=?1",
        [attempt],
        |row| row.get(0),
    )?;
    if saved.status == "waiting" {
        // Nothing was sent; a later Retry admits a fresh attempt.
        tx.execute(
            "UPDATE individual_mail_actions SET status='cancelled' WHERE id=?1 AND status='waiting'",
            [attempt],
        )?;
    }
    let reason = match state {
        "repair" | "undo_repair" if failure.is_some() => Some("The server acknowledgement is saved, but this device could not save it. Refresh the folder and retry, or accept the current state.".into()),
        "repair" | "undo_repair" => Some("The provider acknowledgement is saved. Retry to finish saving it locally before another group step.".into()),
        "uncertain" | "undo_uncertain" => Some("The provider result is unknown. Check the folder; Shep will not repeat this attempt.".into()),
        "skipped" | "undo_skipped" => Some(child_error.unwrap_or_else(|| "A newer choice owns this message".into())),
        "failed" | "undo_failed" if saved.status == "waiting" => child_error,
        "failed" | "undo_failed" if failure.is_some() => Some("The operation could not be completed. Review the saved connection and try again.".into()),
        _ => None,
    };
    let receipt = if saved.acknowledged {
        if saved.inverse {
            previous
                .map(|raw| serde_json::from_str::<Receipt>(&raw))
                .transpose()?
                .map(|mut original| {
                    original.after = saved.after.clone();
                    original.warning = reason.clone();
                    original
                })
        } else {
            Some(Receipt {
                dispatch: saved.dispatch,
                applied: saved.fields,
                after: saved.after,
                warning: reason.clone(),
            })
        }
    } else {
        None
    };
    if state == "repair" || state == "undo_repair" {
        tx.execute("UPDATE group_items SET state=?3,reason=?4,receipt=COALESCE(?5,receipt) WHERE job=?1 AND position=?2",
            params![saved.job,saved.position,state,reason,receipt.as_ref().map(serde_json::to_string).transpose()?])?;
        tx.execute(
            "UPDATE group_jobs SET state='paused' WHERE id=?1 AND state IN ('running','undoing')",
            [&saved.job],
        )?;
        touch(&tx, &saved.job)?;
    } else {
        finish_item(
            &tx,
            &saved.job,
            saved.position,
            state,
            reason.as_deref(),
            receipt.as_ref(),
        )?;
    }
    tx.commit()?;
    Ok(state.into())
}
