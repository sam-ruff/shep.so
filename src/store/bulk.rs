//! Durable exact-membership jobs and small receipts. SQLite, rather than iced,
//! owns arbitrarily large groups. Effects describe pending display state only;
//! provider code always reads the original messages table.
use super::*;
use crate::bulk::{Action, Item, Job, Receipt};
use rusqlite::OptionalExtension;

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS bulk_jobs(
        id TEXT PRIMARY KEY, action TEXT NOT NULL, source TEXT NOT NULL, undo_requested INTEGER NOT NULL DEFAULT 0,
        paused INTEGER NOT NULL DEFAULT 0, created INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS bulk_items(
        job TEXT NOT NULL REFERENCES bulk_jobs(id) ON DELETE CASCADE,
        position INTEGER NOT NULL, id TEXT NOT NULL, original TEXT, undo INTEGER NOT NULL DEFAULT 0,
        status TEXT NOT NULL, receipt TEXT, error TEXT, PRIMARY KEY(job,position));
        CREATE INDEX IF NOT EXISTS bulk_item_work ON bulk_items(job,status,position);
        CREATE TABLE IF NOT EXISTS bulk_totals(
        job TEXT NOT NULL REFERENCES bulk_jobs(id) ON DELETE CASCADE,status TEXT NOT NULL,
        undo INTEGER NOT NULL,count INTEGER NOT NULL,PRIMARY KEY(job,status,undo));
        CREATE TRIGGER IF NOT EXISTS bulk_items_insert AFTER INSERT ON bulk_items BEGIN
        INSERT INTO bulk_totals(job,status,undo,count) VALUES(new.job,new.status,new.undo,1)
        ON CONFLICT(job,status,undo) DO UPDATE SET count=count+1; END;
        CREATE TRIGGER IF NOT EXISTS bulk_items_update AFTER UPDATE OF status,undo ON bulk_items
        WHEN old.status!=new.status OR old.undo!=new.undo BEGIN
        UPDATE bulk_totals SET count=count-1 WHERE job=old.job AND status=old.status AND undo=old.undo;
        INSERT INTO bulk_totals(job,status,undo,count) VALUES(new.job,new.status,new.undo,1)
        ON CONFLICT(job,status,undo) DO UPDATE SET count=count+1; END;
        CREATE TRIGGER IF NOT EXISTS bulk_items_delete AFTER DELETE ON bulk_items BEGIN
        UPDATE bulk_totals SET count=count-1 WHERE job=old.job AND status=old.status AND undo=old.undo; END;
        CREATE TABLE IF NOT EXISTS bulk_effects(
        id TEXT PRIMARY KEY, job TEXT NOT NULL REFERENCES bulk_jobs(id) ON DELETE CASCADE,
        position INTEGER NOT NULL, account TEXT, folder TEXT, unread INTEGER, starred INTEGER);
        CREATE INDEX IF NOT EXISTS bulk_effect_items ON bulk_effects(job,position);
        CREATE VIEW IF NOT EXISTS visible_mail AS SELECT m.rowid AS rowid,m.id,
        COALESCE(e.account,m.account) AS account, COALESCE(e.folder,m.folder) AS folder,
        m.sender,m.subject,m.body,m.timestamp,COALESCE(e.unread,m.unread) AS unread,
        COALESCE(e.starred,m.starred) AS starred,m.data,m.raw
        FROM messages m LEFT JOIN bulk_effects e ON e.id=m.id;",
    )?;
    Ok(())
}
fn bump(c: &Connection) -> anyhow::Result<u64> {
    let revision: u64 = get(c, "bulk_revision")?;
    let next = revision
        .checked_add(1)
        .context("Mail operation revision exhausted")?;
    put(c, "bulk_revision", &next)?;
    Ok(next)
}
fn job(c: &Connection, id: &str) -> anyhow::Result<Job> {
    let (action, undo_requested, paused): (String, bool, bool) = c.query_row(
        "SELECT action,undo_requested,paused FROM bulk_jobs WHERE id=?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let mut result = Job {
        id: id.into(),
        action: serde_json::from_str(&action)?,
        undo_requested,
        paused,
        total: 0,
        remaining: 0,
        running: 0,
        completed: 0,
        restored: 0,
        failed: 0,
        uncertain: 0,
        cancelled: 0,
        revision: get(c, "bulk_revision")?,
    };
    let mut query =
        c.prepare("SELECT status,undo,count FROM bulk_totals WHERE job=? AND count>0")?;
    for row in query.query_map([id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, bool>(1)?,
            r.get::<_, i64>(2)? as usize,
        ))
    })? {
        let (status, undo, count) = row?;
        result.total += count;
        match status.as_str() {
            "queued" => result.remaining += count,
            "running" => {
                result.remaining += count;
                result.running += count;
            }
            "done" if undo => result.restored += count,
            "done" => result.completed += count,
            "failed" => result.failed += count,
            "uncertain" => result.uncertain += count,
            "cancelled" => result.cancelled += count,
            _ => anyhow::bail!("Unknown stored mail-operation state"),
        }
    }
    Ok(result)
}
type StoredItem = (
    u64,
    String,
    Option<String>,
    bool,
    String,
    Option<String>,
    Option<String>,
);
fn read_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredItem> {
    Ok((
        row.get::<_, i64>(0)? as u64,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}
fn parse_item(job: &str, row: StoredItem) -> anyhow::Result<Item> {
    let (position, id, original, undo, status, receipt, error) = row;
    Ok(Item {
        job: job.into(),
        position,
        id,
        original: original.map(|s| serde_json::from_str(&s)).transpose()?,
        undo,
        status,
        receipt: receipt.map(|s| serde_json::from_str(&s)).transpose()?,
        error,
    })
}
fn inverse_effect(c: &Connection, item: &Item) -> anyhow::Result<()> {
    let original = item
        .original
        .as_ref()
        .context("The original message is unavailable")?;
    let (id, account, folder, unread, starred) = match item
        .receipt
        .as_ref()
        .context("The operation has no receipt")?
    {
        Receipt::Move(receipt) => {
            // A server MOVE without destination UID can still be undone by the
            // provider's fingerprint recovery, but has no cached row to project.
            let Some(current) = &receipt.current else {
                return Ok(());
            };
            (
                current.id.clone(),
                Some(original.account_id.clone()),
                Some(original.folder.clone()),
                None,
                None,
            )
        }
        Receipt::Flags { before, .. } => (
            original.id.clone(),
            None,
            None,
            before.unread,
            before.starred,
        ),
        Receipt::Unchanged => return Ok(()),
    };
    c.execute("INSERT INTO bulk_effects(id,job,position,account,folder,unread,starred) VALUES(?,?,?,?,?,?,?)",
        params![id,item.job,item.position as i64,account,folder,unread,starred])
        .context("Another change is pending for one of these messages. Finish it before Undo.")?;
    Ok(())
}

const ACCOUNT_ITEMS: &str = "SELECT i.job,i.position FROM bulk_items i JOIN bulk_jobs j ON j.id=i.job WHERE json_extract(i.original,'$.account_id')=?1 OR json_extract(j.action,'$.Move.account')=?1 OR json_extract(i.receipt,'$.Move.account')=?1";
pub(super) fn account_review(
    c: &Connection,
    account: &str,
    digest: &mut sha2::Sha256,
) -> anyhow::Result<(usize, usize)> {
    use sha2::Digest;
    let sql = format!(
        "SELECT json_array(i.job,i.position,i.original,i.receipt,i.undo,i.status,i.error,j.action),i.status FROM bulk_items i JOIN bulk_jobs j ON j.id=i.job WHERE (i.job,i.position) IN ({ACCOUNT_ITEMS}) ORDER BY i.job,i.position"
    );
    let mut statement = c.prepare(&sql)?;
    let mut rows = statement.query([account])?;
    let (mut count, mut pending) = (0, 0);
    while let Some(row) = rows.next()? {
        let value: String = row.get(0)?;
        let status: String = row.get(1)?;
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value);
        count += 1;
        if matches!(status.as_str(), "queued" | "running" | "uncertain") {
            pending += 1;
        }
    }
    Ok((count, pending))
}
pub(super) fn remove_account(c: &Connection, account: &str) -> anyhow::Result<()> {
    c.execute(
        &format!("DELETE FROM bulk_effects WHERE (job,position) IN ({ACCOUNT_ITEMS})"),
        [account],
    )?;
    c.execute(
        &format!("DELETE FROM bulk_items WHERE (job,position) IN ({ACCOUNT_ITEMS})"),
        [account],
    )?;
    c.execute(
        "DELETE FROM bulk_jobs WHERE NOT EXISTS(SELECT 1 FROM bulk_items WHERE job=bulk_jobs.id)",
        [],
    )?;
    bump(c)?;
    Ok(())
}

impl Store {
    /// Atomically copy reviewed membership and original metadata, then publish
    /// pending effects. Missing members remain explicit failed items. No MIME is
    /// materialized and an overlapping active job rejects the whole transaction.
    pub async fn start_bulk(
        &self,
        id: String,
        selection: MailSelectionId,
        action: Action,
    ) -> anyhow::Result<Job> {
        self.run(move |c| {
            let tx = c.transaction()?;
            if let Some((saved,source)) = tx.query_row("SELECT action,source FROM bulk_jobs WHERE id=?",[&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()? {
                anyhow::ensure!(serde_json::from_str::<Action>(&saved)? == action && source == selection.to_string(),"This operation ID belongs to another review");
                return job(&tx,&id);
            }
            match &action {
                Action::Move{folder,..}=>anyhow::ensure!(!folder.trim().is_empty() && !folder.contains(['\r','\n','\0']),"Choose a valid destination folder"),
                Action::Flags(flags)=>anyhow::ensure!(!flags.is_empty(),"Choose a mail action"),
            }
            let frozen: bool = tx.query_row("SELECT frozen FROM temp.mail_selections WHERE id=?",[selection.to_string()],|r|r.get(0))?;
            anyhow::ensure!(frozen,"Review the selected messages before changing them");
            tx.execute("INSERT INTO bulk_jobs(id,action,source,created) VALUES(?,?,?,?)",params![id,serde_json::to_string(&action)?,selection.to_string(),chrono::Utc::now().timestamp_millis()])?;
            tx.execute("INSERT INTO bulk_items(job,position,id,original,status,error)
                SELECT ?,s.position,s.id,CASE WHEN m.id IS NULL THEN NULL ELSE json_set(m.data,
                '$.account_id',m.account,'$.folder',m.folder,'$.unread',json(CASE m.unread WHEN 1 THEN 'true' ELSE 'false' END),
                '$.starred',json(CASE m.starred WHEN 1 THEN 'true' ELSE 'false' END)) END,
                CASE WHEN m.id IS NULL THEN 'failed' ELSE 'queued' END,
                CASE WHEN m.id IS NULL THEN 'This message is unavailable. Refresh its folder or review its pending move.' END
                FROM temp.mail_selection_rows s LEFT JOIN selectable_mail m ON m.id=s.id WHERE s.selection=? AND s.selected=1",
                params![id,selection.to_string()])?;
            let (account,folder,unread,starred) = match &action {
                Action::Move{account,folder} => (account.clone(),Some(folder.clone()),None,None),
                Action::Flags(flags) => (None,None,flags.unread,flags.starred),
            };
            for account in tx.prepare("SELECT DISTINCT json_extract(original,'$.account_id') FROM bulk_items WHERE job=? AND original IS NOT NULL")?
                .query_map([&id], |r| r.get::<_,String>(0))? {
                folder_actions::idle(&tx, &account?)?;
            }
            if let Some(account) = &account { folder_actions::idle(&tx, account)?; }
            tx.execute("INSERT INTO bulk_effects(id,job,position,account,folder,unread,starred)
                SELECT id,job,position,?,?,?,? FROM bulk_items WHERE job=? AND status='queued'",
                params![account,folder,unread,starred, id]).context("Some selected messages already have pending changes. Wait for them, or review their group in History.")?;
            bump(&tx)?;
            let result=job(&tx,&id)?;
            anyhow::ensure!(result.total>0,"Select at least one message");
            tx.execute("DELETE FROM temp.mail_selections WHERE id=?",[selection.to_string()])?;
            tx.commit()?;
            Ok(result)
        }).await
    }
    pub async fn bulk_job(&self, id: String) -> anyhow::Result<Job> {
        self.run(move |c| job(c, &id)).await
    }
    pub async fn bulk_jobs(&self, offset: usize) -> anyhow::Result<Vec<Job>> {
        self.run(move |c| {
            let ids = c
                .prepare("SELECT id FROM bulk_jobs ORDER BY created DESC,id LIMIT 20 OFFSET ?")?
                .query_map([offset as i64], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids.into_iter().map(|id| job(c, &id)).collect()
        })
        .await
    }
    pub async fn bulk_items(&self, id: String, after: Option<u64>) -> anyhow::Result<Vec<Item>> {
        self.run(move |c|c.prepare("SELECT position,id,original,undo,status,receipt,error FROM bulk_items WHERE job=? AND position>? ORDER BY position LIMIT 50")?
            .query_map(params![id,after.map(i64::try_from).transpose()?.unwrap_or(-1)],read_item)?
            .map(|r|parse_item(&id,r?)).collect()).await
    }
    /// Persist running before contacting a provider. A lost process can never
    /// silently replay this non-idempotent step as a fresh queued operation.
    pub async fn claim_bulk_item(&self, id: String) -> anyhow::Result<Option<Item>> {
        self.run(move |c| {
            let tx=c.transaction()?;
            if job(&tx,&id)?.paused { return Ok(None); }
            let next=tx.query_row("SELECT position,id,original,undo,status,receipt,error FROM bulk_items WHERE job=? AND status='queued' ORDER BY position LIMIT 1",[&id],read_item).optional()?;
            let Some(next)=next else{return Ok(None)};
            let mut item=parse_item(&id,next)?;
            tx.execute("UPDATE bulk_items SET status='running' WHERE job=? AND position=? AND status='queued'",params![id,item.position as i64])?;
            item.status="running".into();
            tx.commit()?; Ok(Some(item))
        }).await
    }
    pub async fn finish_bulk_item(
        &self,
        item: Item,
        result: Result<Receipt, (String, bool)>,
    ) -> anyhow::Result<Job> {
        self.run(move |c| {
            let tx=c.transaction()?;
            let current: (String,bool)=tx.query_row("SELECT status,undo FROM bulk_items WHERE job=? AND position=?",params![item.job,item.position as i64],|r|Ok((r.get(0)?,r.get(1)?)))?;
            anyhow::ensure!(current == ("running".into(),item.undo),"This mail-operation result is no longer current");
            let claimed: Vec<String> = tx.prepare("SELECT id FROM bulk_effects WHERE job=? AND position=?")?
                .query_map(params![item.job,item.position as i64], |r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
            tx.execute("DELETE FROM bulk_effects WHERE job=? AND position=?",params![item.job,item.position as i64])?;
            match result {
                Ok(receipt) => {
                    // Retain the forward receipt for auditing even after Undo.
                    let saved = if item.undo { item.receipt.clone().unwrap() } else { receipt };
                    tx.execute("UPDATE bulk_items SET status='done',receipt=?,error=NULL WHERE job=? AND position=?",params![serde_json::to_string(&saved)?,item.job,item.position as i64])?;
                    if !item.undo && job(&tx,&item.job)?.undo_requested {
                        let mut inverse=item.clone();inverse.receipt=Some(saved);
                        // Failure to stage Undo must never roll back a durable
                        // acknowledgment of the successful forward operation.
                        tx.execute_batch("SAVEPOINT inverse_effect")?;
                        let staged = inverse_effect(&tx,&inverse);
                        let error = if let Err(error)=staged { tx.execute_batch("ROLLBACK TO inverse_effect")?; Some(error.to_string()) } else { None };
                        tx.execute_batch("RELEASE inverse_effect")?;
                        tx.execute("UPDATE bulk_items SET undo=1,status=?,error=? WHERE job=? AND position=?",
                            params![if error.is_some() {"failed"} else {"queued"}, error,item.job,item.position as i64])?;
                    }
                }
                Err((error,uncertain)) => {
                    tx.execute("UPDATE bulk_items SET status=?,error=? WHERE job=? AND position=?",params![if uncertain {"uncertain"} else {"failed"},error,item.job,item.position as i64])?;
                    if uncertain {
                        for id in claimed { tx.execute("INSERT INTO bulk_effects(id,job,position) VALUES(?,?,?)",params![id,item.job,item.position as i64])?; }
                    }
                }
            }
            bump(&tx)?;let result=job(&tx,&item.job)?;tx.commit()?;Ok(result)
        }).await
    }
    /// Undo cancels work which never reached a provider and reverses only durable
    /// successful receipts. A running step is reversed after its acknowledgment.
    pub async fn request_bulk_undo(&self, id: String) -> anyhow::Result<Job> {
        self.run(move |c| {
            let tx=c.transaction()?;
            for account in tx.prepare("SELECT json_extract(original,'$.account_id') FROM bulk_items WHERE job=?1 AND original IS NOT NULL UNION SELECT json_extract(receipt,'$.Move.account') FROM bulk_items WHERE job=?1 AND json_extract(receipt,'$.Move.account') IS NOT NULL")?
                .query_map([&id], |r| r.get::<_,String>(0))? {
                folder_actions::idle(&tx, &account?)?;
            }
            tx.execute("UPDATE bulk_jobs SET undo_requested=1,paused=0 WHERE id=?",[&id])?;
            // The in-flight provider retains ownership, but Undo immediately
            // restores the cached source's display until its receipt arrives.
            tx.execute("UPDATE bulk_effects SET account=NULL,folder=NULL,unread=NULL,starred=NULL WHERE job=? AND position IN (SELECT position FROM bulk_items WHERE job=? AND status='running' AND undo=0)",params![id,id])?;

            tx.execute("DELETE FROM bulk_effects WHERE job=? AND position IN (SELECT position FROM bulk_items WHERE job=? AND status='queued' AND undo=0)",params![id,id])?;
            tx.execute("UPDATE bulk_items SET status='cancelled' WHERE job=? AND status='queued' AND undo=0",[&id])?;
            // Cursor in bounded pages; the transaction keeps this exact group
            // consistent without ever holding every receipt in a Rust Vec.
            let mut after=-1_i64;
            loop {
                let rows=tx.prepare("SELECT position,id,original,undo,status,receipt,error FROM bulk_items WHERE job=? AND ((status='done' AND undo=0) OR (status='failed' AND undo=1)) AND position>? ORDER BY position LIMIT 50")?
                    .query_map(params![id,after],read_item)?.collect::<rusqlite::Result<Vec<_>>>()?;
                if rows.is_empty(){break;}
                for row in rows {
                    let item=parse_item(&id,row)?;after=item.position as i64;
                    inverse_effect(&tx,&item)?;
                    tx.execute("UPDATE bulk_items SET undo=1,status='queued',error=NULL WHERE job=? AND position=?",params![id,after])?;
                }
            }
            bump(&tx)?;let result=job(&tx,&id)?;tx.commit()?;Ok(result)
        }).await
    }
    /// Claim the actual identity discovered during Undo before any provider
    /// write. Never steal another group's claim or revive a completed phase.
    pub async fn claim_bulk_identity(&self, item: Item, id: String) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx=c.transaction()?;
            let current: (String,bool)=tx.query_row("SELECT status,undo FROM bulk_items WHERE job=? AND position=?",
                params![item.job,item.position as i64],|r|Ok((r.get(0)?,r.get(1)?)))?;
            anyhow::ensure!(current == ("running".into(),item.undo),"This group operation is no longer current");
            anyhow::ensure!(item.undo || id==item.id,"The forward message identity changed");
            let inserted=tx.execute("INSERT INTO bulk_effects(id,job,position) VALUES(?,?,?) ON CONFLICT(id) DO NOTHING",
                params![id,item.job,item.position as i64])?;
            let owner: (String,i64)=tx.query_row("SELECT job,position FROM bulk_effects WHERE id=?",[&id],|r|Ok((r.get(0)?,r.get(1)?)))?;
            anyhow::ensure!(owner == (item.job,item.position as i64),"Another group owns this message. Finish or review that change first.");
            if inserted>0 { bump(&tx)?; }
            tx.commit()?;
            Ok(())
        }).await
    }
    pub async fn bulk_owner(&self, id: String) -> anyhow::Result<Option<String>> {
        self.run(move |c| {
            Ok(
                c.query_row("SELECT job FROM bulk_effects WHERE id=?", [id], |r| {
                    r.get(0)
                })
                .optional()?,
            )
        })
        .await
    }
}

pub(super) fn has_effects(c: &Connection) -> anyhow::Result<bool> {
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM bulk_effects WHERE account IS NOT NULL OR folder IS NOT NULL OR unread IS NOT NULL OR starred IS NOT NULL)",[],|r|r.get(0))?)
}
/// One owned, nonduplicated advisory lock per database job. Memory fixtures need
/// no disk file; production leases are checked before execution or recovery.
pub struct BulkLease {
    store: Store,
    id: String,
    _file: Option<std::fs::File>,
}
impl Store {
    pub async fn bulk_lease(&self, id: String) -> anyhow::Result<BulkLease> {
        let path = self
            .run(|c| {
                Ok(c.path()
                    .filter(|p| !p.is_empty())
                    .map(std::path::PathBuf::from))
            })
            .await?;
        let key = id.clone();
        let file = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let Some(path) = path else { return Ok(None) };
            use sha2::{Digest, Sha256};
            let directory = path
                .parent()
                .context("Mail cache has no parent directory")?
                .join("bulk-locks");
            std::fs::create_dir_all(&directory)?;
            let path = directory.join(format!("{:x}.lock", Sha256::digest(key.as_bytes())));
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?;
            fs2::FileExt::try_lock_exclusive(&file)
                .context("This mail operation is still running in another window")?;
            Ok(Some(file))
        })
        .await??;
        Ok(BulkLease {
            store: self.clone(),
            id,
            _file: file,
        })
    }
    /// Called only after obtaining the job lease: a recorded running step now
    /// has no live executor. Retain it for review; resume only never-started work.
    pub async fn resume_bulk(&self, lease: &BulkLease) -> anyhow::Result<Job> {
        anyhow::ensure!(
            Arc::ptr_eq(&self.0, &lease.store.0),
            "Use the lease for this mail cache"
        );
        let id = lease.id.clone();
        self.run(move |c| {
            let tx=c.transaction()?;
            tx.execute("UPDATE bulk_effects SET account=NULL,folder=NULL,unread=NULL,starred=NULL WHERE job=? AND position IN (SELECT position FROM bulk_items WHERE job=? AND status='running')",params![id,id])?;
            tx.execute("UPDATE bulk_items SET status='uncertain',error='Shep closed before this change was acknowledged. Refresh and check the source and destination folders before resolving it.' WHERE job=? AND status='running'",[&id])?;
            tx.execute("UPDATE bulk_jobs SET paused=0 WHERE id=?",[&id])?;
            bump(&tx)?;let result=job(&tx,&id)?;tx.commit()?;Ok(result)
        }).await
    }
    /// Resolving an uncertain result is an explicit acceptance of the current
    /// mail state. It never sends, copies, deletes, or replays a provider action.
    pub async fn accept_bulk_uncertainty(&self, lease: &BulkLease) -> anyhow::Result<Job> {
        anyhow::ensure!(
            Arc::ptr_eq(&self.0, &lease.store.0),
            "Use the lease for this mail cache"
        );
        let id = lease.id.clone();
        self.run(move |c| {
            let tx=c.transaction()?;
            tx.execute("DELETE FROM bulk_effects WHERE job=? AND position IN (SELECT position FROM bulk_items WHERE job=? AND status='uncertain')",params![id,id])?;
            tx.execute("UPDATE bulk_items SET status='cancelled',error=? WHERE job=? AND status='uncertain'",params![crate::bulk::ACCEPTED_STATE_NOTE,id])?;
            bump(&tx)?;let result=job(&tx,&id)?;tx.commit()?;Ok(result)
        }).await
    }
}

impl Store {
    /// Make an explicitly continued group eligible for the worker. Only its
    /// lease holder may recover an interrupted running item or execute work.
    pub async fn continue_bulk(&self, id: String) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let current = job(&tx, &id)?;
            if current.paused {
                tx.execute("UPDATE bulk_jobs SET paused=0 WHERE id=?", [&id])?;
                bump(&tx)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn next_pending_bulk(&self, after: String) -> anyhow::Result<Option<String>> {
        self.run(move |c|Ok(c.query_row("SELECT id FROM bulk_jobs WHERE id>? AND paused=0 AND EXISTS(SELECT 1 FROM bulk_items WHERE job=bulk_jobs.id AND status IN ('queued','running')) ORDER BY id LIMIT 1",[after],|r|r.get(0)).optional()?)).await
    }
}
