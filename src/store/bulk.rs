//! Durable exact-membership jobs and small receipts. SQLite, rather than iced,
//! owns arbitrarily large groups. Effects describe pending display state only;
//! provider code always reads the original messages table.
use super::*;
use crate::bulk::{Action, Item, Job, Receipt};
use rusqlite::OptionalExtension;
mod intents;

pub(super) fn reproject_lineage(c: &Connection, lineage: &str) -> anyhow::Result<()> {
    let item: Option<(String, i64)> = c.query_row("SELECT job,position FROM bulk_admissions WHERE lineage=? ORDER BY sequence DESC LIMIT 1", [lineage], |r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((job, position)) = item {
        intents::refresh_scope(c, &job, Some(position))?;
        bump(c)?;
    }
    Ok(())
}

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
        CREATE TABLE IF NOT EXISTS bulk_flag_receipts(
        job TEXT NOT NULL, position INTEGER NOT NULL, undo INTEGER NOT NULL,
        receipt TEXT NOT NULL, PRIMARY KEY(job,position),
        FOREIGN KEY(job,position) REFERENCES bulk_items(job,position) ON DELETE CASCADE);
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
    intents::schema(c)
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
            "queued" | "repair" => result.remaining += count,
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
    let bound: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM bulk_admissions WHERE job=? AND position=?)",
        params![item.job, item.position as i64],
        |r| r.get(0),
    )?;
    if bound {
        return Ok(());
    }
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
        Receipt::Unchanged | Receipt::Superseded => return Ok(()),
    };
    c.execute("INSERT INTO bulk_effects(id,job,position,account,folder,unread,starred) VALUES(?,?,?,?,?,?,?)",
        params![id,item.job,item.position as i64,account,folder,unread,starred])
        .context("Another change is pending for one of these messages. Finish it before Undo.")?;
    Ok(())
}

const ACCOUNT_ITEMS: &str = "SELECT i.job,i.position FROM bulk_items i JOIN bulk_jobs j ON j.id=i.job WHERE json_extract(i.original,'$.account_id')=?1 OR json_extract(j.action,'$.Move.account')=?1 OR json_extract(i.receipt,'$.Move.account')=?1 OR EXISTS(SELECT 1 FROM bulk_admissions a WHERE a.job=i.job AND a.position=i.position AND json_extract(a.original,'$.account_id')=?1)";
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
        if matches!(
            status.as_str(),
            "queued" | "running" | "repair" | "uncertain"
        ) {
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

fn validate_action(action: &Action) -> anyhow::Result<()> {
    match action {
        Action::Move { folder, .. } => anyhow::ensure!(
            !folder.trim().is_empty() && !folder.contains(['\r', '\n', '\0']),
            "Choose a valid destination folder"
        ),
        Action::Flags(flags) => anyhow::ensure!(!flags.is_empty(), "Choose a mail action"),
    }
    Ok(())
}

fn existing_admission(
    c: &Connection,
    id: &str,
    source: &str,
    action: &Action,
) -> anyhow::Result<Option<Job>> {
    let saved = c
        .query_row(
            "SELECT action,source FROM bulk_jobs WHERE id=?",
            [id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((saved, origin)) = saved else {
        return Ok(None);
    };
    anyhow::ensure!(
        serde_json::from_str::<Action>(&saved)? == *action && origin == source,
        "This operation ID belongs to another request"
    );
    Ok(Some(job(c, id)?))
}

fn publish_admission(c: &Connection, id: &str, action: &Action) -> anyhow::Result<Job> {
    intents::admit(c, id, action)?;
    let account = match action {
        Action::Move { account, .. } => account.clone(),
        Action::Flags(_) => None,
    };
    for account in c.prepare("SELECT DISTINCT json_extract(original,'$.account_id') FROM bulk_items WHERE job=? AND original IS NOT NULL")?
        .query_map([id], |row| row.get::<_, String>(0))? {
        let account = account?;
        connections::allow(c, ConnectionKind::Account, &account)?;
        folder_actions::idle(c, &account)?;
    }
    if let Some(account) = &account {
        connections::allow(c, ConnectionKind::Account, account)?;
        folder_actions::idle(c, account)?;
    }
    anyhow::ensure!(!c.query_row(
        "SELECT EXISTS(SELECT 1 FROM bulk_items i JOIN bulk_effects e ON e.id=i.id
          WHERE i.job=? AND NOT EXISTS(SELECT 1 FROM bulk_admissions a WHERE a.job=e.job AND a.position=e.position))",
        [id], |r| r.get::<_, bool>(0))?,
        "A message has an older pending operation. Review its group in History first.");
    intents::refresh(c, id)?;
    bump(c)?;
    let result = job(c, id)?;
    anyhow::ensure!(result.total > 0, "Select at least one message");
    Ok(result)
}

impl Store {
    pub async fn accepted_bulk_flags(
        &self,
        item: Item,
        mut flags: crate::mail_actions::Flags,
    ) -> anyhow::Result<crate::mail_actions::Flags> {
        self.run(move |c| {
            if !intents::owns(c, &item, "unread")? {
                flags.unread = None;
            }
            if !intents::owns(c, &item, "starred")? {
                flags.starred = None;
            }
            Ok(flags)
        })
        .await
    }
    /// Individual and selected mail actions share one journal and provider owner.
    pub async fn start_individual_mail_action(
        &self,
        id: String,
        original: Mail,
        action: Action,
    ) -> anyhow::Result<Job> {
        self.admit_mail_action(id, original, action, None).await
    }

    pub async fn start_observed_mail_action(
        &self,
        id: String,
        original: Mail,
        action: Action,
        expected_lineage: String,
    ) -> anyhow::Result<Job> {
        anyhow::ensure!(
            !expected_lineage.is_empty(),
            "Refresh this message before trying again."
        );
        self.admit_mail_action(id, original, action, Some(expected_lineage))
            .await
    }

    async fn admit_mail_action(
        &self,
        id: String,
        original: Mail,
        action: Action,
        expected_lineage: Option<String>,
    ) -> anyhow::Result<Job> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let source = if let Some(lineage) = &expected_lineage {
                serde_json::to_string(&(&original.id,&original.account_id,&original.folder,&original.remote_id,lineage))?
            } else { serde_json::to_string(&(
                &original.id,
                &original.account_id,
                &original.folder,
                &original.remote_id,
            ))? };
            if let Some(existing) = existing_admission(&tx, &id, &source, &action)? {
                return Ok(existing);
            }
            validate_action(&action)?;
            connections::allow(&tx, ConnectionKind::Account, &original.account_id)?;
            if let Action::Move {
                account: Some(account),
                ..
            } = &action
            {
                connections::allow(&tx, ConnectionKind::Account, account)?;
            }
            let mut current: Option<String> = tx
                .query_row(
                    "SELECT json_set(m.data,'$.account_id',m.account,'$.folder',m.folder,
                    '$.unread',json(CASE m.unread WHEN 1 THEN 'true' ELSE 'false' END),
                    '$.starred',json(CASE m.starred WHEN 1 THEN 'true' ELSE 'false' END))
                 FROM selectable_mail m LEFT JOIN bulk_effects e ON e.id=m.id JOIN mail_lineage l ON l.id=m.id
                 WHERE m.id=?1 AND (?5 IS NOT NULL OR (m.account=?2 OR COALESCE(e.account,m.account)=?2)
                    AND (m.folder=?3 OR COALESCE(e.folder,m.folder)=?3))
                    AND json_extract(m.data,'$.remote_id')=?4
                    AND (?5 IS NULL OR l.lineage=COALESCE((SELECT lineage FROM mail_lineage_alias WHERE alias=?5),?5))",
                    params![
                        original.id,
                        original.account_id,
                        original.folder,
                        original.remote_id,
                        expected_lineage
                    ],
                    |row| row.get(0),
                )
                .optional()?;
            if current.is_none() && let Some(lineage) = &expected_lineage {
                current = tx.query_row("SELECT json_set(m.data,'$.account_id',m.account,'$.folder',m.folder,
                        '$.unread',json(CASE m.unread WHEN 1 THEN 'true' ELSE 'false' END),
                        '$.starred',json(CASE m.starred WHEN 1 THEN 'true' ELSE 'false' END))
                    FROM mail_identity_history h JOIN mail_lineage l ON l.lineage=COALESCE((SELECT lineage FROM mail_lineage_alias WHERE alias=h.lineage),h.lineage)
                    JOIN selectable_mail m ON m.id=l.id WHERE h.id=?1 AND h.remote=?2
                    AND l.lineage=COALESCE((SELECT lineage FROM mail_lineage_alias WHERE alias=?3),?3) LIMIT 1",
                    params![original.id,original.remote_id,lineage], |r|r.get(0)).optional()?;
            }
            let current = current
                .context("This message changed or is unavailable. Refresh before trying again.")?;
            let actual: Mail = serde_json::from_str(&current)?;
            tx.execute(
                "INSERT INTO bulk_jobs(id,action,source,created) VALUES(?,?,?,?)",
                params![
                    id,
                    serde_json::to_string(&action)?,
                    source,
                    chrono::Utc::now().timestamp_millis()
                ],
            )?;
            tx.execute(
                "INSERT INTO bulk_items(job,position,id,original,status) VALUES(?,0,?,?,'queued')",
                params![id, actual.id, current],
            )?;
            let result = publish_admission(&tx, &id, &action)?;
            tx.commit()?;
            Ok(result)
        })
        .await
    }

    /// Atomically copy reviewed membership and original metadata, then publish
    /// pending effects. Missing members remain explicit failed items. Overlapping
    /// work reserves field ownership and waits for its predecessor receipts.
    pub async fn start_bulk(
        &self,
        id: String,
        selection: MailSelectionId,
        action: Action,
    ) -> anyhow::Result<Job> {
        self.run(move |c| {
            let tx = c.transaction()?;
            if let Some(existing) = existing_admission(&tx, &id, &selection.to_string(), &action)? {
                return Ok(existing);
            }
            validate_action(&action)?;
            let frozen: bool = tx.query_row("SELECT frozen FROM scratch.mail_selections WHERE id=?",[selection.to_string()],|r|r.get(0))?;
            anyhow::ensure!(frozen,"Review the selected messages before changing them");
            tx.execute("INSERT INTO bulk_jobs(id,action,source,created) VALUES(?,?,?,?)",params![id,serde_json::to_string(&action)?,selection.to_string(),chrono::Utc::now().timestamp_millis()])?;
            tx.execute("INSERT INTO bulk_items(job,position,id,original,status,error)
                SELECT ?,s.position,COALESCE(m.id,s.id),CASE WHEN m.id IS NULL THEN NULL ELSE json_set(m.data,
                '$.account_id',m.account,'$.folder',m.folder,'$.unread',json(CASE m.unread WHEN 1 THEN 'true' ELSE 'false' END),
                '$.starred',json(CASE m.starred WHEN 1 THEN 'true' ELSE 'false' END)) END,
                CASE WHEN m.id IS NULL THEN 'failed' ELSE 'queued' END,
                CASE WHEN m.id IS NULL THEN 'This message is unavailable. Refresh its folder or review its pending move.' END
                FROM scratch.mail_selection_rows s
                LEFT JOIN scratch.mail_review_lineage r ON r.selection=s.selection AND r.id=s.id
                LEFT JOIN mail_lineage l ON l.lineage=COALESCE((SELECT lineage FROM mail_lineage_alias WHERE alias=r.lineage),r.lineage)
                LEFT JOIN selectable_mail m ON m.id=l.id WHERE s.selection=? AND s.selected=1",
                params![id,selection.to_string()])?;
            let result = publish_admission(&tx, &id, &action)?;
            tx.execute("DELETE FROM scratch.mail_selections WHERE id=?",[selection.to_string()])?;
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
        self.claim_bulk_item_at(id, None).await
    }

    pub(crate) async fn claim_bulk_item_at(
        &self,
        id: String,
        position: Option<u64>,
    ) -> anyhow::Result<Option<Item>> {
        self.claim_bulk_item_inner(id, position, false).await
    }

    pub(crate) async fn claim_owned_bulk_item(
        &self,
        lease: &BulkLease,
        position: u64,
    ) -> anyhow::Result<Option<Item>> {
        anyhow::ensure!(
            Arc::ptr_eq(&self.0, &lease.store.0),
            "Use the lease for this mail cache"
        );
        self.claim_bulk_item_inner(lease.id.clone(), Some(position), true)
            .await
    }

    pub(crate) async fn bulk_item_state(
        &self,
        id: String,
        position: u64,
    ) -> anyhow::Result<String> {
        self.run(move |c| Ok(c.query_row("SELECT json_array(status,undo,receipt,error) FROM bulk_items WHERE job=? AND position=?", params![id,position as i64], |r|r.get(0))?)).await
    }

    async fn claim_bulk_item_inner(
        &self,
        id: String,
        position: Option<u64>,
        concurrent: bool,
    ) -> anyhow::Result<Option<Item>> {
        self.run(move |c| {
            let tx=c.transaction()?;
            if job(&tx,&id)?.paused { return Ok(None); }
            if !concurrent && tx.query_row("SELECT EXISTS(SELECT 1 FROM bulk_items WHERE job=? AND status IN ('running','repair'))", [&id], |r|r.get::<_,bool>(0))? { return Ok(None); }
            for _ in 0..50 {
            let next=tx.query_row("SELECT position,id,original,undo,status,receipt,error FROM bulk_items i WHERE job=?1 AND status='queued' AND (?2 IS NULL OR position=?2)
                AND NOT EXISTS(SELECT 1 FROM bulk_admissions a JOIN bulk_admissions prior ON prior.lineage=a.lineage AND prior.sequence<a.sequence
                    JOIN bulk_items p ON p.job=prior.job AND p.position=prior.position
                    WHERE a.job=i.job AND a.position=i.position AND p.status IN ('queued','running','repair','uncertain'))
                ORDER BY position LIMIT 1",params![id,position.map(|v|v as i64)],read_item).optional()?;
            let Some(next)=next else{tx.commit()?; return Ok(None)};
            let mut item=parse_item(&id,next)?;
            if let Err(error) = intents::prepare(&tx, &mut item) {
                tx.execute("UPDATE bulk_items SET status='failed',error=? WHERE job=? AND position=?", params![error.to_string(),id,item.position as i64])?;
                intents::refresh_item(&tx, &item)?;
                bump(&tx)?;
                continue;
            }
            if concurrent && tx.query_row("SELECT EXISTS(SELECT 1 FROM bulk_items i JOIN bulk_jobs j ON j.id=i.job
                WHERE i.job=?1 AND i.status IN ('running','repair') AND
                (json_extract(j.action,'$.Move.account') IS NOT NULL OR
                 COALESCE((SELECT m.account FROM bulk_admissions a JOIN mail_lineage l ON l.lineage=a.lineage JOIN messages m ON m.id=l.id WHERE a.job=i.job AND a.position=i.position),json_extract(i.original,'$.account_id'), '')=?2))",
                params![id,item.original.as_ref().map(|mail|mail.account_id.as_str()).unwrap_or("")], |r|r.get::<_,bool>(0))? {
                tx.commit()?;
                return Ok(None);
            }
            tx.execute("UPDATE bulk_items SET status='running' WHERE job=? AND position=? AND status='queued'",params![id,item.position as i64])?;
            item.status="running".into();
            tx.commit()?; return Ok(Some(item));
            }
            tx.commit()?;
            Ok(None)
        }).await
    }
    /// Persist provider acknowledgement before attempting the cache transaction.
    pub async fn acknowledge_bulk_flags(&self, item: Item, receipt: Receipt) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(receipt, Receipt::Flags { .. }),
            "Expected a flag receipt"
        );
        self.run(move |c| {
            let tx = c.transaction()?;
            let current: (String, bool) = tx.query_row(
                "SELECT status,undo FROM bulk_items WHERE job=? AND position=?",
                params![item.job, item.position as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
            anyhow::ensure!(current == ("running".into(), item.undo), "This flag acknowledgement is no longer current");
            tx.execute("INSERT INTO bulk_flag_receipts(job,position,undo,receipt) VALUES(?,?,?,?)",
                params![item.job, item.position as i64, item.undo, serde_json::to_string(&receipt)?])?;
            tx.execute("UPDATE bulk_items SET status='repair',error='Server confirmed. Waiting to update the local cache.' WHERE job=? AND position=?", params![item.job, item.position as i64])?;
            bump(&tx)?;
            tx.commit()?;
            Ok(())
        }).await
    }
    /// Read one acknowledged step for cache-only repair under its job lease.
    pub async fn pending_bulk_flag_repair(
        &self,
        lease: &BulkLease,
    ) -> anyhow::Result<Option<Item>> {
        self.pending_bulk_flag_repair_at(lease, None).await
    }

    pub(crate) async fn pending_bulk_flag_repair_at(
        &self,
        lease: &BulkLease,
        position: Option<u64>,
    ) -> anyhow::Result<Option<Item>> {
        anyhow::ensure!(
            Arc::ptr_eq(&self.0, &lease.store.0),
            "Use the lease for this mail cache"
        );
        let id = lease.id.clone();
        self.run(move |c| {
            let row = c.query_row("SELECT i.position,i.id,i.original,i.undo,i.status,i.receipt,i.error FROM bulk_items i JOIN bulk_flag_receipts r ON r.job=i.job AND r.position=i.position AND r.undo=i.undo WHERE i.job=?1 AND (?2 IS NULL OR i.position=?2) ORDER BY i.position LIMIT 1", params![id,position.map(|v|v as i64)], read_item).optional()?;
            row.map(|row| parse_item(&id, row)).transpose()
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
            anyhow::ensure!(matches!(current.0.as_str(), "running" | "repair") && current.1 == item.undo,"This mail-operation result is no longer current");
            let acknowledged: Option<(bool, String)> = tx.query_row(
                "SELECT undo,receipt FROM bulk_flag_receipts WHERE job=? AND position=?",
                params![item.job,item.position as i64], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
            let result = if let Some((undo, receipt)) = acknowledged {
                anyhow::ensure!(undo == item.undo, "This flag receipt belongs to another phase");
                let receipt: Receipt = serde_json::from_str(&receipt)?;
                let Receipt::Flags { after, .. } = &receipt else { anyhow::bail!("Invalid stored flag receipt") };
                let original = item.original.as_ref().context("The acknowledged message is unavailable")?;
                let changed = tx.execute("UPDATE messages SET unread=COALESCE(?,unread),starred=COALESCE(?,starred) WHERE id=? AND account=? AND folder=? AND json_extract(data,'$.remote_id') IS ?",
                    params![after.unread,after.starred,original.id,original.account_id,original.folder,original.remote_id])?;
                anyhow::ensure!(changed == 1, "The server confirmed this change, but its cached message changed. Refresh History to repair it.");
                write_ledger::record_acknowledged(&tx, &original.account_id, &original.id, WriteKind::Flags, chrono::Utc::now().timestamp())?;
                tx.execute("DELETE FROM bulk_flag_receipts WHERE job=? AND position=?",params![item.job,item.position as i64])?;
                Ok(receipt)
            } else { result };
            let claimed: Vec<String> = tx.prepare("SELECT id FROM bulk_effects WHERE job=? AND position=?")?
                .query_map(params![item.job,item.position as i64], |r|r.get(0))?.collect::<rusqlite::Result<_>>()?;
            tx.execute("DELETE FROM bulk_effects WHERE job=? AND position=?",params![item.job,item.position as i64])?;
            match result {
                Ok(receipt) => {
                    if matches!(receipt, Receipt::Superseded) {
                        tx.execute("UPDATE bulk_items SET status='cancelled',error='A newer decision owns this field. This change was not sent.' WHERE job=? AND position=?", params![item.job,item.position as i64])?;
                        intents::refresh_item(&tx, &item)?;
                        bump(&tx)?;
                        let result = job(&tx, &item.job)?;
                        tx.commit()?;
                        return Ok(result);
                    }
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
                    let cancelled = !uncertain && !item.undo && job(&tx,&item.job)?.undo_requested;
                    let status = if uncertain {"uncertain"} else if cancelled {"cancelled"} else {"failed"};
                    tx.execute("UPDATE bulk_items SET status=?,error=? WHERE job=? AND position=?",params![status,error,item.job,item.position as i64])?;
                    if uncertain {
                        for id in claimed { tx.execute("INSERT INTO bulk_effects(id,job,position) VALUES(?,?,?)",params![id,item.job,item.position as i64])?; }
                    }
                }
            }
            intents::refresh_item(&tx, &item)?;
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
            tx.execute("UPDATE bulk_effects SET account=NULL,folder=NULL,unread=NULL,starred=NULL WHERE job=? AND position IN (SELECT position FROM bulk_items WHERE job=? AND status IN ('running','repair') AND undo=0)",params![id,id])?;

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
            tx.execute("UPDATE bulk_items SET status='cancelled',error='A newer decision owns these fields. Undo was not sent.'
                WHERE job=? AND status='queued' AND undo=1
                AND EXISTS(SELECT 1 FROM bulk_admissions a WHERE a.job=bulk_items.job AND a.position=bulk_items.position)
                AND NOT EXISTS(SELECT 1 FROM bulk_admissions a JOIN bulk_field_owners o ON o.sequence=a.sequence
                    WHERE a.job=bulk_items.job AND a.position=bulk_items.position)", [&id])?;
            intents::refresh(&tx, &id)?;
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
            if matches!(job(&tx, &item.job)?.action, Action::Move { .. }) && !intents::owns(&tx, &item, "location")? {
                return Err(crate::bulk::Superseded.into());
            }
            anyhow::ensure!(item.undo || id==item.id,"The forward message identity changed");
            let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM bulk_admissions WHERE job=? AND position=?)", params![item.job,item.position as i64], |r|r.get(0))?;
            let unresolved_move = item.undo && matches!(&item.receipt, Some(Receipt::Move(receipt)) if receipt.current.is_none());
            anyhow::ensure!(!unresolved_move || id != item.id, "The move destination has not been verified. Refresh its folder before Undo.");
            if bound && !unresolved_move {
                anyhow::ensure!(tx.query_row("SELECT EXISTS(SELECT 1 FROM bulk_admissions a JOIN mail_lineage l ON l.lineage=a.lineage WHERE a.job=? AND a.position=? AND l.id=?)", params![item.job,item.position as i64,id], |r|r.get::<_,bool>(0))?, "The message was replaced. Refresh before trying again.");
                return Ok(());
            }
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
            let owner = c.query_row("SELECT a.job FROM bulk_admissions a
                JOIN mail_lineage l ON l.lineage=a.lineage JOIN bulk_items i ON i.job=a.job AND i.position=a.position
                WHERE l.id=? AND i.status IN ('queued','running','repair','uncertain')
                ORDER BY a.sequence LIMIT 1", [&id], |r|r.get::<_, String>(0)).optional()?;
            if owner.is_some() { return Ok(owner); }
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
/// no disk file; every store also excludes competing in-process executors.
pub struct BulkLease {
    store: Store,
    id: String,
    _file: Option<std::fs::File>,
    _local: worker::Lease,
}
impl Store {
    pub async fn bulk_lease(&self, id: String) -> anyhow::Result<BulkLease> {
        // Own the claim across awaits; cancellation and disk-lock errors release it.
        let local = self.0.lease(id.clone()).await?;
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
            _local: local,
        })
    }
    /// Called only after obtaining the job lease: a recorded running step now
    /// has no live executor. Retain it for review without changing user decisions.
    pub async fn resume_bulk(&self, lease: &BulkLease) -> anyhow::Result<Job> {
        anyhow::ensure!(
            Arc::ptr_eq(&self.0, &lease.store.0),
            "Use the lease for this mail cache"
        );
        let id = lease.id.clone();
        self.run(move |c| {
            let tx=c.transaction()?;
            tx.execute("UPDATE bulk_effects SET account=NULL,folder=NULL,unread=NULL,starred=NULL WHERE job=? AND position IN (SELECT position FROM bulk_items WHERE job=? AND status='running' AND NOT EXISTS(SELECT 1 FROM bulk_flag_receipts r WHERE r.job=bulk_items.job AND r.position=bulk_items.position))",params![id,id])?;
            let recovered = tx.execute("UPDATE bulk_items SET status='uncertain',error='Shep closed before this change was acknowledged. Refresh and check the source and destination folders before resolving it.' WHERE job=? AND status='running' AND NOT EXISTS(SELECT 1 FROM bulk_flag_receipts r WHERE r.job=bulk_items.job AND r.position=bulk_items.position)",[&id])?;
            if recovered > 0 { intents::refresh(&tx, &id)?; bump(&tx)?; }
            let result=job(&tx,&id)?;tx.commit()?;Ok(result)
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
            intents::refresh(&tx, &id)?;
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
        self.run(move |c|Ok(c.query_row("SELECT id FROM bulk_jobs WHERE id>? AND paused=0 AND EXISTS(SELECT 1 FROM bulk_items WHERE job=bulk_jobs.id AND status IN ('queued','running','repair')) ORDER BY id LIMIT 1",[after],|r|r.get(0)).optional()?)).await
    }
}
