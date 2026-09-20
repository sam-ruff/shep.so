use super::*;
use rusqlite::OptionalExtension;
use serde::Deserialize;
use shep_action_core::Status;

pub const CAPACITY: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarJob {
    pub id: String,
    pub origin: String,
    pub source: CalendarSource,
    pub event: CalendarEvent,
    pub request: CalendarEvent,
    pub deleting: bool,
    pub status: String,
    pub receipt: Option<CalendarEvent>,
    pub cache_applied: bool,
    pub checked: bool,
    pub observed: Option<CalendarEvent>,
    pub revision: u64,
    pub previous: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub wait_reason: Option<crate::providers::calendar::WaitReason>,
    #[serde(default)]
    pub retry_at: Option<i64>,
}

pub(crate) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS calendar_actions (
        id TEXT PRIMARY KEY, origin TEXT NOT NULL, source TEXT NOT NULL,
        status TEXT NOT NULL, previous TEXT, data TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS calendar_action_status ON calendar_actions(status);
        CREATE INDEX IF NOT EXISTS calendar_action_origin ON calendar_actions(origin);
        CREATE INDEX IF NOT EXISTS calendar_action_source ON calendar_actions(source);
        CREATE INDEX IF NOT EXISTS calendar_action_previous ON calendar_actions(previous);",
    )?;
    Ok(())
}

pub(crate) fn fence_import(c: &Connection, import_id: &str, note: &str) -> anyhow::Result<()> {
    schema(c)?;
    let count: i64 = c.query_row(
        "SELECT COUNT(*) FROM calendar_actions WHERE status NOT IN ('succeeded','cancelled')",
        [],
        |r| r.get(0),
    )?;
    anyhow::ensure!(
        count <= CAPACITY as i64,
        "This calendar journal exceeds the supported capacity."
    );
    for mut job in jobs(c)? {
        c.execute(
            "INSERT INTO imported_operations VALUES(?,'calendar-action',?,?)",
            params![import_id, job.id, serde_json::to_string(&job)?],
        )?;
        if job.status != "repair" {
            job.status = "uncertain".into();
        }
        job.checked = false;
        job.observed = None;
        job.retry_at = None;
        job.wait_reason = None;
        job.error = Some(note.into());
        write(c, &mut job)?;
    }
    Ok(())
}

fn recover(c: &Connection) -> anyhow::Result<()> {
    let interrupted = c
        .prepare("SELECT data FROM calendar_actions WHERE status='running'")?
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for data in interrupted {
        let mut job: CalendarJob = serde_json::from_str(&data)?;
        job.status = Status::parse(&job.status)
            .context("Unknown calendar journal status")?
            .after_restart()
            .key()
            .into();
        job.error = Some("The previous session ended before the server result was recorded. Check the event before continuing.".into());
        write(c, &mut job)?;
    }
    Ok(())
}

fn read(c: &Connection, id: &str) -> anyhow::Result<CalendarJob> {
    let data: String = c.query_row("SELECT data FROM calendar_actions WHERE id=?", [id], |r| {
        r.get(0)
    })?;
    Ok(serde_json::from_str(&data)?)
}

fn jobs(c: &Connection) -> anyhow::Result<Vec<CalendarJob>> {
    c.prepare("SELECT data FROM calendar_actions WHERE status NOT IN ('succeeded','cancelled') ORDER BY rowid LIMIT 32")?
        .query_map([],|r|r.get::<_,String>(0))?.map(|row|Ok(serde_json::from_str(&row?)?)).collect()
}

fn rebind_children(c: &Connection, job: &CalendarJob) -> anyhow::Result<()> {
    let Some(receipt) = job.receipt.as_ref() else {
        return Ok(());
    };
    let ids = c
        .prepare("SELECT id FROM calendar_actions WHERE previous=? AND status='queued'")?
        .query_map([&job.id], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in ids {
        let mut child = read(c, &id)?;
        child.event.id = receipt.id.clone();
        child.event.etag = receipt.etag.clone();
        child.event.remote_url = receipt.remote_url.clone();
        write(c, &mut child)?;
    }
    Ok(())
}

fn write(c: &Connection, job: &mut CalendarJob) -> anyhow::Result<()> {
    anyhow::ensure!(
        Status::parse(&job.status).is_some(),
        "Unknown calendar journal status"
    );
    let revision: u64 = get(c, "calendar_action_revision")?;
    job.revision = revision + 1;
    put(c, "calendar_action_revision", &job.revision)?;
    c.execute(
        "INSERT INTO calendar_actions(id,origin,source,status,previous,data) VALUES(?,?,?,?,?,?)
        ON CONFLICT(id) DO UPDATE SET status=excluded.status,data=excluded.data",
        params![
            job.id,
            job.origin,
            job.source.id,
            job.status,
            job.previous,
            serde_json::to_string(job)?
        ],
    )?;
    Ok(())
}

fn cache(
    c: &Connection,
    original: &CalendarEvent,
    current: Option<&CalendarEvent>,
) -> anyhow::Result<()> {
    connections::allow(c, ConnectionKind::Calendar, &original.source_id)?;
    anyhow::ensure!(
        current.is_none_or(|event| event.source_id == original.source_id),
        "The checked event belongs to another calendar."
    );
    c.execute(
        "DELETE FROM events WHERE source=? AND json_extract(data,'$.id')=?",
        params![original.source_id, original.id],
    )?;
    if let Some(event) = current {
        c.execute(
            "INSERT OR REPLACE INTO events VALUES(?,?,?,?)",
            params![
                event.key(),
                event.source_id,
                event.start.timestamp(),
                serde_json::to_string(event)?
            ],
        )?;
    }
    calendar_changed(c)
}

impl Store {
    pub async fn recover_calendar_actions(&self) -> anyhow::Result<()> {
        self.run(|c| {
            let tx = c.transaction()?;
            recover(&tx)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn admit_calendar_action(
        &self,
        id: String,
        event: CalendarEvent,
        deleting: bool,
    ) -> anyhow::Result<CalendarJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            if let Some(data) = tx.query_row("SELECT data FROM calendar_actions WHERE id=?", [&id], |r| r.get::<_,String>(0)).optional()? {
                let job: CalendarJob = serde_json::from_str(&data)?;
                anyhow::ensure!(serde_json::to_string(&job.request)? == serde_json::to_string(&event)? && job.deleting == deleting, "This calendar request ID already belongs to another change.");
                return Ok(job);
            }
            anyhow::ensure!(deleting || event.end > event.start, "The event must end after it starts.");
            connections::allow(&tx, ConnectionKind::Calendar, &event.source_id)?;
            let source = get::<Vec<CalendarSource>>(&tx,"calendars")?.into_iter().find(|source| source.id == event.source_id).context("Choose a connected calendar")?;
            crate::providers::calendar::ensure_event_access(&source,&event,deleting)?;
            let previous = tx.query_row("SELECT id FROM calendar_actions WHERE origin=? OR json_extract(data,'$.event.id')=? AND source=? ORDER BY rowid DESC LIMIT 1", params![event.key(),event.id,event.source_id],|r|r.get::<_,String>(0)).optional()?;
            if let Some(previous)=&previous {
                let mut before=read(&tx,previous)?;
                if before.status=="rejected" {before.status="cancelled".into();write(&tx,&mut before)?;}
            }
            let count: i64 = tx.query_row("SELECT COUNT(*) FROM calendar_actions WHERE status NOT IN ('succeeded','cancelled')",[],|r|r.get(0))?;
            anyhow::ensure!(count < CAPACITY as i64, "Finish or review an existing calendar change before adding another.");
            let mut job = CalendarJob { id, origin:event.key(), source, request:event.clone(), event, deleting, status:"queued".into(), receipt:None, cache_applied:false, checked:false, observed:None, revision:0, previous, error:None, attempts:0, wait_reason:None, retry_at:None };
            write(&tx,&mut job)?;
            tx.commit()?;
            Ok(job)
        }).await
    }

    pub async fn calendar_jobs(&self) -> anyhow::Result<Vec<CalendarJob>> {
        self.run(|c| jobs(c)).await
    }

    pub async fn calendar_action_snapshot(
        &self,
    ) -> anyhow::Result<(u64, u64, Vec<CalendarEvent>, Vec<CalendarJob>)> {
        self.run(|c| {
            let events = c
                .prepare("SELECT data FROM events WHERE NOT EXISTS(SELECT 1 FROM connection_tombstones t WHERE t.kind='calendar' AND t.id=events.source) ORDER BY start")?
                .query_map([], |r| r.get::<_, String>(0))?
                .map(|row| Ok(serde_json::from_str(&row?)?))
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok((
                get(c, "calendar_revision")?,
                get(c, "calendar_action_revision")?,
                events,
                jobs(c)?,
            ))
        })
        .await
    }

    pub async fn calendar_job(&self, id: String) -> anyhow::Result<CalendarJob> {
        self.run(move |c| read(c, &id)).await
    }

    pub async fn next_calendar_action(&self) -> anyhow::Result<Option<String>> {
        self.run(|c| Ok(c.query_row("SELECT a.id FROM calendar_actions a LEFT JOIN calendar_actions p ON p.id=a.previous
            WHERE a.status='repair' AND json_extract(a.data,'$.checked')=0 AND json_extract(a.data,'$.cache_applied')=0 OR (a.status='queued' OR a.status='waiting' AND json_extract(a.data,'$.retry_at')<=unixepoch()) AND (p.id IS NULL OR p.status IN ('succeeded','rejected','cancelled')) ORDER BY a.rowid LIMIT 1",[],|r|r.get(0)).optional()?)).await
    }

    pub async fn claim_calendar_action(&self, id: String) -> anyhow::Result<CalendarJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = read(&tx, &id)?;
            anyhow::ensure!(
                job.status == "queued"
                    || job.status == "waiting"
                        && job
                            .retry_at
                            .is_some_and(|at| at <= chrono::Utc::now().timestamp()),
                "This calendar change is no longer waiting."
            );
            if let Some(previous) = &job.previous {
                let before = read(&tx, previous)?;
                anyhow::ensure!(
                    matches!(
                        before.status.as_str(),
                        "succeeded" | "rejected" | "cancelled"
                    ),
                    "An earlier change needs checking first."
                );
                if (before.status == "succeeded" || before.checked)
                    && let Some(receipt) = before.receipt.or(before.observed)
                {
                    job.event.id = receipt.id;
                    job.event.etag = receipt.etag;
                    job.event.remote_url = receipt.remote_url;
                }
            }
            let source = get::<Vec<CalendarSource>>(&tx, "calendars")?
                .into_iter()
                .find(|source| source.id == job.source.id)
                .context("This calendar was removed")?;
            anyhow::ensure!(
                source == job.source,
                "The calendar connection changed. Review this event before trying again."
            );
            connections::allow(&tx, ConnectionKind::Calendar, &source.id)?;
            job.status = "running".into();
            job.attempts = job.attempts.saturating_add(1);
            job.retry_at = None;
            job.wait_reason = None;
            job.error = None;
            write(&tx, &mut job)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }

    pub async fn next_calendar_retry(&self) -> anyhow::Result<Option<i64>> {
        self.next_unreserved_calendar_retry(Vec::new()).await
    }

    pub(crate) async fn next_unreserved_calendar_retry(
        &self,
        occupied: Vec<String>,
    ) -> anyhow::Result<Option<i64>> {
        self.run(move |c| {
            Ok(c.query_row(
                "SELECT min(json_extract(a.data,'$.retry_at')) FROM calendar_actions a
            LEFT JOIN calendar_actions p ON p.id=a.previous WHERE a.status='waiting'
            AND 'calendar:'||a.source NOT IN (SELECT value FROM json_each(?))
            AND NOT EXISTS(SELECT 1 FROM scratch.action_backoff b WHERE b.domain=2 AND b.id=a.id AND b.until>unixepoch())
            AND (p.id IS NULL OR p.status IN ('succeeded','rejected','cancelled'))",
                [serde_json::to_string(&occupied)?],
                |r| r.get(0),
            )?)
        })
        .await
    }

    pub async fn wait_calendar_action(
        &self,
        id: String,
        expected: u64,
        reason: crate::providers::calendar::WaitReason,
        error: String,
    ) -> anyhow::Result<CalendarJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = read(&tx, &id)?;
            anyhow::ensure!(
                job.status == "running" && job.revision == expected,
                "This calendar attempt changed."
            );
            job.status = "waiting".into();
            job.wait_reason = Some(reason);
            job.retry_at =
                if reason == crate::providers::calendar::WaitReason::Offline && job.attempts < 6 {
                    Some(
                        chrono::Utc::now().timestamp()
                            + 5 * (1_i64 << job.attempts.saturating_sub(1).min(5)),
                    )
                } else {
                    None
                };
            job.error = Some(error);
            write(&tx, &mut job)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }

    pub async fn retry_calendar_action(
        &self,
        id: String,
        expected: u64,
    ) -> anyhow::Result<CalendarJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = read(&tx, &id)?;
            anyhow::ensure!(
                job.status == "waiting" && job.revision == expected,
                "Only a change known not to have reached the server can retry."
            );
            job.status = "queued".into();
            job.attempts = 0;
            job.retry_at = None;
            job.wait_reason = None;
            job.error = None;
            write(&tx, &mut job)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }

    pub async fn record_calendar_receipt(
        &self,
        id: String,
        expected: u64,
        receipt: CalendarEvent,
    ) -> anyhow::Result<CalendarJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = read(&tx, &id)?;
            anyhow::ensure!(
                job.status == "running" && job.revision == expected,
                "The calendar attempt changed before its receipt was saved."
            );
            anyhow::ensure!(
                receipt.source_id == job.source.id,
                "The receipt belongs to another calendar."
            );
            job.receipt = Some(receipt);
            job.status = "repair".into();
            write(&tx, &mut job)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }

    pub async fn fail_calendar_action(
        &self,
        id: String,
        expected: u64,
        uncertain: bool,
        error: String,
    ) -> anyhow::Result<CalendarJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = read(&tx, &id)?;
            anyhow::ensure!(
                job.revision == expected
                    && matches!(job.status.as_str(), "running" | "queued" | "waiting"),
                "The calendar attempt changed before its failure was saved."
            );
            job.status = if uncertain { "uncertain" } else { "rejected" }.into();
            job.error = Some(error);
            write(&tx, &mut job)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }

    pub async fn apply_calendar_receipt(&self, id: String) -> anyhow::Result<CalendarJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = read(&tx, &id)?;
            anyhow::ensure!(
                job.status == "repair" && !job.checked && !job.cache_applied,
                "This event has no pending acknowledged receipt."
            );
            let receipt = job
                .receipt
                .as_ref()
                .context("The acknowledged calendar receipt is missing")?;
            cache(&tx, &job.event, (!job.deleting).then_some(receipt))?;
            job.cache_applied = true;
            let needs_check = !job.deleting && receipt.remote_url.is_some() && receipt.etag.is_none();
            job.status = if needs_check { "repair" } else { "succeeded" }.into();
            job.error = needs_check.then(|| "Saved on the server. Check this event before making another change because its server version is missing.".into());
            write(&tx, &mut job)?;
            rebind_children(&tx, &job)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }

    pub async fn check_calendar_action(
        &self,
        id: String,
        expected: u64,
        current: Option<CalendarEvent>,
    ) -> anyhow::Result<CalendarJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = read(&tx, &id)?;
            anyhow::ensure!(
                job.revision == expected && matches!(job.status.as_str(), "uncertain" | "repair"),
                "This calendar review changed. Check the event again."
            );
            cache(&tx, &job.event, current.as_ref())?;
            job.checked = true;
            job.observed = current;
            write(&tx, &mut job)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }

    pub async fn resolve_calendar_action(
        &self,
        id: String,
        expected: u64,
    ) -> anyhow::Result<CalendarJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = read(&tx, &id)?;
            anyhow::ensure!(
                job.revision == expected,
                "This calendar review changed. Review it again."
            );
            anyhow::ensure!(
                matches!(job.status.as_str(), "rejected" | "waiting")
                    || job.checked && matches!(job.status.as_str(), "uncertain" | "repair"),
                "Check the server before accepting this result."
            );
            job.status = "cancelled".into();
            if job.checked {
                job.receipt = job.observed.clone();
            }
            job.error = None;
            write(&tx, &mut job)?;
            rebind_children(&tx, &job)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }
}
