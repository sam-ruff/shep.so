use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
#[cfg(test)]
use shep_calendar_core::ProviderFailure;
#[cfg(test)]
use shep_calendar_core::http::MockCalendarProvider;
pub use shep_calendar_core::http::{CalendarProvider, GoogleCalendarProvider};
use shep_calendar_core::{Event, FailureKind, Mutation, Receipt, Source};

const HISTORY_LIMIT: u32 = 50;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct ClaimRejected(&'static str);

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct ClaimWaiting(&'static str);

#[derive(Serialize)]
pub struct Activity {
    id: String,
    status: String,
    error: Option<String>,
    created: i64,
    subject: Option<String>,
    mutation: Mutation,
    receipt: Option<Receipt>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CalendarAdmission {
    pub id: String,
    pub status: String,
    pub mutation: Mutation,
    pub subject: String,
    pub receipt: Option<Receipt>,
}

pub fn action_admission(db: &Connection, id: &str) -> Result<Option<CalendarAdmission>> {
    let saved: Option<(String, String, Option<String>, Option<String>)> = db
        .query_row(
            "SELECT a.status,COALESCE(a.admission_request,a.mutation),a.subject,r.receipt FROM calendar_actions a LEFT JOIN calendar_action_receipts r ON r.action=a.id WHERE a.id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((status, request, subject, receipt)) = saved else {
        return Ok(None);
    };
    let subject = subject.context("This legacy calendar request has no authenticated owner.")?;
    Ok(Some(CalendarAdmission {
        id: id.to_owned(),
        status,
        mutation: serde_json::from_str(&request)?,
        subject,
        receipt: receipt
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
    }))
}

pub fn admit_bound(
    db: &mut Connection,
    id: &str,
    mutation: &Mutation,
    subject: &str,
) -> Result<CalendarAdmission> {
    let tx = db.transaction()?;
    anyhow::ensure!(
        !subject.is_empty() && subject.len() <= 255,
        "Reconnect Google Calendar before changing events."
    );
    if let Some(saved) = action_admission(&tx, id)? {
        anyhow::ensure!(
            saved.mutation == *mutation && saved.subject == subject,
            "This calendar request identity already belongs to another change."
        );
        tx.commit()?;
        return Ok(saved);
    }
    let bound: String = tx
        .query_row(
            "SELECT subject FROM calendar_binding WHERE id=1",
            [],
            |row| row.get(0),
        )
        .context("Sync Google Calendar before changing events.")?;
    anyhow::ensure!(
        bound == subject,
        "Google Calendar changed accounts. Refresh before changing events."
    );
    let count: i64 = tx.query_row("SELECT count(*) FROM calendar_actions WHERE status IN ('queued','running','waiting','repair','rejected','uncertain')", [], |row| row.get(0))?;
    anyhow::ensure!(
        count < 32,
        "Calendar changes are catching up. Retry shortly."
    );
    let (before, requested) = match mutation {
        Mutation::Save { before, after } => (before.as_ref(), after),
        Mutation::Delete { before } => (Some(before), before),
    };
    anyhow::ensure!(
        requested.is_bounded() && before.is_none_or(Event::is_bounded),
        "This event contains oversized calendar metadata."
    );
    let source: String = tx
        .query_row(
            "SELECT source FROM calendar_sources WHERE id=?1",
            [&requested.source_id],
            |row| row.get(0),
        )
        .context("Sync this calendar before changing events.")?;
    anyhow::ensure!(
        !serde_json::from_str::<Source>(&source)?.read_only,
        "This calendar is read only."
    );
    let existing: Option<(String, String, String)> = tx
        .query_row(
            "SELECT a.id,a.status,a.mutation FROM calendar_intents i JOIN calendar_actions a ON a.id=i.action WHERE i.source_id=?1 AND i.event_id=?2",
            params![requested.source_id, requested.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let mut admitted = mutation.clone();
    if let Some((existing_id, status, raw)) = existing {
        anyhow::ensure!(
            matches!(status.as_str(), "queued" | "waiting"),
            "This event already has a provider change in progress. Review it before saving again."
        );
        let previous: Mutation = serde_json::from_str(&raw)?;
        let (
            Mutation::Save {
                before: confirmed,
                after: prior_requested,
            },
            Mutation::Save {
                before: Some(observed),
                after: replacement,
            },
        ) = (previous, mutation)
        else {
            anyhow::bail!(
                "This event already has a pending change. Wait for it to finish or review it first."
            );
        };
        anyhow::ensure!(
            observed == &prior_requested,
            "This event changed after it was shown. Reopen it before saving."
        );
        admitted = Mutation::Save {
            before: confirmed,
            after: replacement.clone(),
        };
        tx.execute(
            "UPDATE calendar_actions SET status='cancelled',error='Replaced by a newer local edit before provider dispatch.' WHERE id=?1 AND status IN ('queued','waiting')",
            [&existing_id],
        )?;
        tx.execute(
            "DELETE FROM calendar_intents WHERE action=?1",
            [&existing_id],
        )?;
    }
    if let Mutation::Save {
        before: Some(before),
        after,
    } = &admitted
    {
        anyhow::ensure!(
            before.source_id == after.source_id
                && before.id == after.id
                && before.etag == after.etag
                && before.remote_url == after.remote_url,
            "An edit must preserve its provider identity. Reopen the event before saving."
        );
    }
    if let Mutation::Save {
        before: Some(before),
        ..
    }
    | Mutation::Delete { before } = &admitted
    {
        let current: Option<String> = tx
            .query_row(
                "SELECT event FROM calendar_events WHERE source_id=?1 AND id=?2",
                params![before.source_id, before.id],
                |row| row.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            current
                .as_deref()
                .map(serde_json::from_str::<Event>)
                .transpose()?
                .as_ref()
                == Some(before),
            "This event changed. Reopen it before saving."
        );
    }
    tx.execute("INSERT INTO calendar_actions(id,status,error,created,mutation,subject,admission_request) VALUES(?1,'queued',NULL,unixepoch('subsec')*1000,?2,?3,?4)", params![id, serde_json::to_string(&admitted)?,subject,serde_json::to_string(mutation)?])?;
    let event = match &admitted {
        Mutation::Save { after, .. } => after,
        Mutation::Delete { before } => before,
    };
    tx.execute("INSERT INTO calendar_intents(source_id,event_id,action) VALUES(?1,?2,?3) ON CONFLICT(source_id,event_id) DO UPDATE SET action=excluded.action",params![event.source_id,event.id,id])?;
    tx.execute(
        "UPDATE calendar_clock SET revision=revision+1 WHERE id=1",
        [],
    )?;
    tx.commit()?;
    Ok(CalendarAdmission {
        id: id.to_owned(),
        status: "queued".into(),
        mutation: mutation.clone(),
        subject: subject.to_owned(),
        receipt: None,
    })
}

#[cfg(test)]
fn admit(db: &mut Connection, id: &str, mutation: &Mutation) -> Result<()> {
    admit_bound(db, id, mutation, "test-subject").map(|_| ())
}

pub fn activities(db: &Connection, offset: u32) -> Result<Vec<Activity>> {
    anyhow::ensure!(offset <= 10_000, "Calendar history offset is too large.");
    let mut statement = db.prepare("SELECT a.id,a.status,a.error,a.created,a.subject,a.mutation,r.receipt FROM calendar_actions a INDEXED BY calendar_action_attention LEFT JOIN calendar_action_receipts r ON r.action=a.id WHERE a.status NOT IN ('succeeded','cancelled') ORDER BY a.created DESC,a.id LIMIT ?1 OFFSET ?2")?;
    statement
        .query_map(params![HISTORY_LIMIT, offset], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
            ))
        })?
        .map(|row| {
            let (id, status, error, created, subject, mutation, receipt) = row?;
            Ok(Activity {
                id,
                status,
                error,
                created,
                subject,
                mutation: serde_json::from_str(&mutation)?,
                receipt: receipt
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
            })
        })
        .collect::<Result<Vec<_>>>()
}

pub fn events(db: &Connection) -> Result<Vec<Event>> {
    let mut statement =
        db.prepare("SELECT event FROM calendar_events ORDER BY source_id,id LIMIT 5000")?;
    statement
        .query_map([], |row| row.get::<_, String>(0))?
        .map(|row| Ok(serde_json::from_str(&row?)?))
        .collect::<Result<Vec<_>>>()
}

pub fn sources(db: &Connection) -> Result<Vec<Source>> {
    let mut statement = db.prepare("SELECT source FROM calendar_sources ORDER BY id LIMIT 50")?;
    statement
        .query_map([], |row| row.get::<_, String>(0))?
        .map(|row| Ok(serde_json::from_str(&row?)?))
        .collect()
}

pub fn projected_snapshot(db: &Connection) -> Result<Snapshot> {
    let tx = db.unchecked_transaction()?;
    let subject = tx
        .query_row(
            "SELECT subject FROM calendar_binding WHERE id=1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let sources = sources(&tx)?;
    let mut projected = events(&tx)?
        .into_iter()
        .map(|event| ((event.source_id.clone(), event.id.clone()), event))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut statement=tx.prepare("SELECT a.mutation,r.receipt FROM calendar_intents i JOIN calendar_actions a ON a.id=i.action LEFT JOIN calendar_action_receipts r ON r.action=a.id ORDER BY i.source_id,i.event_id LIMIT 32")?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (mutation, receipt) in rows {
        let mutation: Mutation = serde_json::from_str(&mutation)?;
        let key = match &mutation {
            Mutation::Save { after, .. } => (after.source_id.clone(), after.id.clone()),
            Mutation::Delete { before } => (before.source_id.clone(), before.id.clone()),
        };
        if let Some(receipt) = receipt {
            let receipt: Receipt = serde_json::from_str(&receipt)?;
            projected.remove(&key);
            if let Some(after) = receipt.after {
                projected.insert((after.source_id.clone(), after.id.clone()), after);
            }
            continue;
        }
        match mutation {
            Mutation::Save { after, .. } => {
                projected.insert(key, after);
            }
            Mutation::Delete { .. } => {
                projected.remove(&key);
            }
        }
    }
    drop(statement);
    tx.commit()?;
    Ok(Snapshot {
        subject,
        sources,
        events: projected.into_values().collect(),
    })
}

#[derive(Serialize)]
pub struct Snapshot {
    subject: Option<String>,
    sources: Vec<Source>,
    events: Vec<Event>,
}

pub async fn sync_bound(
    db: &crate::database::Database,
    provider: &impl CalendarProvider,
    token: String,
    subject: String,
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> Result<Snapshot> {
    anyhow::ensure!(
        !subject.is_empty() && subject.len() <= 255,
        "Reconnect Google Calendar before syncing."
    );
    let observed_subject = subject.clone();
    let (revision, prior, active) = db
        .read(move |db| {
            let tx = db.unchecked_transaction()?;
            let state = (
                tx.query_row(
                    "SELECT revision FROM calendar_clock WHERE id=1",
                    [],
                    |row| row.get::<_, i64>(0),
                )?,
                tx.query_row(
                    "SELECT subject FROM calendar_binding WHERE id=1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()?,
                tx.query_row("SELECT EXISTS(SELECT 1 FROM calendar_intents)", [], |row| {
                    row.get::<_, bool>(0)
                })?,
            );
            tx.commit()?;
            Ok(state)
        })
        .await?;
    if prior.as_deref() != Some(subject.as_str()) {
        anyhow::ensure!(
            !active,
            "Finish or cancel queued calendar changes before switching Google accounts."
        );
    }
    anyhow::ensure!(
        end > start && end - start <= chrono::Duration::days(730),
        "Calendar sync must cover at most two years."
    );
    let sources = provider.sources(&token).await.map_err(anyhow::Error::new)?;
    let mut events = Vec::new();
    for source in &sources {
        events.extend(
            provider
                .events(&token, source, start, end)
                .await
                .map_err(anyhow::Error::new)?,
        );
        anyhow::ensure!(
            events.len() <= 5000,
            "The calendar sync window contains more than 5,000 events."
        );
    }
    let saved_sources = sources.clone();
    let saved_events = events.clone();
    db.write(move |db| {
        let tx = db.transaction()?;
        let current:i64=tx.query_row("SELECT revision FROM calendar_clock WHERE id=1",[],|row|row.get(0))?;
        let current_subject:Option<String>=tx.query_row("SELECT subject FROM calendar_binding WHERE id=1",[],|row|row.get(0)).optional()?;
        anyhow::ensure!(current==revision&&current_subject==prior,"Calendar changed while sync was running. Refresh again.");
        tx.execute("DELETE FROM calendar_sources", [])?;
        tx.execute("DELETE FROM calendar_events", [])?;
        for source in saved_sources {
            tx.execute(
                "INSERT INTO calendar_sources(id,source) VALUES(?1,?2)",
                params![source.id, serde_json::to_string(&source)?],
            )?;
        }
        for event in saved_events {
            tx.execute(
                "INSERT INTO calendar_events(source_id,id,event) VALUES(?1,?2,?3)",
                params![event.source_id, event.id, serde_json::to_string(&event)?],
            )?;
        }
        tx.execute("INSERT INTO calendar_binding(id,subject) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET subject=excluded.subject",[observed_subject])?;
        tx.execute("UPDATE calendar_clock SET revision=revision+1 WHERE id=1",[])?;
        tx.commit()?;
        Ok(())
    })
    .await?;
    Ok(Snapshot {
        subject: Some(subject),
        sources,
        events,
    })
}

#[cfg(test)]
async fn sync(
    db: &crate::database::Database,
    provider: &impl CalendarProvider,
    token: String,
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> Result<Snapshot> {
    sync_bound(db, provider, token, "test-subject".into(), start, end).await
}

pub fn mark_waiting(db: &Connection, id: &str, error: &str) -> Result<()> {
    db.execute("UPDATE calendar_actions SET status='waiting',error=?2 WHERE id=?1 AND status IN ('queued','waiting')", params![id,error])?;
    Ok(())
}

pub fn cancel(db: &Connection, id: &str) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    let changed=tx.execute("UPDATE calendar_actions SET status='cancelled',error=NULL WHERE id=?1 AND status IN ('queued','waiting','rejected')", [id])?;
    anyhow::ensure!(changed == 1, "This calendar change has already started.");
    tx.execute("DELETE FROM calendar_intents WHERE action=?1", [id])?;
    tx.execute(
        "UPDATE calendar_clock SET revision=revision+1 WHERE id=1",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn repair(db: &mut Connection, id: &str) -> Result<()> {
    let tx = db.transaction()?;
    let raw:Option<String>=tx.query_row("SELECT r.receipt FROM calendar_actions a JOIN calendar_action_receipts r ON r.action=a.id WHERE a.id=?1 AND a.status='repair'",[id],|row|row.get(0)).optional()?;
    let Some(raw) = raw else {
        return Ok(());
    };
    let receipt: Receipt = serde_json::from_str(&raw)?;
    if let Some(after) = receipt.after {
        let current: Option<String> = tx
            .query_row(
                "SELECT event FROM calendar_events WHERE source_id=?1 AND id=?2",
                params![after.source_id, after.id],
                |row| row.get(0),
            )
            .optional()?;
        let current = current
            .as_deref()
            .map(serde_json::from_str::<Event>)
            .transpose()?;
        let safe = match current.as_ref() {
            Some(event) => event == &after || receipt.before.as_ref() == Some(event),
            None => receipt.before.is_none(),
        };
        anyhow::ensure!(
            safe,
            "A newer cached event must be reviewed before applying this receipt."
        );
        tx.execute("INSERT INTO calendar_events(source_id,id,event) VALUES(?1,?2,?3) ON CONFLICT(source_id,id) DO UPDATE SET event=excluded.event",params![after.source_id,after.id,serde_json::to_string(&after)?])?;
    } else {
        let before = receipt
            .before
            .context("The acknowledged delete receipt has no source event.")?;
        let current: Option<String> = tx
            .query_row(
                "SELECT event FROM calendar_events WHERE source_id=?1 AND id=?2",
                params![before.source_id, before.id],
                |row| row.get(0),
            )
            .optional()?;
        let current = current
            .as_deref()
            .map(serde_json::from_str::<Event>)
            .transpose()?;
        anyhow::ensure!(
            current.as_ref().is_none_or(|event| event == &before),
            "A newer cached event must be reviewed before applying this delete receipt."
        );
        tx.execute(
            "DELETE FROM calendar_events WHERE source_id=?1 AND id=?2",
            params![before.source_id, before.id],
        )?;
    }
    tx.execute(
        "UPDATE calendar_actions SET status='succeeded',error=NULL WHERE id=?1 AND status='repair'",
        [id],
    )?;
    tx.execute("DELETE FROM calendar_intents WHERE action=?1", [id])?;
    tx.execute(
        "UPDATE calendar_clock SET revision=revision+1 WHERE id=1",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

pub async fn execute_bound(
    db: &crate::database::Database,
    provider: &impl CalendarProvider,
    id: String,
    token: String,
    subject: String,
) -> Result<()> {
    let lookup = id.clone();
    let status = db
        .read(move |db| {
            Ok(db.query_row(
                "SELECT status FROM calendar_actions WHERE id=?1",
                [lookup],
                |row| row.get::<_, String>(0),
            )?)
        })
        .await?;
    if status == "repair" {
        let saved = id;
        return db.write(move |db| repair(db, &saved)).await;
    }
    let saved = id.clone();
    let claimed=db.write(move |db| {
        let tx=db.transaction()?;
        let (raw,bound):(String,Option<String>)=tx.query_row("SELECT mutation,subject FROM calendar_actions WHERE id=?1 AND status IN ('queued','waiting')", [&saved], |row|Ok((row.get(0)?,row.get(1)?))).context("This calendar change is no longer runnable.")?;
        let current:String=tx.query_row("SELECT subject FROM calendar_binding WHERE id=1",[],|row|row.get(0)).context("Google Calendar must be reconnected.")?;
        if bound.as_deref()!=Some(subject.as_str())||current!=subject{return Err(ClaimWaiting("Reconnect the original Google account to continue this change.").into());}
        let mutation=serde_json::from_str::<Mutation>(&raw)?;
        let event=match &mutation {Mutation::Save{after,..}=>after,Mutation::Delete{before}=>before};
        let source:String=tx.query_row("SELECT source FROM calendar_sources WHERE id=?1",[&event.source_id],|row|row.get(0)).context("This calendar source was removed.")?;
        if serde_json::from_str::<Source>(&source)?.read_only{return Err(ClaimRejected("This calendar is read only.").into());}
        if let Mutation::Save{before:Some(before),after}=&mutation
            && (before.source_id!=after.source_id||before.id!=after.id||before.etag!=after.etag||before.remote_url!=after.remote_url) {
            return Err(ClaimRejected("The event provider identity changed before dispatch.").into());
        }
        let owner:Option<String>=tx.query_row("SELECT action FROM calendar_intents WHERE source_id=?1 AND event_id=?2",params![event.source_id,event.id],|row|row.get(0)).optional()?;
        if owner.as_deref()!=Some(saved.as_str()) { return Err(ClaimRejected("A newer change owns this event.").into()); }
        if let Mutation::Save{before:Some(before),..}|Mutation::Delete{before}= &mutation {
            let current:Option<String>=tx.query_row("SELECT event FROM calendar_events WHERE source_id=?1 AND id=?2",params![before.source_id,before.id],|row|row.get(0)).optional()?;
            if current.as_deref().map(serde_json::from_str::<Event>).transpose()?.as_ref()!=Some(before) { return Err(ClaimRejected("This event changed before the provider write started.").into()); }
        }
        tx.execute("UPDATE calendar_actions SET status='running',error=NULL WHERE id=?1", [&saved])?;
        tx.commit()?; Ok(mutation)
    }).await;
    let mutation = match claimed {
        Ok(mutation) => mutation,
        Err(error) if error.downcast_ref::<ClaimRejected>().is_some() => {
            let saved = id;
            let message = format!("{error:#}");
            db.write(move|db|{let tx=db.transaction()?;tx.execute("UPDATE calendar_actions SET status='rejected',error=?2 WHERE id=?1 AND status IN ('queued','waiting')",params![&saved,message])?;tx.execute("DELETE FROM calendar_intents WHERE action=?1",[&saved])?;tx.execute("UPDATE calendar_clock SET revision=revision+1 WHERE id=1",[])?;tx.commit()?;Ok(())}).await?;
            return Ok(());
        }
        Err(error) if error.downcast_ref::<ClaimWaiting>().is_some() => {
            let saved = id;
            let message = format!("{error:#}");
            db.write(move|db|{db.execute("UPDATE calendar_actions SET status='waiting',error=?2 WHERE id=?1 AND status IN ('queued','waiting')",params![saved,message])?;Ok(())}).await?;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let result = match &mutation {
        Mutation::Save { after, .. } => provider.save(&token, &id, after).await.map(Some),
        Mutation::Delete { before } => provider.delete(&token, before).await.map(|()| None),
    };
    match result {
        Ok(after) => {
            let receipt = Receipt {
                request_id: id.clone(),
                before: match &mutation {
                    Mutation::Save { before, .. } => before.clone(),
                    Mutation::Delete { before } => Some(before.clone()),
                },
                after: after.clone(),
            };
            let saved = id.clone();
            db.write(move |db| { let tx=db.transaction()?;
                tx.execute("INSERT INTO calendar_action_receipts(action,receipt) VALUES(?1,?2)",params![&saved,serde_json::to_string(&receipt)?])?;
                tx.execute("UPDATE calendar_actions SET status='repair',error='Google saved this event. Finish saving it on this device.' WHERE id=?1 AND status='running'",[&saved])?;tx.commit()?;Ok(())}).await?;
            let saved = id;
            db.write(move |db| repair(db, &saved)).await
        }
        Err(error) => {
            let status = match error.kind {
                FailureKind::Waiting => "waiting",
                FailureKind::Rejected => "rejected",
                FailureKind::Uncertain => "uncertain",
            };
            let message = error.to_string();
            let saved = id;
            db.write(move|db| {let tx=db.transaction()?;tx.execute("UPDATE calendar_actions SET status=?2,error=?3 WHERE id=?1 AND status='running'",params![&saved,status,message])?;if status=="rejected"{tx.execute("DELETE FROM calendar_intents WHERE action=?1",[&saved])?;tx.execute("UPDATE calendar_clock SET revision=revision+1 WHERE id=1",[])?;}tx.commit()?;Ok(())}).await
        }
    }
}

#[cfg(test)]
async fn execute(
    db: &crate::database::Database,
    provider: &impl CalendarProvider,
    id: String,
    token: String,
) -> Result<()> {
    execute_bound(db, provider, id, token, "test-subject".into()).await
}

pub async fn inspect_bound(
    db: &crate::database::Database,
    provider: &impl CalendarProvider,
    id: String,
    token: String,
    subject: String,
) -> Result<()> {
    let saved = id.clone();
    let mutation = db
        .read(move |db| {
            let (status, raw, bound): (String, String, Option<String>) = db.query_row(
                "SELECT status,mutation,subject FROM calendar_actions WHERE id=?1",
                [saved],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            anyhow::ensure!(
                status == "uncertain",
                "This calendar change does not need inspection."
            );
            let current: String = db
                .query_row(
                    "SELECT subject FROM calendar_binding WHERE id=1",
                    [],
                    |row| row.get(0),
                )
                .context("Google Calendar must be reconnected.")?;
            anyhow::ensure!(
                bound.as_deref() == Some(subject.as_str()) && current == subject,
                "Reconnect the original Google account to inspect this change."
            );
            Ok(serde_json::from_str::<Mutation>(&raw)?)
        })
        .await?;
    let mut expected = match &mutation {
        Mutation::Save { after, .. } => after.clone(),
        Mutation::Delete { before } => before.clone(),
    };
    if expected.is_create() {
        expected.id = format!("shep{}", id.replace('-', ""));
    }
    let observed = provider.read(&token, &expected).await?;
    let proven = match &mutation {
        Mutation::Save { .. } => observed
            .as_ref()
            .is_some_and(|event| event.same_content(&expected)),
        Mutation::Delete { .. } => observed.is_none(),
    };
    if !proven {
        let saved = id;
        db.write(move|db|{let tx=db.transaction()?;tx.execute("UPDATE calendar_actions SET status='rejected',error='Google does not contain the exact saved event. Reopen the event before trying again.' WHERE id=?1 AND status='uncertain'",[&saved])?;tx.execute("DELETE FROM calendar_intents WHERE action=?1",[&saved])?;tx.execute("UPDATE calendar_clock SET revision=revision+1 WHERE id=1",[])?;tx.commit()?;Ok(())}).await?;
        return Ok(());
    }
    let receipt = Receipt {
        request_id: id.clone(),
        before: match mutation {
            Mutation::Save { before, .. } => before,
            Mutation::Delete { before } => Some(before),
        },
        after: observed,
    };
    let saved = id.clone();
    db.write(move|db|{let tx=db.transaction()?;
        tx.execute("INSERT OR IGNORE INTO calendar_action_receipts(action,receipt) VALUES(?1,?2)",params![&saved,serde_json::to_string(&receipt)?])?;
        tx.execute("UPDATE calendar_actions SET status='repair',error='The exact Google event was found. Finish saving it on this device.' WHERE id=?1 AND status='uncertain'",[&saved])?;tx.commit()?;Ok(())}).await?;
    db.write(move |db| repair(db, &id)).await
}

#[cfg(test)]
async fn inspect(
    db: &crate::database::Database,
    provider: &impl CalendarProvider,
    id: String,
    token: String,
) -> Result<()> {
    inspect_bound(db, provider, id, token, "test-subject".into()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::{TimeZone, Utc};
    use mockall::predicate::eq;

    fn event(id: &str, etag: Option<&str>) -> Event {
        Event {
            id: id.into(),
            source_id: "primary".into(),
            title: "Review".into(),
            start: Utc.timestamp_opt(1_800_000_000, 0).single().expect("time"),
            end: Utc.timestamp_opt(1_800_003_600, 0).single().expect("time"),
            location: String::new(),
            description: String::new(),
            all_day: false,
            etag: etag.map(str::to_owned),
            remote_url: etag.map(|_| id.into()),
        }
    }

    async fn database() -> (tempfile::TempDir, std::sync::Arc<crate::database::Database>) {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory
            .path()
            .join("calendar.sqlite")
            .to_string_lossy()
            .into_owned();
        let db = crate::database::Database::open(path)
            .await
            .expect("database");
        db.write(|db| {
            let source = Source {
                id: "primary".into(),
                name: "Personal".into(),
                read_only: false,
            };
            db.execute(
                "INSERT OR IGNORE INTO calendar_sources(id,source) VALUES('primary',?1)",
                [serde_json::to_string(&source)?],
            )?;
            db.execute(
                "INSERT OR REPLACE INTO calendar_binding(id,subject) VALUES(1,'test-subject')",
                [],
            )?;
            Ok(())
        })
        .await
        .expect("source");
        (directory, db)
    }

    #[tokio::test]
    async fn queued_action_is_bound_to_the_observed_google_subject() {
        let (directory, db) = database().await;
        let requested = event("local", None);
        db.write({
            let requested = requested.clone();
            move |db| {
                admit_bound(
                    db,
                    "action",
                    &Mutation::Save {
                        before: None,
                        after: requested,
                    },
                    "test-subject",
                )
            }
        })
        .await
        .expect("admit");
        drop(db);
        let db = crate::database::Database::open(
            directory
                .path()
                .join("calendar.sqlite")
                .to_string_lossy()
                .into_owned(),
        )
        .await
        .expect("reopen profile");
        db.write(|db| {
            db.execute(
                "UPDATE calendar_binding SET subject='replacement-subject' WHERE id=1",
                [],
            )?;
            Ok(())
        })
        .await
        .expect("switch subject");
        let mut provider = MockCalendarProvider::new();
        provider.expect_save().times(0);
        provider.expect_delete().times(0);
        provider.expect_read().times(0);
        execute_bound(
            &db,
            &provider,
            "action".into(),
            "token".into(),
            "replacement-subject".into(),
        )
        .await
        .expect("classify subject mismatch");
        let state = db
            .read(|db| {
                Ok(db.query_row(
                    "SELECT status,EXISTS(SELECT 1 FROM calendar_intents WHERE action='action') FROM calendar_actions WHERE id='action'",
                    [],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
                )?)
            })
            .await
            .expect("state");
        assert_eq!(state, ("waiting".into(), true));
    }

    #[tokio::test]
    async fn newer_local_edit_replaces_only_an_undispatched_save() {
        let (_directory, db) = database().await;
        let before = event("remote", Some("v1"));
        let mut first = before.clone();
        first.title = "First".into();
        let mut second = first.clone();
        second.title = "Second".into();
        db.write({
            let before = before.clone();
            let first = first.clone();
            move |db| {
                db.execute(
                    "INSERT INTO calendar_events(source_id,id,event) VALUES('primary','remote',?1)",
                    [serde_json::to_string(&before)?],
                )?;
                admit(
                    db,
                    "first",
                    &Mutation::Save {
                        before: Some(before),
                        after: first,
                    },
                )
            }
        })
        .await
        .expect("first admission");
        db.write({
            let first = first.clone();
            let second = second.clone();
            move |db| {
                admit(
                    db,
                    "second",
                    &Mutation::Save {
                        before: Some(first),
                        after: second,
                    },
                )
            }
        })
        .await
        .expect("replacement admission");
        let (old_status, new_raw, owner) = db.read(|db| Ok((
            db.query_row("SELECT status FROM calendar_actions WHERE id='first'", [], |row| row.get::<_, String>(0))?,
            db.query_row("SELECT mutation FROM calendar_actions WHERE id='second'", [], |row| row.get::<_, String>(0))?,
            db.query_row("SELECT action FROM calendar_intents WHERE source_id='primary' AND event_id='remote'", [], |row| row.get::<_, String>(0))?,
        ))).await.expect("state");
        assert_eq!(old_status, "cancelled");
        assert_eq!(owner, "second");
        assert_eq!(
            serde_json::from_str::<Mutation>(&new_raw).expect("mutation"),
            Mutation::Save {
                before: Some(before),
                after: second
            },
        );
    }

    #[tokio::test]
    async fn exact_admission_request_reconciles_lost_replies_after_coalescing() {
        let (_directory, db) = database().await;
        let before = event("remote", Some("v1"));
        let mut first = before.clone();
        first.title = "First".into();
        let mut second = first.clone();
        second.title = "Second".into();
        db.write({
            let before = before.clone();
            let first = first.clone();
            move |db| {
                db.execute(
                    "INSERT INTO calendar_events(source_id,id,event) VALUES('primary','remote',?1)",
                    [serde_json::to_string(&before)?],
                )?;
                admit(
                    db,
                    "first",
                    &Mutation::Save {
                        before: Some(before),
                        after: first,
                    },
                )
            }
        })
        .await
        .expect("first");
        let request = Mutation::Save {
            before: Some(first),
            after: second,
        };
        let admitted = db
            .write({
                let request = request.clone();
                move |db| admit_bound(db, "stable", &request, "test-subject")
            })
            .await
            .expect("replacement");
        assert_eq!(admitted.mutation, request);
        db.write(|db| {
            db.execute(
                "UPDATE calendar_actions SET status='waiting' WHERE id='stable'",
                [],
            )?;
            Ok(())
        })
        .await
        .expect("waiting");
        let reconciled = db
            .write({
                let request = request.clone();
                move |db| admit_bound(db, "stable", &request, "test-subject")
            })
            .await
            .expect("lost reply");
        assert_eq!(reconciled.status, "waiting");
        assert_eq!(reconciled.mutation, request);
        assert_eq!(
            db.read(|db| action_admission(db, "stable"))
                .await
                .expect("lookup"),
            Some(reconciled),
        );
        let acknowledged = Receipt {
            request_id: "stable".into(),
            before: Some(before.clone()),
            after: Some(event("remote", Some("v2"))),
        };
        db.write({
            let acknowledged = acknowledged.clone();
            move |db| {
                db.execute(
                    "UPDATE calendar_actions SET status='repair' WHERE id='stable'",
                    [],
                )?;
                db.execute(
                    "INSERT INTO calendar_action_receipts(action,receipt) VALUES('stable',?1)",
                    [serde_json::to_string(&acknowledged)?],
                )?;
                Ok(())
            }
        })
        .await
        .expect("receipt");
        assert_eq!(
            db.read(|db| action_admission(db, "stable"))
                .await
                .expect("receipt lookup")
                .and_then(|value| value.receipt),
            Some(acknowledged),
        );
        let mut changed = request;
        if let Mutation::Save { after, .. } = &mut changed {
            after.title = "Third".into();
        }
        let error = db
            .write(move |db| admit_bound(db, "stable", &changed, "test-subject"))
            .await
            .expect_err("UUID reuse");
        assert!(error.to_string().contains("another change"));
    }

    #[tokio::test]
    async fn newer_local_edit_cannot_replace_started_or_reviewable_work() {
        for status in ["running", "uncertain", "repair"] {
            let (_directory, db) = database().await;
            let before = event("remote", Some("v1"));
            let mut first = before.clone();
            first.title = "First".into();
            let mut second = first.clone();
            second.title = "Second".into();
            db.write({
                let before = before.clone();
                let first = first.clone();
                move |db| {
                    db.execute(
                        "INSERT INTO calendar_events(source_id,id,event) VALUES('primary','remote',?1)",
                        [serde_json::to_string(&before)?],
                    )?;
                    admit(db, "first", &Mutation::Save { before: Some(before), after: first })?;
                    db.execute("UPDATE calendar_actions SET status=?1 WHERE id='first'", [status])?;
                    Ok(())
                }
            }).await.expect("first state");
            let refused = db
                .write(move |db| {
                    admit(
                        db,
                        "second",
                        &Mutation::Save {
                            before: Some(first),
                            after: second,
                        },
                    )
                })
                .await
                .expect_err("started work must retain ownership");
            assert!(refused.to_string().contains("provider change in progress"));
            let owner = db.read(|db| Ok(db.query_row(
                "SELECT action FROM calendar_intents WHERE source_id='primary' AND event_id='remote'", [], |row| row.get::<_, String>(0)
            )?)).await.expect("owner");
            assert_eq!(owner, "first");
        }
    }

    #[tokio::test]
    async fn unbound_legacy_intent_cannot_adopt_a_new_google_account() {
        let (_directory, db) = database().await;
        db.write(|db| {
            db.execute("DELETE FROM calendar_binding", [])?;
            db.execute(
                "INSERT INTO calendar_actions(id,status,created,mutation) VALUES('legacy','uncertain',1,?1)",
                [serde_json::to_string(&Mutation::Save {
                    before: None,
                    after: event("local", None),
                })?],
            )?;
            db.execute("INSERT INTO calendar_intents VALUES('primary','local','legacy')", [])?;
            Ok(())
        }).await.expect("legacy state");
        let mut provider = MockCalendarProvider::new();
        provider.expect_sources().times(0);
        provider.expect_events().times(0);
        assert!(
            sync_bound(
                &db,
                &provider,
                "token".into(),
                "new-subject".into(),
                Utc.timestamp_opt(1_700_000_000, 0).single().expect("start"),
                Utc.timestamp_opt(1_760_000_000, 0).single().expect("end"),
            )
            .await
            .is_err()
        );
        let snapshot = db.read(projected_snapshot).await.expect("snapshot");
        assert!(snapshot.subject.is_none());
        assert_eq!(snapshot.events.len(), 1);
    }

    #[tokio::test]
    async fn legacy_unbound_action_waits_without_dispatch_but_its_receipt_repairs() {
        let (_directory, db) = database().await;
        let requested = event("local", None);
        db.write({
            let requested = requested.clone();
            move |db| {
                db.execute(
                    "INSERT INTO calendar_actions(id,status,error,created,mutation,subject) VALUES('legacy','queued',NULL,1,?1,NULL)",
                    [serde_json::to_string(&Mutation::Save { before: None, after: requested.clone() })?],
                )?;
                db.execute(
                    "INSERT INTO calendar_intents(source_id,event_id,action) VALUES('primary','local','legacy')",
                    [],
                )?;
                Ok(())
            }
        }).await.expect("legacy action");
        let mut provider = MockCalendarProvider::new();
        provider.expect_save().times(0);
        provider.expect_delete().times(0);
        provider.expect_read().times(0);
        execute_bound(
            &db,
            &provider,
            "legacy".into(),
            "token".into(),
            "test-subject".into(),
        )
        .await
        .expect("classify unbound");
        let waiting = db
            .read(|db| {
                Ok(db.query_row(
                    "SELECT status FROM calendar_actions WHERE id='legacy'",
                    [],
                    |row| row.get::<_, String>(0),
                )?)
            })
            .await
            .expect("waiting");
        assert_eq!(waiting, "waiting");
        let mut acknowledged = requested;
        acknowledged.id = "remote".into();
        acknowledged.etag = Some("v1".into());
        acknowledged.remote_url = Some("remote".into());
        db.write(move |db| {
            db.execute(
                "UPDATE calendar_actions SET status='repair' WHERE id='legacy'",
                [],
            )?;
            db.execute(
                "INSERT INTO calendar_action_receipts(action,receipt) VALUES('legacy',?1)",
                [serde_json::to_string(&Receipt {
                    request_id: "legacy".into(),
                    before: None,
                    after: Some(acknowledged),
                })?],
            )?;
            repair(db, "legacy")
        })
        .await
        .expect("repair acknowledged legacy action");
        let repaired = db
            .read(|db| {
                Ok(db.query_row(
                    "SELECT status FROM calendar_actions WHERE id='legacy'",
                    [],
                    |row| row.get::<_, String>(0),
                )?)
            })
            .await
            .expect("repaired");
        assert_eq!(repaired, "succeeded");
    }

    #[tokio::test]
    async fn stale_sync_cannot_replace_cache_after_a_new_admission() {
        let (_directory, db) = database().await;
        let source = Source {
            id: "primary".into(),
            name: "Personal".into(),
            read_only: false,
        };
        let stale = event("stale", Some("v1"));
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let mut provider = MockCalendarProvider::new();
        let returned_source = source.clone();
        let started_call = started.clone();
        let release_call = release.clone();
        provider.expect_sources().times(1).return_once(move |_| {
            Box::pin(async move {
                started_call.notify_one();
                release_call.notified().await;
                Ok(vec![returned_source])
            })
        });
        provider
            .expect_events()
            .times(1)
            .return_once(move |_, _, _, _| Box::pin(async move { Ok(vec![stale]) }));
        let provider = std::sync::Arc::new(provider);
        let sync_db = db.clone();
        let sync_provider = provider.clone();
        let task = tokio::spawn(async move {
            sync_bound(
                &sync_db,
                sync_provider.as_ref(),
                "token".into(),
                "test-subject".into(),
                Utc.timestamp_opt(1_700_000_000, 0).single().expect("start"),
                Utc.timestamp_opt(1_760_000_000, 0).single().expect("end"),
            )
            .await
        });
        started.notified().await;
        db.write(move |db| {
            admit(
                db,
                "newer",
                &Mutation::Save {
                    before: None,
                    after: event("newer", None),
                },
            )
        })
        .await
        .expect("newer admission");
        release.notify_one();
        let error = match task.await.expect("join") {
            Ok(_) => panic!("stale sync was applied"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("changed while sync"));
        let snapshot = db.read(projected_snapshot).await.expect("snapshot");
        assert!(snapshot.events.iter().any(|entry| entry.id == "newer"));
        assert!(!snapshot.events.iter().any(|entry| entry.id == "stale"));
    }

    #[tokio::test]
    async fn held_sync_cannot_overwrite_an_acknowledged_receipt_repair() {
        let (_directory, db) = database().await;
        let before = event("remote", Some("v1"));
        let mut requested = before.clone();
        requested.title = "Requested".into();
        let mut acknowledged = requested.clone();
        acknowledged.etag = Some("v2".into());
        db.write({
            let before = before.clone();
            let requested = requested.clone();
            move |db| {
                db.execute(
                    "INSERT INTO calendar_events(source_id,id,event) VALUES('primary','remote',?1)",
                    [serde_json::to_string(&before)?],
                )?;
                admit(
                    db,
                    "edit",
                    &Mutation::Save {
                        before: Some(before),
                        after: requested,
                    },
                )
            }
        })
        .await
        .expect("admit edit");
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let mut provider = MockCalendarProvider::new();
        let started_call = started.clone();
        let release_call = release.clone();
        provider.expect_sources().times(1).return_once(move |_| {
            Box::pin(async move {
                started_call.notify_one();
                release_call.notified().await;
                Ok(vec![Source {
                    id: "primary".into(),
                    name: "Personal".into(),
                    read_only: false,
                }])
            })
        });
        let stale = before.clone();
        provider
            .expect_events()
            .times(1)
            .return_once(move |_, _, _, _| Box::pin(async move { Ok(vec![stale]) }));
        let provider = std::sync::Arc::new(provider);
        let sync_db = db.clone();
        let sync_provider = provider.clone();
        let task = tokio::spawn(async move {
            sync_bound(
                &sync_db,
                sync_provider.as_ref(),
                "token".into(),
                "test-subject".into(),
                Utc.timestamp_opt(1_700_000_000, 0).single().expect("start"),
                Utc.timestamp_opt(1_760_000_000, 0).single().expect("end"),
            )
            .await
        });
        started.notified().await;
        db.write({
            let before = before.clone();
            let acknowledged = acknowledged.clone();
            move |db| {
                db.execute(
                    "UPDATE calendar_actions SET status='repair' WHERE id='edit'",
                    [],
                )?;
                db.execute(
                    "INSERT INTO calendar_action_receipts(action,receipt) VALUES('edit',?1)",
                    [serde_json::to_string(&Receipt {
                        request_id: "edit".into(),
                        before: Some(before),
                        after: Some(acknowledged),
                    })?],
                )?;
                repair(db, "edit")
            }
        })
        .await
        .expect("repair receipt");
        release.notify_one();
        assert!(task.await.expect("join").is_err());
        let snapshot = db.read(projected_snapshot).await.expect("snapshot");
        assert_eq!(snapshot.events, vec![acknowledged]);
    }

    #[tokio::test]
    async fn acknowledged_provider_write_saves_receipt_before_terminal_cache() {
        let (_directory, db) = database().await;
        let requested = event("local", None);
        db.write({
            let requested = requested.clone();
            move |db| {
                admit(
                    db,
                    "action",
                    &Mutation::Save {
                        before: None,
                        after: requested,
                    },
                )
            }
        })
        .await
        .expect("admit");
        let mut provider = MockCalendarProvider::new();
        let mut saved = requested.clone();
        saved.id = "remote".into();
        saved.etag = Some("v1".into());
        saved.remote_url = Some("remote".into());
        let reply = saved.clone();
        provider
            .expect_save()
            .with(eq("token"), eq("action"), eq(requested))
            .times(1)
            .return_once(move |_, _, _| Box::pin(async move { Ok(reply) }));
        provider.expect_read().times(0);
        execute(&db, &provider, "action".into(), "token".into())
            .await
            .expect("execute");
        let state=db.read(move|db|Ok(db.query_row("SELECT a.status,EXISTS(SELECT 1 FROM calendar_action_receipts r WHERE r.action=a.id),EXISTS(SELECT 1 FROM calendar_events e WHERE e.id='remote') FROM calendar_actions a WHERE a.id='action'",[],|row|Ok((row.get::<_,String>(0)?,row.get::<_,bool>(1)?,row.get::<_,bool>(2)?)))?)).await.expect("state");
        assert_eq!(state, ("succeeded".into(), true, true));
    }

    #[tokio::test]
    async fn unknown_provider_result_is_never_replayed_by_inspection() {
        let (_directory, db) = database().await;
        let requested = event("local", None);
        db.write({
            let requested = requested.clone();
            move |db| {
                admit(
                    db,
                    "action",
                    &Mutation::Save {
                        before: None,
                        after: requested,
                    },
                )
            }
        })
        .await
        .expect("admit");
        let mut provider = MockCalendarProvider::new();
        provider.expect_save().times(1).return_once(|_, _, _| {
            Box::pin(async { Err(ProviderFailure::uncertain("lost reply")) })
        });
        execute(&db, &provider, "action".into(), "token".into())
            .await
            .expect("execute");
        let mut observed = requested.clone();
        observed.id = "shepaction".into();
        observed.etag = Some("v1".into());
        observed.remote_url = Some("shepaction".into());
        provider.expect_save().times(0);
        provider
            .expect_read()
            .with(
                eq("token"),
                mockall::predicate::function(|event: &Event| event.id == "shepaction"),
            )
            .times(1)
            .return_once(move |_, _| Box::pin(async move { Ok(Some(observed)) }));
        inspect(&db, &provider, "action".into(), "token".into())
            .await
            .expect("inspect");
        let status = db
            .read(|db| {
                Ok(db.query_row(
                    "SELECT status FROM calendar_actions WHERE id='action'",
                    [],
                    |row| row.get::<_, String>(0),
                )?)
            })
            .await
            .expect("status");
        assert_eq!(status, "succeeded");
    }

    #[tokio::test]
    async fn definite_refusal_rejects_while_predispatch_wait_remains_runnable() {
        let (_directory, db) = database().await;
        let requested = event("local", None);
        db.write({
            let requested = requested.clone();
            move |db| {
                admit(
                    db,
                    "refused",
                    &Mutation::Save {
                        before: None,
                        after: requested,
                    },
                )
            }
        })
        .await
        .expect("admit");
        let mut provider = MockCalendarProvider::new();
        provider
            .expect_save()
            .times(1)
            .return_once(|_, _, _| Box::pin(async { Err(ProviderFailure::rejected("forbidden")) }));
        execute(&db, &provider, "refused".into(), "token".into())
            .await
            .expect("execute");
        db.write({
            let requested = requested.clone();
            move |db| {
                admit(
                    db,
                    "waiting",
                    &Mutation::Save {
                        before: None,
                        after: requested,
                    },
                )?;
                mark_waiting(db, "waiting", "Connect Google")
            }
        })
        .await
        .expect("waiting");
        let states = db
            .read(|db| {
                let mut statement =
                    db.prepare("SELECT id,status FROM calendar_actions ORDER BY id")?;
                Ok(statement
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?)
            })
            .await
            .expect("states");
        assert_eq!(
            states,
            vec![
                ("refused".into(), "rejected".into()),
                ("waiting".into(), "waiting".into())
            ]
        );
    }

    #[tokio::test]
    async fn changed_event_is_rejected_at_claim_without_provider_io() {
        let (_directory, db) = database().await;
        let before = event("remote", Some("v1"));
        db.write({
            let before = before.clone();
            move |db| {
                db.execute(
                    "INSERT INTO calendar_events(source_id,id,event) VALUES(?1,?2,?3)",
                    params![before.source_id, before.id, serde_json::to_string(&before)?],
                )?;
                Ok(())
            }
        })
        .await
        .expect("seed");
        let mut after = before.clone();
        after.title = "Mine".into();
        db.write({
            let before = before.clone();
            move |db| {
                admit(
                    db,
                    "action",
                    &Mutation::Save {
                        before: Some(before),
                        after,
                    },
                )
            }
        })
        .await
        .expect("admit");
        let mut replacement = before;
        replacement.etag = Some("v2".into());
        replacement.title = "Elsewhere".into();
        db.write(move |db| {
            db.execute(
                "UPDATE calendar_events SET event=?1 WHERE source_id='primary' AND id='remote'",
                [serde_json::to_string(&replacement)?],
            )?;
            Ok(())
        })
        .await
        .expect("replace");
        let mut provider = MockCalendarProvider::new();
        provider.expect_save().times(0);
        provider.expect_read().times(0);
        execute(&db, &provider, "action".into(), "token".into())
            .await
            .expect("classified");
        let status = db
            .read(|db| {
                Ok(db.query_row(
                    "SELECT status FROM calendar_actions WHERE id='action'",
                    [],
                    |row| row.get::<_, String>(0),
                )?)
            })
            .await
            .expect("status");
        assert_eq!(status, "rejected");
    }

    #[tokio::test]
    async fn acknowledged_receipt_survives_cache_failure_without_replay() {
        let (_directory, db) = database().await;
        let requested = event("local", None);
        db.write({let requested=requested.clone();move|db|{admit(db,"action",&Mutation::Save{before:None,after:requested})?;db.execute_batch("CREATE TRIGGER fail_calendar_cache BEFORE INSERT ON calendar_events BEGIN SELECT RAISE(ABORT,'disk fixture'); END;")?;Ok(())}}).await.expect("admit");
        let mut saved = requested;
        saved.id = "remote".into();
        saved.etag = Some("v1".into());
        saved.remote_url = Some("remote".into());
        let mut provider = MockCalendarProvider::new();
        provider
            .expect_save()
            .times(1)
            .return_once(move |_, _, _| Box::pin(async move { Ok(saved) }));
        provider.expect_read().times(0);
        assert!(
            execute(&db, &provider, "action".into(), "token".into())
                .await
                .is_err()
        );
        let state=db.read(|db|Ok(db.query_row("SELECT status,EXISTS(SELECT 1 FROM calendar_action_receipts WHERE action='action') FROM calendar_actions WHERE id='action'",[],|row|Ok((row.get::<_,String>(0)?,row.get::<_,bool>(1)?)))?)).await.expect("state");
        assert_eq!(state, ("repair".into(), true));
        assert!(
            execute(&db, &provider, "action".into(), "token".into())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn unknown_delete_only_inspects_before_removing_exact_cache() {
        let (_directory, db) = database().await;
        let before = event("remote", Some("v1"));
        db.write({
            let before = before.clone();
            move |db| {
                db.execute(
                    "INSERT INTO calendar_events(source_id,id,event) VALUES(?1,?2,?3)",
                    params![before.source_id, before.id, serde_json::to_string(&before)?],
                )?;
                admit(db, "delete", &Mutation::Delete { before })
            }
        })
        .await
        .expect("admit");
        let mut provider = MockCalendarProvider::new();
        provider
            .expect_delete()
            .with(eq("token"), eq(before.clone()))
            .times(1)
            .return_once(|_, _| {
                Box::pin(async { Err(ProviderFailure::uncertain("lost delete reply")) })
            });
        provider.expect_save().times(0);
        provider.expect_sources().times(0);
        provider.expect_events().times(0);
        execute(&db, &provider, "delete".into(), "token".into())
            .await
            .expect("classified");
        provider.expect_delete().times(0);
        provider
            .expect_read()
            .with(eq("token"), eq(before))
            .times(1)
            .return_once(|_, _| Box::pin(async { Ok(None) }));
        inspect(&db, &provider, "delete".into(), "token".into())
            .await
            .expect("inspect");
        let state = db
            .read(|db| {
                Ok((
                    db.query_row(
                        "SELECT status FROM calendar_actions WHERE id='delete'",
                        [],
                        |row| row.get::<_, String>(0),
                    )?,
                    db.query_row("SELECT count(*) FROM calendar_events", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                ))
            })
            .await
            .expect("state");
        assert_eq!(state, ("succeeded".into(), 0));
    }

    #[tokio::test]
    async fn discovery_commits_bounded_sources_with_events() {
        let (_directory, db) = database().await;
        let source = Source {
            id: "primary".into(),
            name: "Personal".into(),
            read_only: false,
        };
        let saved = event("remote", Some("v1"));
        let mut provider = MockCalendarProvider::new();
        let returned_source = source.clone();
        provider
            .expect_sources()
            .with(eq("token"))
            .times(1)
            .return_once(move |_| Box::pin(async move { Ok(vec![returned_source]) }));
        provider
            .expect_events()
            .times(1)
            .return_once(move |_, _, _, _| Box::pin(async move { Ok(vec![saved]) }));
        provider.expect_save().times(0);
        provider.expect_delete().times(0);
        provider.expect_read().times(0);
        let snapshot = sync(
            &db,
            &provider,
            "token".into(),
            Utc.timestamp_opt(1_700_000_000, 0).single().expect("start"),
            Utc.timestamp_opt(1_760_000_000, 0).single().expect("end"),
        )
        .await
        .expect("sync");
        assert_eq!(snapshot.sources, vec![source]);
        assert_eq!(snapshot.events.len(), 1);
        let counts = db
            .read(|db| {
                Ok((
                    db.query_row("SELECT count(*) FROM calendar_sources", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                    db.query_row("SELECT count(*) FROM calendar_events", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                ))
            })
            .await
            .expect("counts");
        assert_eq!(counts, (1, 1));
    }

    #[tokio::test]
    async fn retired_old_action_never_projects_over_newer_completed_event() {
        let (_directory, db) = database().await;
        let before = event("remote", Some("v1"));
        let mut older = before.clone();
        older.title = "Older".into();
        let mut newer = before.clone();
        newer.title = "Newer".into();
        db.write({
            let before = before.clone();
            let older = older.clone();
            let newer = newer.clone();
            move |db| {
                db.execute(
                    "INSERT OR REPLACE INTO calendar_sources(id,source) VALUES('primary',?1)",
                    [serde_json::to_string(&Source {
                        id: "primary".into(),
                        name: "Personal".into(),
                        read_only: false,
                    })?],
                )?;
                db.execute(
                    "INSERT INTO calendar_events(source_id,id,event) VALUES('primary','remote',?1)",
                    [serde_json::to_string(&before)?],
                )?;
                admit(
                    db,
                    "old",
                    &Mutation::Save {
                        before: Some(before.clone()),
                        after: older,
                    },
                )?;
                db.execute(
                    "UPDATE calendar_actions SET created=1,status='rejected' WHERE id='old'",
                    [],
                )?;
                db.execute("DELETE FROM calendar_intents WHERE action='old'", [])?;
                admit(
                    db,
                    "new",
                    &Mutation::Save {
                        before: Some(before),
                        after: newer.clone(),
                    },
                )?;
                db.execute(
                    "UPDATE calendar_actions SET created=1,status='succeeded' WHERE id='new'",
                    [],
                )?;
                let mut completed = newer.clone();
                completed.etag = Some("v2".into());
                db.execute(
                    "UPDATE calendar_events SET event=?1 WHERE source_id='primary' AND id='remote'",
                    [serde_json::to_string(&completed)?],
                )?;
                db.execute("DELETE FROM calendar_intents WHERE action='new'", [])?;
                Ok(())
            }
        })
        .await
        .expect("state");
        let snapshot = db.read(projected_snapshot).await.expect("snapshot");
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].title, "Newer");
    }

    #[tokio::test]
    async fn edit_receipt_accepts_exact_before_and_rejects_newer_same_content_etag() {
        let (_directory, db) = database().await;
        let before = event("remote", Some("v1"));
        let mut requested = before.clone();
        requested.title = "Edited".into();
        let mut saved = requested.clone();
        saved.etag = Some("v2".into());
        db.write({
            let before = before.clone();
            let requested = requested.clone();
            let saved = saved.clone();
            move |db| {
                db.execute(
                    "INSERT INTO calendar_events(source_id,id,event) VALUES('primary','remote',?1)",
                    [serde_json::to_string(&before)?],
                )?;
                admit(
                    db,
                    "edit",
                    &Mutation::Save {
                        before: Some(before.clone()),
                        after: requested,
                    },
                )?;
                db.execute(
                    "UPDATE calendar_actions SET status='repair' WHERE id='edit'",
                    [],
                )?;
                db.execute(
                    "INSERT INTO calendar_action_receipts(action,receipt) VALUES('edit',?1)",
                    [serde_json::to_string(&Receipt {
                        request_id: "edit".into(),
                        before: Some(before),
                        after: Some(saved),
                    })?],
                )?;
                repair(db, "edit")?;
                Ok(())
            }
        })
        .await
        .expect("repair");
        let mut newer = saved.clone();
        newer.etag = Some("v99".into());
        let mut next = saved.clone();
        next.title = "Next".into();
        let mut acknowledged = next.clone();
        acknowledged.etag = Some("v3".into());
        let refused = db
            .write(move |db| {
                admit(
                    db,
                    "next",
                    &Mutation::Save {
                        before: Some(saved.clone()),
                        after: next,
                    },
                )?;
                db.execute(
                    "UPDATE calendar_actions SET status='repair' WHERE id='next'",
                    [],
                )?;
                db.execute(
                    "INSERT INTO calendar_action_receipts(action,receipt) VALUES('next',?1)",
                    [serde_json::to_string(&Receipt {
                        request_id: "next".into(),
                        before: Some(saved),
                        after: Some(acknowledged.clone()),
                    })?],
                )?;
                db.execute(
                    "UPDATE calendar_events SET event=?1 WHERE source_id='primary' AND id='remote'",
                    [serde_json::to_string(&newer)?],
                )?;
                repair(db, "next")
            })
            .await;
        assert!(refused.is_err());
    }
}
