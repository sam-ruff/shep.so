use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
#[cfg(test)]
use shep_calendar_core::ProviderFailure;
#[cfg(test)]
use shep_calendar_core::http::MockCalendarProvider;
pub use shep_calendar_core::http::{CalendarProvider, GoogleCalendarProvider};
use shep_calendar_core::{Event, FailureKind, Mutation, Receipt, Source};

pub mod connections;

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
    connection_id: Option<String>,
    connection_revision: Option<i64>,
    credential_slot: Option<String>,
    checked: bool,
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

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CalDavAdmission {
    pub id: String,
    pub status: String,
    pub mutation: Mutation,
    pub connection_id: String,
    pub connection_revision: i64,
    pub credential_slot: String,
    pub receipt: Option<Receipt>,
}

type SavedCalDavAdmission = (
    String,
    String,
    Option<String>,
    Option<i64>,
    Option<String>,
    Option<String>,
);

pub fn caldav_action_admission(db: &Connection, id: &str) -> Result<Option<CalDavAdmission>> {
    let saved: Option<SavedCalDavAdmission> = db
        .query_row(
            "SELECT a.status,COALESCE(a.admission_request,a.mutation),a.connection_id,a.connection_revision,a.credential_slot,r.receipt FROM calendar_actions a LEFT JOIN calendar_action_receipts r ON r.action=a.id WHERE a.id=?1",
            [id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
        )
        .optional()?;
    let Some((status, request, connection_id, connection_revision, credential_slot, receipt)) =
        saved
    else {
        return Ok(None);
    };
    Ok(Some(CalDavAdmission {
        id: id.to_owned(),
        status,
        mutation: serde_json::from_str(&request)?,
        connection_id: connection_id.context("This is not a CalDAV action.")?,
        connection_revision: connection_revision
            .context("This CalDAV action has no connection revision.")?,
        credential_slot: credential_slot.context("This CalDAV action has no credential slot.")?,
        receipt: receipt
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
    }))
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
            "SELECT source FROM calendar_sources WHERE id=?1 AND connection_id IS NULL",
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

pub fn admit_caldav(
    db: &mut Connection,
    id: &str,
    mutation: &Mutation,
    connection_id: &str,
    observed_revision: i64,
) -> Result<CalDavAdmission> {
    let tx = db.transaction()?;
    if let Some(saved) = caldav_action_admission(&tx, id)? {
        anyhow::ensure!(
            saved.mutation == *mutation
                && saved.connection_id == connection_id
                && saved.connection_revision == observed_revision,
            "This calendar request identity already belongs to another change."
        );
        tx.commit()?;
        return Ok(saved);
    }
    let connection = connections::active(&tx, connection_id)?;
    anyhow::ensure!(
        connection.revision == observed_revision,
        "This calendar connection changed. Refresh before changing events."
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
            "SELECT source FROM calendar_sources WHERE id=?1 AND connection_id=?2",
            params![requested.source_id, connection_id],
            |row| row.get(0),
        )
        .context("Sync this CalDAV calendar before changing events.")?;
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
    tx.execute(
        "INSERT INTO calendar_actions(id,status,error,created,mutation,subject,admission_request,connection_id,connection_revision,credential_slot) VALUES(?1,'queued',NULL,unixepoch('subsec')*1000,?2,NULL,?3,?4,?5,?6)",
        params![id,serde_json::to_string(&admitted)?,serde_json::to_string(mutation)?,connection_id,observed_revision,connection.credential_slot],
    )?;
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
    Ok(CalDavAdmission {
        id: id.to_owned(),
        status: "queued".into(),
        mutation: mutation.clone(),
        connection_id: connection_id.to_owned(),
        connection_revision: observed_revision,
        credential_slot: connection.credential_slot,
        receipt: None,
    })
}

#[cfg(test)]
fn admit(db: &mut Connection, id: &str, mutation: &Mutation) -> Result<()> {
    admit_bound(db, id, mutation, "test-subject").map(|_| ())
}

pub fn activities(db: &Connection, offset: u32) -> Result<Vec<Activity>> {
    anyhow::ensure!(offset <= 10_000, "Calendar history offset is too large.");
    let mut statement = db.prepare("SELECT a.id,a.status,a.error,a.created,a.subject,a.connection_id,a.connection_revision,a.credential_slot,EXISTS(SELECT 1 FROM calendar_action_observations o WHERE o.action=a.id),a.mutation,r.receipt FROM calendar_actions a INDEXED BY calendar_action_attention LEFT JOIN calendar_action_receipts r ON r.action=a.id WHERE a.status NOT IN ('succeeded','cancelled') ORDER BY a.created DESC,a.id LIMIT ?1 OFFSET ?2")?;
    statement
        .query_map(params![HISTORY_LIMIT, offset], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<i64>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, bool>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        })?
        .map(|row| {
            let (
                id,
                status,
                error,
                created,
                subject,
                connection_id,
                connection_revision,
                credential_slot,
                checked,
                mutation,
                receipt,
            ) = row?;
            Ok(Activity {
                id,
                status,
                error,
                created,
                subject,
                connection_id,
                connection_revision,
                credential_slot,
                checked,
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
        tx.execute("DELETE FROM calendar_events WHERE source_id IN (SELECT id FROM calendar_sources WHERE connection_id IS NULL)", [])?;
        tx.execute("DELETE FROM calendar_sources WHERE connection_id IS NULL", [])?;
        for source in saved_sources {
            tx.execute(
                "INSERT INTO calendar_sources(id,source,connection_id) VALUES(?1,?2,NULL)",
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

pub fn accept_current_state(db: &mut Connection, id: &str) -> Result<()> {
    let tx = db.transaction()?;
    type CheckedState = (
        String,
        String,
        Option<String>,
        i64,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<String>,
    );
    let checked: Option<CheckedState> = tx
        .query_row(
            "SELECT a.status,a.mutation,o.observed,o.cache_revision,o.subject,o.connection_id,o.connection_revision,o.credential_slot FROM calendar_actions a JOIN calendar_action_observations o ON o.action=a.id WHERE a.id=?1",
            [id],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?,row.get(7)?)),
        )
        .optional()?;
    let Some((
        status,
        raw,
        observed,
        cache_revision,
        subject,
        connection_id,
        connection_revision,
        credential_slot,
    )) = checked
    else {
        anyhow::bail!("Check the provider event before accepting its current state.");
    };
    anyhow::ensure!(
        status == "uncertain",
        "Check the provider event before accepting its current state."
    );
    let current_revision: i64 = tx.query_row(
        "SELECT revision FROM calendar_clock WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        current_revision == cache_revision,
        "Calendar data changed after this check. Check the provider event again."
    );
    if let Some(connection_id) = connection_id.as_deref() {
        let active = connections::active(&tx, connection_id)
            .context("Reconnect the checked CalDAV calendar before accepting its state.")?;
        anyhow::ensure!(
            Some(active.revision) == connection_revision
                && Some(active.credential_slot.as_str()) == credential_slot.as_deref(),
            "The CalDAV connection changed after this check. Check it again."
        );
    } else {
        let current_subject: String = tx
            .query_row(
                "SELECT subject FROM calendar_binding WHERE id=1",
                [],
                |row| row.get(0),
            )
            .context("Reconnect the checked Google account before accepting its state.")?;
        anyhow::ensure!(
            subject.as_deref() == Some(current_subject.as_str()),
            "The Google account changed after this check. Check it again."
        );
    }
    let mutation: Mutation = serde_json::from_str(&raw)?;
    let event = match &mutation {
        Mutation::Save { after, .. } => after,
        Mutation::Delete { before } => before,
    };
    let owner: Option<String> = tx
        .query_row(
            "SELECT action FROM calendar_intents WHERE source_id=?1 AND event_id=?2",
            params![event.source_id, event.id],
            |row| row.get(0),
        )
        .optional()?;
    anyhow::ensure!(
        owner.as_deref() == Some(id),
        "A newer calendar change owns this event."
    );
    tx.execute(
        "DELETE FROM calendar_events WHERE source_id=?1 AND id=?2",
        params![event.source_id, event.id],
    )?;
    if let Some(observed) = observed {
        let observed: Event = serde_json::from_str(&observed)?;
        let expected_id = match &mutation {
            Mutation::Save { before: None, .. } if connection_id.is_none() => {
                format!("shep{}", id.replace('-', ""))
            }
            _ => event.id.clone(),
        };
        anyhow::ensure!(
            observed.source_id == event.source_id && observed.id == expected_id,
            "The checked event identity is invalid. Check it again."
        );
        tx.execute(
            "INSERT INTO calendar_events(source_id,id,event) VALUES(?1,?2,?3) ON CONFLICT(source_id,id) DO UPDATE SET event=excluded.event",
            params![observed.source_id, observed.id, serde_json::to_string(&observed)?],
        )?;
    }
    tx.execute("DELETE FROM calendar_intents WHERE action=?1", [id])?;
    tx.execute(
        "DELETE FROM calendar_action_observations WHERE action=?1",
        [id],
    )?;
    tx.execute(
        "UPDATE calendar_actions SET status='cancelled',error='The checked provider state was kept.' WHERE id=?1 AND status='uncertain'",
        [id],
    )?;
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
        let (raw,bound,connection_id,connection_revision,credential_slot):(String,Option<String>,Option<String>,Option<i64>,Option<String>)=tx.query_row("SELECT mutation,subject,connection_id,connection_revision,credential_slot FROM calendar_actions WHERE id=?1 AND status IN ('queued','waiting')", [&saved], |row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).context("This calendar change is no longer runnable.")?;
        if let Some(connection_id)=connection_id.as_deref() {
            let active=connections::active(&tx,connection_id).map_err(|_|ClaimWaiting("Reconnect the original CalDAV calendar to continue this change."))?;
            if Some(active.revision)!=connection_revision||Some(active.credential_slot.as_str())!=credential_slot.as_deref(){return Err(ClaimWaiting("Reconnect the original CalDAV calendar to continue this change.").into());}
        } else {
            let current:String=tx.query_row("SELECT subject FROM calendar_binding WHERE id=1",[],|row|row.get(0)).context("Google Calendar must be reconnected.")?;
            if bound.as_deref()!=Some(subject.as_str())||current!=subject{return Err(ClaimWaiting("Reconnect the original Google account to continue this change.").into());}
        }
        let mutation=serde_json::from_str::<Mutation>(&raw)?;
        let event=match &mutation {Mutation::Save{after,..}=>after,Mutation::Delete{before}=>before};
        let source:String=if let Some(connection_id)=connection_id.as_deref(){tx.query_row("SELECT source FROM calendar_sources WHERE id=?1 AND connection_id=?2",params![&event.source_id,connection_id],|row|row.get(0)).context("This calendar source was removed.")?}else{tx.query_row("SELECT source FROM calendar_sources WHERE id=?1 AND connection_id IS NULL",[&event.source_id],|row|row.get(0)).context("This calendar source was removed.")?};
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
                tx.execute("UPDATE calendar_actions SET status='repair',error='The calendar provider saved this event. Finish saving it on this device.' WHERE id=?1 AND status='running'",[&saved])?;tx.commit()?;Ok(())}).await?;
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

pub fn ensure_google_action(db: &Connection, id: &str) -> Result<()> {
    let google: bool = db.query_row(
        "SELECT subject IS NOT NULL AND connection_id IS NULL FROM calendar_actions WHERE id=?1",
        [id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(google, "This action belongs to another calendar provider.");
    Ok(())
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
            let (status, raw, bound, connection_id, connection_revision, credential_slot): (String, String, Option<String>,Option<String>,Option<i64>,Option<String>) = db.query_row(
                "SELECT status,mutation,subject,connection_id,connection_revision,credential_slot FROM calendar_actions WHERE id=?1",
                [saved],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
            )?;
            anyhow::ensure!(
                status == "uncertain",
                "This calendar change does not need inspection."
            );
            if let Some(connection_id)=connection_id.as_deref() {
                let active=connections::active(db,connection_id).context("Reconnect the original CalDAV calendar to inspect this change.")?;
                anyhow::ensure!(Some(active.revision)==connection_revision&&Some(active.credential_slot.as_str())==credential_slot.as_deref(),"Reconnect the original CalDAV calendar to inspect this change.");
            } else {
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
            }
            let cache_revision = db.query_row(
                "SELECT revision FROM calendar_clock WHERE id=1",
                [],
                |row| row.get::<_, i64>(0),
            )?;
            Ok((
                serde_json::from_str::<Mutation>(&raw)?,
                bound,
                connection_id,
                connection_revision,
                credential_slot,
                cache_revision,
            ))
        })
        .await?;
    let (mutation, bound, connection_id, connection_revision, credential_slot, cache_revision) =
        mutation;
    let caldav = connection_id.is_some();
    let mut expected = match &mutation {
        Mutation::Save { after, .. } => after.clone(),
        Mutation::Delete { before } => before.clone(),
    };
    if expected.is_create() && !caldav {
        expected.id = format!("shep{}", id.replace('-', ""));
    }
    let observed = provider.read(&token, &expected).await?;
    if let Some(event) = &observed {
        anyhow::ensure!(
            event.is_bounded()
                && event.end > event.start
                && event.id == expected.id
                && event.source_id == expected.source_id
                && event.etag.as_ref().is_some_and(|value| !value.is_empty())
                && expected
                    .remote_url
                    .as_ref()
                    .is_none_or(|url| event.remote_url.as_ref() == Some(url)),
            "The provider did not return the exact event and its current version."
        );
    }
    let saved = id;
    let event = match &mutation {
        Mutation::Save { after, .. } => after,
        Mutation::Delete { before } => before,
    };
    let source_id = event.source_id.clone();
    let event_id = event.id.clone();
    db.write(move |db| {
        let tx = db.transaction()?;
        let current: (String, Option<String>, Option<String>, Option<i64>, Option<String>, i64) = tx.query_row(
            "SELECT a.status,a.subject,a.connection_id,a.connection_revision,a.credential_slot,c.revision FROM calendar_actions a CROSS JOIN calendar_clock c WHERE a.id=?1 AND c.id=1",
            [&saved],
            |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
        )?;
        anyhow::ensure!(
            current == (
                "uncertain".into(),
                bound.clone(),
                connection_id.clone(),
                connection_revision,
                credential_slot.clone(),
                cache_revision,
            ),
            "Calendar data changed while the provider event was checked. Check it again."
        );
        let owner: Option<String> = tx
            .query_row(
                "SELECT action FROM calendar_intents WHERE source_id=?1 AND event_id=?2",
                params![source_id, event_id],
                |row| row.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            owner.as_deref() == Some(saved.as_str()),
            "A newer calendar change owns this event."
        );
        let observed = observed
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        tx.execute(
            "INSERT INTO calendar_action_observations(action,observed,cache_revision,subject,connection_id,connection_revision,credential_slot) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(action) DO UPDATE SET observed=excluded.observed,cache_revision=excluded.cache_revision,subject=excluded.subject,connection_id=excluded.connection_id,connection_revision=excluded.connection_revision,credential_slot=excluded.credential_slot",
            params![saved,observed,cache_revision,bound,connection_id,connection_revision,credential_slot],
        )?;
        tx.execute("UPDATE calendar_actions SET error='The provider state was checked. Review it before accepting; the original outcome remains unconfirmed.' WHERE id=?1 AND status='uncertain'",[&saved])?;
        tx.commit()?;
        Ok(())
    }).await?;
    Ok(())
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
    async fn caldav_admission_freezes_connection_revision_and_credential_slot() {
        let (directory, db) = database().await;
        let request = connections::ConnectionRequest {
            connection: shep_calendar_core::caldav::CalDavConnection {
                id: "caldav-home".into(),
                url: "https://calendar.example.test/home/".into(),
                username: "sam".into(),
            },
            credential_slot: "calendar-setup".into(),
            observed_revision: None,
            observed_credential_slot: None,
        };
        db.write({
            let request = request.clone();
            move |db| {
                connections::prepare(db, "setup", &request)?;
                connections::claim_probe(db, "setup")?;
                connections::activate(
                    db,
                    "setup",
                    &request,
                    &[Source {
                        id: "caldav-home".into(),
                        name: "Home".into(),
                        read_only: false,
                    }],
                    &[],
                )?;
                Ok(())
            }
        })
        .await
        .expect("connection");
        let mut requested = event("local", None);
        requested.source_id = "caldav-home".into();
        let mutation = Mutation::Save {
            before: None,
            after: requested,
        };
        let admission = db
            .write({
                let mutation = mutation.clone();
                move |db| admit_caldav(db, "action", &mutation, "caldav-home", 1)
            })
            .await
            .expect("admission");
        assert_eq!(admission.connection_revision, 1);
        assert_eq!(admission.credential_slot, "calendar-setup");
        let reconciled = db
            .read(move |db| {
                assert!(ensure_google_action(db, "action").is_err());
                caldav_action_admission(db, "action")
            })
            .await
            .expect("lookup")
            .expect("saved");
        assert_eq!(reconciled, admission);
        drop(db);
        drop(directory);
    }

    #[tokio::test]
    async fn caldav_action_dispatch_rechecks_frozen_connection_and_saves_receipt() {
        let (directory, db) = database().await;
        let request = connections::ConnectionRequest {
            connection: shep_calendar_core::caldav::CalDavConnection {
                id: "caldav-home".into(),
                url: "https://calendar.example.test/home/".into(),
                username: "sam".into(),
            },
            credential_slot: "calendar-setup".into(),
            observed_revision: None,
            observed_credential_slot: None,
        };
        db.write({
            let request = request.clone();
            move |db| {
                connections::prepare(db, "setup", &request)?;
                connections::claim_probe(db, "setup")?;
                connections::activate(
                    db,
                    "setup",
                    &request,
                    &[Source {
                        id: "caldav-home".into(),
                        name: "Home".into(),
                        read_only: false,
                    }],
                    &[],
                )?;
                Ok(())
            }
        })
        .await
        .expect("connection");
        let mut requested = event("local", None);
        requested.source_id = "caldav-home".into();
        let mutation = Mutation::Save {
            before: None,
            after: requested.clone(),
        };
        db.write({
            let mutation = mutation.clone();
            move |db| admit_caldav(db, "action", &mutation, "caldav-home", 1).map(|_| ())
        })
        .await
        .expect("admission");
        let mut provider = MockCalendarProvider::new();
        provider
            .expect_save()
            .with(eq("secret"), eq("action"), eq(requested.clone()))
            .times(1)
            .return_once(|_, _, event| {
                let mut event = event.clone();
                event.etag = Some("\"v1\"".into());
                event.remote_url = Some("local.ics".into());
                Box::pin(async move { Ok(event) })
            });
        execute_bound(
            &db,
            &provider,
            "action".into(),
            "secret".into(),
            String::new(),
        )
        .await
        .expect("execute");
        let saved = db
            .read(|db| {
                Ok(db.query_row(
                    "SELECT status,EXISTS(SELECT 1 FROM calendar_action_receipts WHERE action='action') FROM calendar_actions WHERE id='action'",
                    [],
                    |row| Ok((row.get::<_,String>(0)?,row.get::<_,bool>(1)?)),
                )?)
            })
            .await
            .expect("saved");
        assert_eq!(saved, ("succeeded".into(), true));
        drop(db);
        drop(directory);
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
        assert_eq!(status, "uncertain");
        db.write(|db| accept_current_state(db, "action"))
            .await
            .expect("explicit adoption");
        let state = db
            .read(|db| {
                Ok((
                    db.query_row(
                        "SELECT status FROM calendar_actions WHERE id='action'",
                        [],
                        |row| row.get::<_, String>(0),
                    )?,
                    db.query_row("SELECT count(*) FROM calendar_action_receipts", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                ))
            })
            .await
            .expect("adopted state");
        assert_eq!(state, ("cancelled".into(), 0));
    }

    #[tokio::test]
    async fn nonmatching_inspection_retains_uncertainty_and_local_ownership() {
        for missing in [true, false] {
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
                .expect("uncertain");
            let mut observed = requested;
            observed.id = "shepaction".into();
            observed.title = "A later server edit".into();
            observed.etag = Some("v2".into());
            observed.remote_url = Some("shepaction".into());
            provider.expect_read().times(1).return_once(move |_, _| {
                Box::pin(async move { Ok((!missing).then_some(observed)) })
            });
            inspect(&db, &provider, "action".into(), "token".into())
                .await
                .expect("inspect");
            let state = db.read(|db| {
                Ok(db.query_row(
                    "SELECT status,EXISTS(SELECT 1 FROM calendar_intents WHERE action='action'),EXISTS(SELECT 1 FROM calendar_action_receipts WHERE action='action') FROM calendar_actions WHERE id='action'",
                    [], |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?, row.get::<_, bool>(2)?))
                )?)
            }).await.expect("state");
            assert_eq!(state, ("uncertain".into(), true, false));
            assert!(
                execute(&db, &provider, "action".into(), "token".into())
                    .await
                    .is_err()
            );
        }
    }

    #[tokio::test]
    async fn inspection_rejects_another_identity_or_missing_version() {
        for invalid in ["id", "source", "etag"] {
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
                    )?;
                    db.execute(
                        "UPDATE calendar_actions SET status='uncertain' WHERE id='action'",
                        [],
                    )?;
                    Ok(())
                }
            })
            .await
            .expect("uncertain admission");
            let mut observed = requested;
            observed.id = "shepaction".into();
            observed.etag = Some("v2".into());
            match invalid {
                "id" => observed.id = "another-event".into(),
                "source" => observed.source_id = "another-calendar".into(),
                _ => observed.etag = None,
            }
            let mut provider = MockCalendarProvider::new();
            provider
                .expect_read()
                .times(1)
                .return_once(move |_, _| Box::pin(async move { Ok(Some(observed)) }));
            assert!(
                inspect(&db, &provider, "action".into(), "token".into())
                    .await
                    .is_err()
            );
            let state = db
                .read(|db| {
                    Ok((
                        db.query_row(
                            "SELECT status FROM calendar_actions WHERE id='action'",
                            [],
                            |row| row.get::<_, String>(0),
                        )?,
                        db.query_row(
                            "SELECT count(*) FROM calendar_action_observations",
                            [],
                            |row| row.get::<_, i64>(0),
                        )?,
                        db.query_row("SELECT count(*) FROM calendar_action_receipts", [], |row| {
                            row.get::<_, i64>(0)
                        })?,
                    ))
                })
                .await
                .expect("retained state");
            assert_eq!(state, ("uncertain".into(), 0, 0));
        }
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
        assert_eq!(state, ("uncertain".into(), 1));
        db.write(|db| accept_current_state(db, "delete"))
            .await
            .expect("explicit absence adoption");
        let remaining = db
            .read(|db| {
                Ok((
                    db.query_row("SELECT count(*) FROM calendar_events", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                    db.query_row("SELECT count(*) FROM calendar_action_receipts", [], |row| {
                        row.get::<_, i64>(0)
                    })?,
                ))
            })
            .await
            .expect("adopted absence");
        assert_eq!(remaining, (0, 0));
    }

    #[tokio::test]
    async fn discovery_commits_bounded_sources_with_events() {
        let (_directory, db) = database().await;
        db.write(|db| {
            db.execute("INSERT INTO calendar_connections(id,config,credential_slot,revision) VALUES('caldav-home','{}','calendar-existing',1)",[])?;
            db.execute("INSERT INTO calendar_sources(id,source,connection_id) VALUES('caldav-home',?1,'caldav-home')",[serde_json::to_string(&Source{id:"caldav-home".into(),name:"Home".into(),read_only:false})?])?;
            Ok(())
        }).await.expect("CalDAV source");
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
        assert_eq!(counts, (2, 1));
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

    #[tokio::test]
    async fn inspected_different_or_absent_state_is_applied_only_after_adoption() {
        for missing in [false, true] {
            let (_directory, db) = database().await;
            let before = event("remote", Some("v1"));
            let mut requested = before.clone();
            requested.title = "Requested".into();
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
                        "action",
                        &Mutation::Save {
                            before: Some(before),
                            after: requested,
                        },
                    )?;
                    db.execute(
                        "UPDATE calendar_actions SET status='uncertain' WHERE id='action'",
                        [],
                    )?;
                    Ok(())
                }
            })
            .await
            .expect("uncertain action");
            let mut observed = before;
            observed.title = "Provider state".into();
            observed.etag = Some("v2".into());
            let expected = (!missing).then_some(observed.clone());
            let mut provider = MockCalendarProvider::new();
            provider.expect_read().times(1).return_once(move |_, _| {
                Box::pin(async move { Ok((!missing).then_some(observed)) })
            });

            inspect(&db, &provider, "action".into(), "token".into())
                .await
                .expect("inspect");
            let activity = db.read(|db| activities(db, 0)).await.expect("activity");
            assert!(activity[0].checked);
            db.write(|db| accept_current_state(db, "action"))
                .await
                .expect("adopt");
            let cached = db
                .read(|db| {
                    let raw: Option<String> = db
                        .query_row(
                            "SELECT event FROM calendar_events WHERE source_id='primary' AND id='remote'",
                            [],
                            |row| row.get(0),
                        )
                        .optional()?;
                    raw.as_deref()
                        .map(serde_json::from_str::<Event>)
                        .transpose()
                        .map_err(Into::into)
                })
                .await
                .expect("cache");
            assert_eq!(cached, expected);
        }
    }

    #[tokio::test]
    async fn checked_observation_survives_restart_without_provider_replay() {
        let (directory, db) = database().await;
        let path = directory
            .path()
            .join("calendar.sqlite")
            .to_string_lossy()
            .into_owned();
        let before = event("remote", Some("v1"));
        let mut requested = before.clone();
        requested.title = "Requested".into();
        db.write({
            let before = before.clone();
            move |db| {
                db.execute(
                    "INSERT INTO calendar_events(source_id,id,event) VALUES('primary','remote',?1)",
                    [serde_json::to_string(&before)?],
                )?;
                admit(
                    db,
                    "action",
                    &Mutation::Save {
                        before: Some(before),
                        after: requested,
                    },
                )?;
                db.execute(
                    "UPDATE calendar_actions SET status='uncertain' WHERE id='action'",
                    [],
                )?;
                Ok(())
            }
        })
        .await
        .expect("uncertain action");
        let mut observed = before;
        observed.title = "Provider state".into();
        observed.etag = Some("v2".into());
        let mut provider = MockCalendarProvider::new();
        provider
            .expect_read()
            .times(1)
            .return_once(move |_, _| Box::pin(async move { Ok(Some(observed)) }));
        inspect(&db, &provider, "action".into(), "token".into())
            .await
            .expect("inspect");
        drop(db);

        let reopened = crate::database::Database::open(path).await.expect("reopen");
        assert!(
            reopened
                .read(|db| activities(db, 0))
                .await
                .expect("activity")[0]
                .checked
        );
        reopened
            .write(|db| accept_current_state(db, "action"))
            .await
            .expect("adopt after restart");
    }

    #[tokio::test]
    async fn provider_read_cannot_publish_an_observation_across_a_newer_revision() {
        for matching in [false, true] {
            let (_directory, db) = database().await;
            let requested = event("local", None);
            let mut matching_event = requested.clone();
            matching_event.id = "shepaction".into();
            matching_event.etag = Some("v2".into());
            matching_event.remote_url = Some("shepaction".into());
            db.write(move |db| {
                admit(
                    db,
                    "action",
                    &Mutation::Save {
                        before: None,
                        after: requested,
                    },
                )?;
                db.execute(
                    "UPDATE calendar_actions SET status='uncertain' WHERE id='action'",
                    [],
                )?;
                Ok(())
            })
            .await
            .expect("uncertain action");
            let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let mut provider = MockCalendarProvider::new();
            provider.expect_read().times(1).return_once(move |_, _| {
                Box::pin(async move {
                    let _ = entered_tx.send(());
                    let _ = release_rx.await;
                    Ok(matching.then_some(matching_event))
                })
            });
            let inspected_db = db.clone();
            let inspection = tokio::spawn(async move {
                inspect(&inspected_db, &provider, "action".into(), "token".into()).await
            });
            entered_rx.await.expect("provider read entered");
            db.write(|db| {
                db.execute(
                    "UPDATE calendar_clock SET revision=revision+1 WHERE id=1",
                    [],
                )?;
                Ok(())
            })
            .await
            .expect("newer revision");
            release_tx.send(()).expect("release read");
            assert!(inspection.await.expect("inspection task").is_err());
            let observations = db
                .read(|db| {
                    db.query_row(
                        "SELECT count(*) FROM calendar_action_observations",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(Into::into)
                })
                .await
                .expect("observations");
            assert_eq!(observations, 0);
        }
    }

    #[tokio::test]
    async fn checked_current_state_retires_only_the_owned_uncertain_intent() {
        let mut provider_event = event("shepaction", Some("v2"));
        provider_event.source_id = "primary".into();
        for observed in [Some(provider_event), None] {
            let (_directory, db) = database().await;
            let requested = event("local", None);
            let expected = observed.clone();
            db.write(move |db| {
                admit(
                    db,
                    "action",
                    &Mutation::Save {
                        before: None,
                        after: requested,
                    },
                )?;
                let revision: i64 = db.query_row(
                    "SELECT revision FROM calendar_clock WHERE id=1",
                    [],
                    |row| row.get(0),
                )?;
                db.execute(
                    "UPDATE calendar_actions SET status='uncertain',error='The provider no longer matches the requested event.' WHERE id='action'",
                    [],
                )?;
                db.execute(
                    "INSERT INTO calendar_action_observations(action,observed,cache_revision,subject) VALUES('action',?1,?2,'test-subject')",
                    params![observed.as_ref().map(serde_json::to_string).transpose()?,revision],
                )?;
                accept_current_state(db, "action")
            })
            .await
            .expect("accept checked state");
            let state = db
                .read(|db| {
                    let cached: Option<String> = db
                        .query_row(
                            "SELECT event FROM calendar_events WHERE source_id='primary' AND id IN ('local','shepaction') LIMIT 1",
                            [],
                            |row| row.get(0),
                        )
                        .optional()?;
                    Ok((
                        db.query_row(
                            "SELECT status FROM calendar_actions WHERE id='action'",
                            [],
                            |row| row.get::<_, String>(0),
                        )?,
                        db.query_row(
                            "SELECT count(*) FROM calendar_intents WHERE action='action'",
                            [],
                            |row| row.get::<_, i64>(0),
                        )?,
                        cached
                            .as_deref()
                            .map(serde_json::from_str::<Event>)
                            .transpose()?,
                    ))
                })
                .await
                .expect("state");
            assert_eq!(state, ("cancelled".into(), 0, expected));
        }
    }

    #[tokio::test]
    async fn stale_checked_state_cannot_replace_newer_calendar_data() {
        let (_directory, db) = database().await;
        let requested = event("local", None);
        db.write(move |db| {
            admit(
                db,
                "action",
                &Mutation::Save {
                    before: None,
                    after: requested,
                },
            )?;
            let revision: i64 = db.query_row(
                "SELECT revision FROM calendar_clock WHERE id=1",
                [],
                |row| row.get(0),
            )?;
            db.execute(
                "UPDATE calendar_actions SET status='uncertain' WHERE id='action'",
                [],
            )?;
            db.execute(
                "INSERT INTO calendar_action_observations(action,observed,cache_revision,subject) VALUES('action',NULL,?1,'test-subject')",
                [revision],
            )?;
            db.execute(
                "UPDATE calendar_clock SET revision=revision+1 WHERE id=1",
                [],
            )?;
            Ok(())
        })
        .await
        .expect("checked state");

        assert!(
            db.write(|db| accept_current_state(db, "action"))
                .await
                .is_err()
        );
        let ownership = db
            .read(|db| {
                db.query_row(
                    "SELECT action FROM calendar_intents WHERE source_id='primary' AND event_id='local'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .map_err(Into::into)
            })
            .await
            .expect("ownership");
        assert_eq!(ownership, "action");
    }
}
