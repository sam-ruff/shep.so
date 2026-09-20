use super::*;
use rusqlite::OptionalExtension;
#[cfg(test)]
mod tests;

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch("CREATE TABLE scratch.action_backoff(domain INTEGER NOT NULL,id TEXT NOT NULL,until INTEGER NOT NULL,PRIMARY KEY(domain,id));
        CREATE INDEX scratch.action_backoff_deadline ON action_backoff(until);")?;
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) enum Work {
    Mail { id: String, position: u64 },
    Folder(String),
    Calendar(String),
    Outgoing(String),
    Creation(String),
    Removal(String),
}

#[derive(Clone, Debug)]
pub(crate) struct ReadyWork {
    pub work: Work,
    pub accounts: Vec<String>,
    pub cursor: String,
}

pub(crate) enum WorkPage {
    Ready(ReadyWork),
    More(String),
    Done,
}

const MAIL_KEYS: &str = "SELECT job,position,CASE WHEN status='repair' THEN '0' ELSE '1' END||job||':'||printf('%020d',position) AS cursor
    FROM bulk_items INDEXED BY bulk_ready_seek
    WHERE status IN ('queued','running','repair')
      AND (CASE WHEN status='repair' THEN '0' ELSE '1' END||job||':'||printf('%020d',position))>?1
      AND (CASE WHEN status='repair' THEN '0' ELSE '1' END||job||':'||printf('%020d',position))<?2
    ORDER BY cursor LIMIT 50";

impl Work {
    pub fn key(&self) -> String {
        match self {
            Self::Mail { id, position } => format!("{id}:{position}"),
            _ => self.id().to_owned(),
        }
    }
    pub fn id(&self) -> &str {
        match self {
            Self::Mail { id, .. }
            | Self::Folder(id)
            | Self::Calendar(id)
            | Self::Outgoing(id)
            | Self::Creation(id)
            | Self::Removal(id) => id,
        }
    }
}

impl Store {
    pub(crate) async fn action_work_progress(
        &self,
        domain: usize,
        id: String,
        changed: bool,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(domain < 6, "Unknown action domain");
        let domain = domain as i64;
        self.run(move |c| {
            if changed {
                c.execute("DELETE FROM scratch.action_backoff WHERE domain=? AND id=? AND until<=unixepoch()",params![domain,id])?;
            } else {
                c.execute("INSERT INTO scratch.action_backoff(domain,id,until) VALUES(?1,?2,unixepoch()+2)
                    ON CONFLICT(domain,id) DO UPDATE SET until=excluded.until",params![domain,id])?;
            }
            Ok(())
        }).await
    }

    pub(crate) async fn expire_action_backoff(&self) -> anyhow::Result<bool> {
        self.run(|c|Ok(c.execute("DELETE FROM scratch.action_backoff WHERE rowid IN (SELECT rowid FROM scratch.action_backoff WHERE until<=unixepoch() ORDER BY until LIMIT 32)",[])? > 0)).await
    }

    pub(crate) async fn next_action_backoff(
        &self,
        blocked: Vec<usize>,
    ) -> anyhow::Result<Option<i64>> {
        self.run(move |c|Ok(c.query_row("SELECT min(until) FROM scratch.action_backoff WHERE domain NOT IN (SELECT value FROM json_each(?))",[serde_json::to_string(&blocked)?],|r|r.get(0))?)).await
    }
    #[cfg(test)]
    pub(crate) async fn next_action_work(
        &self,
        domain: usize,
        mut after: String,
        occupied: Vec<String>,
        active: Vec<String>,
        cache_only: bool,
    ) -> anyhow::Result<Option<ReadyWork>> {
        loop {
            match self
                .scan_action_work(domain, after, occupied.clone(), active.clone(), cache_only)
                .await?
            {
                WorkPage::Ready(work) => return Ok(Some(work)),
                WorkPage::More(cursor) => after = cursor,
                WorkPage::Done => return Ok(None),
            }
        }
    }

    pub(crate) async fn scan_action_work(
        &self,
        domain: usize,
        after: String,
        occupied: Vec<String>,
        active: Vec<String>,
        cache_only: bool,
    ) -> anyhow::Result<WorkPage> {
        self.run(move |c| {
            let occupied = serde_json::to_string(&occupied)?;
            let active = serde_json::to_string(&active)?;
            let parameters = params![after, occupied, active, cache_only];
            let mut continuation = None;
            let row: Option<(String, i64, String, Option<String>, bool)> = match domain {
                0 => {
                    let keys = c.prepare(MAIL_KEYS)?.query_map(params![after, if cache_only { "1" } else { "2" }], |r| Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
                    if keys.len() == 50 { continuation = keys.last().map(|(_,_,cursor)|cursor.clone()); }
                    let mut found = None;
                    for (job, position, _) in keys {
                        found = c.prepare_cached(
                    "SELECT j.id,i.position,
                        COALESCE((SELECT m.account FROM bulk_admissions a JOIN mail_lineage l ON l.lineage=a.lineage JOIN messages m ON m.id=l.id WHERE a.job=i.job AND a.position=i.position),json_extract(i.original,'$.account_id'),''),
                        CASE WHEN i.undo=1 THEN json_extract(i.original,'$.account_id') ELSE json_extract(j.action,'$.Move.account') END,i.status='repair'
                     FROM bulk_jobs j JOIN bulk_items i ON i.job=j.id
                     WHERE i.job=?1 AND i.position=?4 AND j.paused=0 AND (j.id||':'||i.position) NOT IN (SELECT value FROM json_each(?3))
                       AND NOT EXISTS(SELECT 1 FROM scratch.action_backoff b WHERE b.domain=0 AND b.id=j.id AND b.until>unixepoch())
                       AND i.status IN ('queued','running','repair')
                       AND NOT EXISTS(SELECT 1 FROM connection_tombstones t WHERE t.kind='account' AND (t.id=COALESCE((SELECT m.account FROM bulk_admissions a JOIN mail_lineage l ON l.lineage=a.lineage JOIN messages m ON m.id=l.id WHERE a.job=i.job AND a.position=i.position),json_extract(i.original,'$.account_id'), '') OR t.id=json_extract(j.action,'$.Move.account')))
                       AND (i.status IN ('running','repair') OR NOT EXISTS(SELECT 1 FROM bulk_admissions a JOIN bulk_admissions prior ON prior.lineage=a.lineage AND prior.sequence<a.sequence
                           JOIN bulk_items p ON p.job=prior.job AND p.position=prior.position
                           WHERE a.job=i.job AND a.position=i.position AND p.status IN ('queued','running','repair','uncertain')))
                       AND 'mail:'||COALESCE((SELECT m.account FROM bulk_admissions a JOIN mail_lineage l ON l.lineage=a.lineage JOIN messages m ON m.id=l.id WHERE a.job=i.job AND a.position=i.position),json_extract(i.original,'$.account_id'),'') NOT IN (SELECT value FROM json_each(?2))
                       AND (CASE WHEN i.undo=1 THEN json_extract(i.original,'$.account_id') ELSE json_extract(j.action,'$.Move.account') END IS NULL
                         OR 'mail:'||CASE WHEN i.undo=1 THEN json_extract(i.original,'$.account_id') ELSE json_extract(j.action,'$.Move.account') END NOT IN (SELECT value FROM json_each(?2)))
                     LIMIT 1")?.query_row(
                    params![job,occupied,active,position], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
                        if found.is_some() { break; }
                    }
                    found
                },
                1 => c.query_row(
                    "SELECT id,0,account,NULL,0 FROM folder_jobs j WHERE closed=0 AND id>?1 AND ?4=0
                     AND NOT EXISTS(SELECT 1 FROM connection_tombstones t WHERE t.kind='account' AND t.id=j.account)
                     AND NOT EXISTS(SELECT 1 FROM scratch.action_backoff b WHERE b.domain=1 AND b.id=j.id AND b.until>unixepoch())
                     AND 'mail:'||account NOT IN (SELECT value FROM json_each(?2))
                     AND id NOT IN (SELECT value FROM json_each(?3))
                     AND NOT EXISTS(SELECT 1 FROM folder_steps s WHERE s.job=j.id AND json_extract(s.status,'$') IN ('Rejected','Uncertain'))
                     ORDER BY id LIMIT 1", parameters, |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?,
                2 => c.query_row(
                    "SELECT a.id,0,a.source,NULL,a.status='repair' FROM calendar_actions a LEFT JOIN calendar_actions p ON p.id=a.previous
                     WHERE NOT EXISTS(SELECT 1 FROM connection_tombstones t WHERE t.kind='calendar' AND t.id=a.source)
                     AND (CASE WHEN a.status='repair' THEN '0' ELSE '1' END||a.id)>?1 AND (?4=0 OR a.status='repair') AND 'calendar:'||a.source NOT IN (SELECT value FROM json_each(?2))
                     AND NOT EXISTS(SELECT 1 FROM scratch.action_backoff b WHERE b.domain=2 AND b.id=a.id AND b.until>unixepoch())
                     AND a.id NOT IN (SELECT value FROM json_each(?3))
                     AND (a.status='repair' AND json_extract(a.data,'$.checked')=0 AND json_extract(a.data,'$.cache_applied')=0
                         OR (a.status='queued' OR a.status='waiting' AND json_extract(a.data,'$.retry_at')<=unixepoch())
                            AND (p.id IS NULL OR p.status IN ('succeeded','rejected','cancelled')))
                     ORDER BY CASE WHEN a.status='repair' THEN '0' ELSE '1' END||a.id LIMIT 1", parameters, |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?,
                3 => c.query_row(
                    "SELECT attempt,0,account,NULL,0 FROM outgoing WHERE stage IN ('Preparing','Queued') AND attempt>?1 AND ?4=0
                     AND NOT EXISTS(SELECT 1 FROM connection_tombstones t WHERE t.kind='account' AND t.id=outgoing.account)
                     AND NOT EXISTS(SELECT 1 FROM scratch.action_backoff b WHERE b.domain=3 AND b.id=outgoing.attempt AND b.until>unixepoch())
                     AND 'mail:'||account NOT IN (SELECT value FROM json_each(?2))
                     AND attempt NOT IN (SELECT value FROM json_each(?3)) ORDER BY attempt LIMIT 1",
                    parameters, |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?,
                4 => c.query_row(
                    "SELECT json_extract(data,'$.id'),0,account,NULL,0 FROM folder_creations j
                     WHERE json_extract(data,'$.stage') IN ('queued','checking','repair') AND json_extract(data,'$.id')>?1 AND ?4=0
                     AND NOT EXISTS(SELECT 1 FROM connection_tombstones t WHERE t.kind='account' AND t.id=j.account)
                     AND NOT EXISTS(SELECT 1 FROM scratch.action_backoff b WHERE b.domain=4 AND b.id=json_extract(j.data,'$.id') AND b.until>unixepoch())
                     AND 'mail:'||account NOT IN (SELECT value FROM json_each(?2))
                     AND json_extract(data,'$.id') NOT IN (SELECT value FROM json_each(?3)) ORDER BY json_extract(data,'$.id') LIMIT 1",
                    parameters, |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?,
                5 => c.query_row("SELECT json_extract(pending,'$.id'),0,CASE kind WHEN 'account' THEN 'mail:' ELSE 'calendar:' END||id,NULL,0 FROM connection_tombstones r
                    WHERE json_extract(pending,'$.stage') IN ('queued','cleanup') AND json_extract(pending,'$.id')>?1 AND ?4=0
                    AND (CASE kind WHEN 'account' THEN 'mail:' ELSE 'calendar:' END||id) NOT IN (SELECT value FROM json_each(?2))
                    AND json_extract(pending,'$.id') NOT IN (SELECT value FROM json_each(?3))
                    AND NOT EXISTS(SELECT 1 FROM scratch.action_backoff b WHERE b.domain=5 AND b.id=json_extract(r.pending,'$.id') AND b.until>unixepoch())
                    ORDER BY json_extract(pending,'$.id') LIMIT 1",parameters,|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?,
                _ => anyhow::bail!("Unknown action domain"),
            };
            Ok(row.map_or_else(||continuation.map_or(WorkPage::Done, WorkPage::More), |(id, position, source, destination, repair)| {
                let prefix = if domain == 2 { "calendar:" } else { "mail:" };
                let mut accounts = vec![if domain==5 {source.clone()} else {format!("{prefix}{source}")}];
                if let Some(destination) = destination.filter(|v| v != &source) { accounts.push(format!("mail:{destination}")); }
                let cursor = match domain {
                    0 => format!("{}{id}:{position:020}", if repair { '0' } else { '1' }),
                    2 => format!("{}{id}", if repair { '0' } else { '1' }),
                    _ => id.clone(),
                };
                let work = match domain { 0 => Work::Mail { id, position: position as u64 }, 1 => Work::Folder(id), 2 => Work::Calendar(id), 4 => Work::Creation(id), 5 => Work::Removal(id), _ => Work::Outgoing(id) };
                WorkPage::Ready(ReadyWork { work, accounts, cursor })
            }))
        }).await
    }
}
