use super::*;
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemovalStage {
    Queued,
    Cleanup,
    Failed,
    Succeeded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct RemovalJob {
    pub id: String,
    pub target: ConnectionRef,
    pub fingerprint: String,
    pub review_epoch: u64,
    pub cancel_transfers: bool,
    pub label: String,
    pub stage: RemovalStage,
    pub local_done: bool,
    pub device_credentials: bool,
    pub calendar_key: bool,
    pub calendar_revision: Option<u64>,
    pub revision: u64,
    pub error: Option<String>,
}

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    let columns = c
        .prepare("PRAGMA table_info(connection_tombstones)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|column| column == "pending") {
        c.execute_batch("ALTER TABLE connection_tombstones ADD COLUMN pending TEXT;")?;
    }
    c.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS connection_removal_request ON connection_tombstones(json_extract(pending,'$.id'));
        CREATE INDEX IF NOT EXISTS connection_removal_pending ON connection_tombstones(json_extract(pending,'$.stage'),json_extract(pending,'$.id'));")?;
    Ok(())
}

pub(super) fn epoch(c: &Connection, target: &ConnectionRef) -> anyhow::Result<u64> {
    let value: i64 = c
        .query_row(
            "SELECT revision FROM connection_removal_epochs WHERE kind=? AND id=?",
            params![target.kind.key(), target.id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok(u64::try_from(value)?)
}

pub(super) fn allow_reconnect(
    c: &Connection,
    kind: ConnectionKind,
    id: &str,
) -> anyhow::Result<()> {
    let pending: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM connection_tombstones WHERE kind=? AND id=? AND pending IS NOT NULL AND json_extract(pending,'$.stage')!='succeeded')",params![kind.key(),id],|r|r.get(0))?;
    anyhow::ensure!(
        !pending,
        "Finish this connection's local removal before reconnecting it. Open removal progress in Preferences."
    );
    Ok(())
}

fn load(c: &Connection, id: &str) -> anyhow::Result<RemovalJob> {
    let data: String = c.query_row(
        "SELECT pending FROM connection_tombstones WHERE json_extract(pending,'$.id')=?",
        [id],
        |r| r.get(0),
    )?;
    Ok(serde_json::from_str(&data)?)
}

pub(crate) fn pending(c: &Connection) -> anyhow::Result<Vec<RemovalJob>> {
    c.prepare("SELECT pending FROM connection_tombstones WHERE json_extract(pending,'$.stage') IN ('queued','cleanup','failed') ORDER BY json_extract(pending,'$.id') LIMIT 32")?.query_map([],|r|r.get::<_,String>(0))?.map(|row|Ok(serde_json::from_str(&row?)?)).collect()
}

pub(crate) fn fence_import(c: &Connection) -> anyhow::Result<()> {
    let revision = changed(c)?;
    c.execute("UPDATE connection_tombstones SET pending=json_set(pending,'$.device_credentials',json('false'),'$.stage',CASE WHEN json_extract(pending,'$.local_done')=1 THEN 'cleanup' ELSE 'queued' END,'$.error',NULL,'$.revision',?) WHERE pending IS NOT NULL AND json_extract(pending,'$.stage')!='succeeded'",[revision as i64])?;
    Ok(())
}

pub(super) fn current(c: &Connection, expected: &RemovalJob) -> anyhow::Result<RemovalJob> {
    let job = load(c, &expected.id)?;
    anyhow::ensure!(
        &job == expected,
        "This removal changed. Refresh its progress."
    );
    Ok(job)
}

pub(super) fn save(c: &Connection, job: &mut RemovalJob) -> anyhow::Result<()> {
    job.revision = changed(c)?;
    let written=c.execute("UPDATE connection_tombstones SET pending=? WHERE kind=? AND id=? AND json_extract(pending,'$.id')=?",params![serde_json::to_string(job)?,job.target.kind.key(),job.target.id,job.id])?;
    anyhow::ensure!(written == 1, "This removal no longer owns the connection.");
    Ok(())
}

impl Store {
    pub(crate) async fn finish_removal_credentials(
        &self,
        target: ConnectionRef,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let data: Option<String> = tx
                .query_row(
                    "SELECT pending FROM connection_tombstones WHERE kind=? AND id=?",
                    params![target.kind.key(), target.id],
                    |r| r.get(0),
                )
                .optional()?
                .flatten();
            if let Some(data) = data {
                let mut job: RemovalJob = serde_json::from_str(&data)?;
                let pending: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM credential_cleanup WHERE kind=? AND id=?)",
                    params![target.kind.key(), target.id],
                    |r| r.get(0),
                )?;
                if job.local_done && !pending && job.stage != RemovalStage::Succeeded {
                    job.stage = RemovalStage::Succeeded;
                    job.error = None;
                    save(&tx, &mut job)?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub(crate) async fn admit_connection_removal(
        &self,
        id: String,
        expected: RemovalPreview,
        cancel_transfers: bool,
    ) -> anyhow::Result<RemovalJob> {
        uuid::Uuid::parse_str(&id)?;
        self.run(move |c| {
            let tx=c.transaction()?;
            if let Ok(job)=load(&tx,&id) {
                anyhow::ensure!(job.target==expected.target && job.fingerprint==expected.fingerprint && job.review_epoch==expected.removal_epoch && job.cancel_transfers==cancel_transfers,"This saved removal has different input.");
                return Ok(job);
            }
            let target=&expected.target;
            let pending:i64=tx.query_row("SELECT count(*) FROM connection_tombstones WHERE json_extract(pending,'$.stage') IN ('queued','cleanup','failed')",[],|r|r.get(0))?;
            anyhow::ensure!(pending<32,"Finish pending local removals before removing another connection.");
            allow(&tx,target.kind,&target.id)?;
            anyhow::ensure!(epoch(&tx,target)?==expected.removal_epoch,"This connection was removed or reconnected after review. Review it again.");
            let reviewed=preview(&tx,target.clone())?;
            anyhow::ensure!(reviewed.fingerprint==expected.fingerprint,"Local data changed while this dialog was open. Review the updated counts before removing the connection.");
            anyhow::ensure!(reviewed.transfers==0 || cancel_transfers,"Confirm cancellation of the unfinished mail changes before removing this account.");
            let mut google_data=None;
            let mut calendar_key=false;
            match target.kind {
                ConnectionKind::Account => {
                    let mut accounts:Vec<Account>=get(&tx,"accounts")?;
                    accounts.retain(|account|account.id!=target.id);
                    put(&tx,"accounts",&accounts)?;
                }
                ConnectionKind::Calendar => {
                    let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM calendar_actions WHERE source=? AND status NOT IN ('succeeded','cancelled'))",[&target.id],|r|r.get(0))?;
                    anyhow::ensure!(!pending,"Finish or review this calendar's pending changes before removing it.");
                    let mut sources:Vec<CalendarSource>=get(&tx,"calendars")?;
                    if let Some(source)=sources.iter().find(|source|source.id==target.id) {
                        calendar_key=source.kind==CalendarKind::CalDav;
                        if !calendar_key { google_data=Some(serde_json::to_string(source)?); }
                    }
                    sources.retain(|source|source.id!=target.id);
                    put(&tx,"calendars",&sources)?;
                    calendar_changed(&tx)?;
                }
            }
            let revision=changed(&tx)?;
            let calendar_revision=if target.kind==ConnectionKind::Calendar {Some(get(&tx,"calendar_revision")?)} else {None};
            let job=RemovalJob{id,target:target.clone(),fingerprint:expected.fingerprint,review_epoch:expected.removal_epoch,cancel_transfers,label:expected.name.chars().take(120).collect(),stage:RemovalStage::Queued,local_done:false,device_credentials:true,calendar_key,calendar_revision,revision,error:None};
            tx.execute("INSERT INTO connection_tombstones(kind,id,revision,google_data,pending) VALUES(?,?,?,?,?)",params![target.kind.key(),target.id,revision as i64,google_data,serde_json::to_string(&job)?])?;
            tx.execute("INSERT INTO connection_removal_epochs(kind,id,revision) VALUES(?,?,?) ON CONFLICT(kind,id) DO UPDATE SET revision=excluded.revision",params![target.kind.key(),target.id,revision as i64])?;
            drafts::changed(&tx)?;
            outgoing::changed(&tx)?;
            tx.commit()?;
            Ok(job)
        }).await
    }

    pub(crate) async fn removal_job(&self, id: String) -> anyhow::Result<RemovalJob> {
        self.run(move |c| load(c, &id)).await
    }

    pub(crate) async fn removal_progress(
        &self,
        expected: RemovalJob,
        stage: RemovalStage,
        error: Option<String>,
    ) -> anyhow::Result<RemovalJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = current(&tx, &expected)?;
            anyhow::ensure!(
                stage != RemovalStage::Succeeded || job.local_done,
                "Local removal is unfinished."
            );
            job.stage = stage;
            job.error = error;
            save(&tx, &mut job)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }
}
