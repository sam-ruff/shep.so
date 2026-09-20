use super::*;
use rusqlite::OptionalExtension;

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
}

#[derive(Clone, Debug)]
pub(crate) struct ReadyWork {
    pub work: Work,
    pub accounts: Vec<String>,
    pub cursor: String,
}

impl Work {
    pub fn id(&self) -> &str {
        match self {
            Self::Mail { id, .. } | Self::Folder(id) | Self::Calendar(id) | Self::Outgoing(id) => {
                id
            }
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
        anyhow::ensure!(domain < 4, "Unknown action domain");
        let domain = domain as i64;
        self.run(move |c| {
            if changed {
                c.execute("DELETE FROM scratch.action_backoff WHERE domain=? AND id=?",params![domain,id])?;
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
    pub(crate) async fn next_action_work(
        &self,
        domain: usize,
        after: String,
        occupied: Vec<String>,
        active: Vec<String>,
        cache_only: bool,
    ) -> anyhow::Result<Option<ReadyWork>> {
        self.run(move |c| {
            let occupied = serde_json::to_string(&occupied)?;
            let active = serde_json::to_string(&active)?;
            let parameters = params![after, occupied, active, cache_only];
            let row: Option<(String, i64, String, Option<String>, bool)> = match domain {
                0 => c.query_row(
                    "SELECT j.id,i.position,
                        COALESCE((SELECT m.account FROM bulk_admissions a JOIN mail_lineage l ON l.lineage=a.lineage JOIN messages m ON m.id=l.id WHERE a.job=i.job AND a.position=i.position),json_extract(i.original,'$.account_id'),''),
                        json_extract(j.action,'$.Move.account'),i.status='repair'
                     FROM bulk_jobs j JOIN bulk_items i ON i.job=j.id
                     WHERE j.paused=0 AND (CASE WHEN i.status='repair' THEN '0' ELSE '1' END||j.id)>?1 AND j.id NOT IN (SELECT value FROM json_each(?3))
                       AND NOT EXISTS(SELECT 1 FROM scratch.action_backoff b WHERE b.domain=0 AND b.id=j.id AND b.until>unixepoch())
                       AND i.status IN ('queued','running','repair') AND (?4=0 OR i.status='repair')
                       AND (i.status IN ('running','repair') OR NOT EXISTS(SELECT 1 FROM bulk_admissions a JOIN bulk_admissions prior ON prior.lineage=a.lineage AND prior.sequence<a.sequence
                           JOIN bulk_items p ON p.job=prior.job AND p.position=prior.position
                           WHERE a.job=i.job AND a.position=i.position AND p.status IN ('queued','running','repair','uncertain')))
                       AND 'mail:'||COALESCE((SELECT m.account FROM bulk_admissions a JOIN mail_lineage l ON l.lineage=a.lineage JOIN messages m ON m.id=l.id WHERE a.job=i.job AND a.position=i.position),json_extract(i.original,'$.account_id'),'') NOT IN (SELECT value FROM json_each(?2))
                       AND (json_extract(j.action,'$.Move.account') IS NULL OR 'mail:'||json_extract(j.action,'$.Move.account') NOT IN (SELECT value FROM json_each(?2)))
                     ORDER BY CASE WHEN i.status='repair' THEN '0' ELSE '1' END||j.id,CASE i.status WHEN 'repair' THEN 0 WHEN 'running' THEN 1 ELSE 2 END,i.position LIMIT 1",
                    parameters, |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?,
                1 => c.query_row(
                    "SELECT id,0,account,NULL,0 FROM folder_jobs j WHERE closed=0 AND id>?1 AND ?4=0
                     AND NOT EXISTS(SELECT 1 FROM scratch.action_backoff b WHERE b.domain=1 AND b.id=j.id AND b.until>unixepoch())
                     AND 'mail:'||account NOT IN (SELECT value FROM json_each(?2))
                     AND id NOT IN (SELECT value FROM json_each(?3))
                     AND NOT EXISTS(SELECT 1 FROM folder_steps s WHERE s.job=j.id AND json_extract(s.status,'$') IN ('Rejected','Uncertain'))
                     ORDER BY id LIMIT 1", parameters, |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?,
                2 => c.query_row(
                    "SELECT a.id,0,a.source,NULL,a.status='repair' FROM calendar_actions a LEFT JOIN calendar_actions p ON p.id=a.previous
                     WHERE (CASE WHEN a.status='repair' THEN '0' ELSE '1' END||a.id)>?1 AND (?4=0 OR a.status='repair') AND 'calendar:'||a.source NOT IN (SELECT value FROM json_each(?2))
                     AND NOT EXISTS(SELECT 1 FROM scratch.action_backoff b WHERE b.domain=2 AND b.id=a.id AND b.until>unixepoch())
                     AND a.id NOT IN (SELECT value FROM json_each(?3))
                     AND (a.status='repair' AND json_extract(a.data,'$.checked')=0 AND json_extract(a.data,'$.cache_applied')=0
                         OR (a.status='queued' OR a.status='waiting' AND json_extract(a.data,'$.retry_at')<=unixepoch())
                            AND (p.id IS NULL OR p.status IN ('succeeded','rejected','cancelled')))
                     ORDER BY CASE WHEN a.status='repair' THEN '0' ELSE '1' END||a.id LIMIT 1", parameters, |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?,
                3 => c.query_row(
                    "SELECT attempt,0,account,NULL,0 FROM outgoing WHERE stage='Queued' AND attempt>?1 AND ?4=0
                     AND NOT EXISTS(SELECT 1 FROM scratch.action_backoff b WHERE b.domain=3 AND b.id=outgoing.attempt AND b.until>unixepoch())
                     AND 'mail:'||account NOT IN (SELECT value FROM json_each(?2))
                     AND attempt NOT IN (SELECT value FROM json_each(?3)) ORDER BY attempt LIMIT 1",
                    parameters, |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?,
                _ => anyhow::bail!("Unknown action domain"),
            };
            Ok(row.map(|(id, position, source, destination, repair)| {
                let prefix = if domain == 2 { "calendar:" } else { "mail:" };
                let mut accounts = vec![format!("{prefix}{source}")];
                if let Some(destination) = destination.filter(|v| v != &source) { accounts.push(format!("mail:{destination}")); }
                let cursor = if matches!(domain,0|2) { format!("{}{id}",if repair { '0' } else { '1' }) } else { id.clone() };
                let work = match domain { 0 => Work::Mail { id, position: position as u64 }, 1 => Work::Folder(id), 2 => Work::Calendar(id), _ => Work::Outgoing(id) };
                ReadyWork { work, accounts, cursor }
            }))
        }).await
    }
}
