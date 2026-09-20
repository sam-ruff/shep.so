use super::*;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
mod removal;
pub(crate) use removal::fence_import as fence_removal_import;
pub(super) use removal::pending as pending_removals;
pub use removal::{RemovalJob, RemovalStage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
pub enum ConnectionKind {
    Account,
    Calendar,
}
impl ConnectionKind {
    pub fn key(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Calendar => "calendar",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ConnectionRef {
    pub kind: ConnectionKind,
    pub id: String,
}
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct RemovalPreview {
    #[serde(default)]
    pub removal_epoch: u64,
    pub target: ConnectionRef,
    pub name: String,
    pub address: String,
    pub messages: usize,
    pub drafts: usize,
    pub events: usize,
    pub transfers: usize,
    pub outgoing: usize,
    pub mail_history: usize,
    pub fingerprint: String,
}
#[derive(Debug, Clone)]
pub struct CredentialCleanup {
    pub target: ConnectionRef,
    pub key: String,
}

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    super::account_setup::schema(c)?;
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS connection_tombstones (
        kind TEXT NOT NULL,id TEXT NOT NULL,revision INTEGER NOT NULL,google_data TEXT,
        PRIMARY KEY(kind,id));
        CREATE TABLE IF NOT EXISTS connection_removal_epochs (
        kind TEXT NOT NULL,id TEXT NOT NULL,revision INTEGER NOT NULL,PRIMARY KEY(kind,id));
        CREATE TABLE IF NOT EXISTS credential_cleanup (
        kind TEXT NOT NULL,id TEXT NOT NULL,key TEXT NOT NULL,PRIMARY KEY(kind,id,key));",
    )?;
    removal::schema(c)?;
    Ok(())
}
pub(super) fn changed(c: &Connection) -> anyhow::Result<u64> {
    let revision: u64 = get(c, "connections_revision")?;
    let revision = revision
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .context("Connection revision overflow")?;
    put(c, "connections_revision", &revision)?;
    Ok(revision)
}
pub(super) fn removed(
    c: &Connection,
    kind: ConnectionKind,
    id: &str,
) -> anyhow::Result<Option<u64>> {
    Ok(c.query_row(
        "SELECT revision FROM connection_tombstones WHERE kind=? AND id=?",
        params![kind.key(), id],
        |r| r.get::<_, i64>(0),
    )
    .optional()?
    .map(u64::try_from)
    .transpose()?)
}
pub(super) fn allow(c: &Connection, kind: ConnectionKind, id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        removed(c, kind, id)?.is_none(),
        "This connection was removed. Add it again before making changes."
    );
    Ok(())
}
pub(super) fn revive(c: &Connection, kind: ConnectionKind, id: &str) -> anyhow::Result<()> {
    removal::allow_reconnect(c, kind, id)?;
    clear_removal(c, kind, id)
}
pub(super) fn restore_removed(
    c: &Connection,
    kind: ConnectionKind,
    id: &str,
) -> anyhow::Result<()> {
    let unfinished:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM connection_tombstones WHERE kind=? AND id=? AND pending IS NOT NULL AND json_extract(pending,'$.local_done')=0)",params![kind.key(),id],|r|r.get(0))?;
    anyhow::ensure!(
        !unfinished,
        "Finish this connection's local removal before restoring its backup."
    );
    // Restore owns the lifecycle grant and replaces credentials after this transaction.
    clear_removal(c, kind, id)
}
fn clear_removal(c: &Connection, kind: ConnectionKind, id: &str) -> anyhow::Result<()> {
    c.execute(
        "DELETE FROM connection_tombstones WHERE kind=? AND id=?",
        params![kind.key(), id],
    )?;
    c.execute(
        "DELETE FROM credential_cleanup WHERE kind=? AND id=?",
        params![kind.key(), id],
    )?;
    Ok(())
}
pub(super) fn transfers(c: &Connection, account: &str) -> anyhow::Result<Vec<(String, String)>> {
    let rows = c.prepare("SELECT k.key,k.value,m.account FROM kv k LEFT JOIN messages m ON m.id=substr(k.key,10) WHERE k.key LIKE 'transfer:%' ORDER BY k.key")?
        .query_map([], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?)))?
        .collect::<Result<Vec<_>,_>>()?;
    let mut found = Vec::new();
    for (key, value, source) in rows {
        let journal: Option<(String, String, String)> = serde_json::from_str(&value)?;
        if let Some((destination, _, _)) = journal
            && (source.as_deref() == Some(account) || destination == account)
        {
            found.push((key, value));
        }
    }
    Ok(found)
}
fn preview(c: &Connection, target: ConnectionRef) -> anyhow::Result<RemovalPreview> {
    let mut digest = Sha256::new();
    let (name, address) = match target.kind {
        ConnectionKind::Account => {
            let account = get::<Vec<Account>>(c, "accounts")?
                .into_iter()
                .find(|a| a.id == target.id)
                .context("This account is no longer connected.")?;
            digest.update(serde_json::to_vec(&account)?);
            let setup: Option<String> = c.query_row("SELECT a.data FROM account_setup_attempts a JOIN account_setup_current p ON p.attempt=a.id WHERE p.account=?", [&target.id], |r| r.get(0)).optional()?;
            if let Some(setup) = setup {
                digest.update(setup);
            }
            (account.name, account.email)
        }
        ConnectionKind::Calendar => {
            let source = get::<Vec<CalendarSource>>(c, "calendars")?
                .into_iter()
                .find(|s| s.id == target.id)
                .context("This calendar is no longer connected.")?;
            digest.update(serde_json::to_vec(&source)?);
            (source.name, source.url)
        }
    };
    let mut out = RemovalPreview {
        removal_epoch: removal::epoch(c, &target)?,
        target,
        name,
        address,
        messages: 0,
        drafts: 0,
        events: 0,
        transfers: 0,
        outgoing: 0,
        mail_history: 0,
        fingerprint: String::new(),
    };
    match out.target.kind {
        ConnectionKind::Account => {
            let mut statement = c.prepare("SELECT id FROM messages WHERE account=? ORDER BY id")?;
            let mut rows = statement.query([&out.target.id])?;
            while let Some(row) = rows.next()? {
                let id: String = row.get(0)?;
                digest.update((id.len() as u64).to_le_bytes());
                digest.update(id);
                out.messages += 1;
            }
            for draft in drafts::snapshot(c)?
                .drafts
                .into_iter()
                .filter(|d| d.account_id == out.target.id)
            {
                digest.update(serde_json::to_vec(&draft)?);
                // Text autosaves intentionally skip attachments in Draft's
                // serializer. Removal must also review the current file list.
                digest.update(serde_json::to_vec(&draft.attachments)?);
                out.drafts += 1;
            }
            let deliveries = c
                .prepare("SELECT data,stage FROM outgoing WHERE account=? ORDER BY attempt")?
                .query_map([&out.target.id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (data, stage) in deliveries {
                digest.update(data);
                if !matches!(stage.as_str(), "Complete" | "Released") {
                    out.outgoing += 1;
                }
            }
            let pending = transfers(c, &out.target.id)?;
            out.transfers = pending.len();
            digest.update(serde_json::to_vec(&pending)?);
            let (history, pending_bulk) = bulk::account_review(c, &out.target.id, &mut digest)?;
            out.mail_history = history;
            out.transfers += pending_bulk;
            out.transfers += move_journal::account_review(c, &out.target.id, &mut digest)?;
            out.transfers += folder_actions::account_review(c, &out.target.id, &mut digest)?;
            let mut creations=c.prepare("SELECT data FROM folder_creations WHERE account=? AND data IS NOT NULL AND json_extract(data,'$.stage') NOT IN ('succeeded','cancelled','dismissed') ORDER BY json_extract(data,'$.id')")?;
            let mut rows = creations.query([&out.target.id])?;
            while let Some(row) = rows.next()? {
                let data: String = row.get(0)?;
                digest.update((data.len() as u64).to_le_bytes());
                digest.update(data);
                out.transfers += 1;
            }
        }
        ConnectionKind::Calendar => {
            let mut statement = c.prepare("SELECT data FROM events WHERE source=? ORDER BY id")?;
            let mut rows = statement.query([&out.target.id])?;
            while let Some(row) = rows.next()? {
                let data: String = row.get(0)?;
                digest.update(data);
                out.events += 1;
            }
        }
    }
    out.fingerprint = format!("{:x}", digest.finalize());
    Ok(out)
}
impl Store {
    pub async fn check_connection(&self, target: ConnectionRef) -> anyhow::Result<()> {
        self.run(move |c| allow(c, target.kind, &target.id)).await
    }
    pub async fn removal_preview(&self, target: ConnectionRef) -> anyhow::Result<RemovalPreview> {
        self.run(move |c| preview(c, target)).await
    }
    pub async fn remove_connection(
        &self,
        expected: RemovalPreview,
        cancel_transfers: bool,
    ) -> anyhow::Result<()> {
        let review = expected.clone();
        let saved:Option<String>=self.run(move |c|Ok(c.query_row("SELECT json_extract(pending,'$.id') FROM connection_tombstones WHERE kind=? AND id=? AND json_extract(pending,'$.fingerprint')=? AND json_extract(pending,'$.review_epoch')=? AND json_extract(pending,'$.cancel_transfers')=?",params![review.target.kind.key(),review.target.id,review.fingerprint,review.removal_epoch as i64,cancel_transfers],|row|row.get(0)).optional()?)).await?;
        let mut job = self
            .admit_connection_removal(
                saved.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                expected,
                cancel_transfers,
            )
            .await?;
        while !job.local_done {
            job = self.finish_connection_removal(job).await?;
        }
        Ok(())
    }

    pub(crate) async fn finish_connection_removal(
        &self,
        expected: RemovalJob,
    ) -> anyhow::Result<RemovalJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = removal::current(&tx, &expected)?;
            anyhow::ensure!(job.stage==RemovalStage::Queued,"Refresh removal progress before continuing cleanup.");
            anyhow::ensure!(!job.local_done, "This connection's local data was already removed.");
            let target = &expected.target;
            let mut keys = Vec::new();
            match target.kind {
                ConnectionKind::Account => {
                    super::account_setup::remove(&tx, &target.id)?;
                    let mut accounts: Vec<Account> = get(&tx, "accounts")?;
                    accounts.retain(|a| a.id != target.id);
                    super::profile_sync::join::reconnected(&tx, &target.id)?;
                    put(&tx, "accounts", &accounts)?;
                    if job.device_credentials { keys.extend([target.id.clone(),format!("{}:smtp",target.id)]); }
                    for (key,_) in transfers(&tx, &target.id)? { tx.execute("DELETE FROM kv WHERE key=?", [key])?; }
                    tx.execute("DELETE FROM draft_sent WHERE id IN (SELECT draft FROM outgoing WHERE account=?)",[&target.id])?;
                    tx.execute("DELETE FROM outgoing WHERE account=?",[&target.id])?;
                    tx.execute("DELETE FROM sent_folders WHERE account=?",[&target.id])?;
                    outgoing::changed(&tx)?;
                    bulk::remove_account(&tx,&target.id)?;
                    tx.execute("DELETE FROM mail_identity_history WHERE account=?", [&target.id])?;
                    move_journal::remove_account(&tx, &target.id)?;
                    tx.execute("DELETE FROM folder_jobs WHERE account=?", [&target.id])?;
                    tx.execute("DELETE FROM folder_creations WHERE account=?", [&target.id])?;
                    tx.execute("DELETE FROM messages WHERE id IN (SELECT id FROM messages WHERE account=? ORDER BY id LIMIT 50)", [&target.id])?;
                    let remains:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM messages WHERE account=?)",[&target.id],|r|r.get(0))?;
                    if remains {
                        job.stage=RemovalStage::Queued;
                        removal::save(&tx,&mut job)?;
                        tx.commit()?;
                        return Ok(job);
                    }
                    super::notifications::remove_account(&tx, &target.id)?;
                    tx.execute("DELETE FROM conversation_tokens WHERE account=?", [&target.id])?;
                    let mut folder_map: std::collections::HashMap<String,Vec<String>> = get(&tx, "account_folders")?;
                    folder_map.remove(&target.id); put(&tx, "account_folders", &folder_map)?;
                    let mut catalogs: std::collections::HashMap<String,Vec<crate::folders::Mailbox>> = get(&tx, "folder_catalogs")?;
                    catalogs.remove(&target.id); put(&tx, "folder_catalogs", &catalogs)?;
                    let folders: std::collections::BTreeSet<_> = folder_map.values().flatten().cloned().collect();
                    put(&tx, "folders", &folders)?;
                    let mut drafts: Vec<Draft> = get(&tx, "drafts")?;
                    for draft in drafts.iter().filter(|d| d.account_id == target.id) {
                        tx.execute("DELETE FROM draft_attachments WHERE draft=?", [&draft.id])?;
                        tx.execute("DELETE FROM draft_sent WHERE id=?", [&draft.id])?;
                    }
                    drafts.retain(|d| d.account_id != target.id); put(&tx, "drafts", &drafts)?;
                    drafts::changed(&tx)?;
                }
                ConnectionKind::Calendar => {
                    let mut sources: Vec<CalendarSource> = get(&tx, "calendars")?;
                    if job.device_credentials && job.calendar_key { keys.push(target.id.clone()); }
                    sources.retain(|s| s.id != target.id); put(&tx, "calendars", &sources)?;
                    let mut archived: std::collections::HashSet<String> = get(&tx, "google_archived")?;
                    archived.remove(&target.id);
                    put(&tx, "google_archived", &archived)?;
                    tx.execute("DELETE FROM events WHERE id IN (SELECT id FROM events WHERE source=? ORDER BY id LIMIT 50)", [&target.id])?;
                    calendar_changed(&tx)?;
                    let remains:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE source=?)",[&target.id],|r|r.get(0))?;
                    if remains {
                        job.stage=RemovalStage::Queued;
                        removal::save(&tx,&mut job)?;
                        tx.commit()?;
                        return Ok(job);
                    }
                }
            }
            for key in keys { tx.execute("INSERT OR IGNORE INTO credential_cleanup VALUES(?,?,?)", params![target.kind.key(),target.id,key])?; }
            job.local_done = true;
            let pending_keys:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM credential_cleanup WHERE kind=? AND id=?)",params![target.kind.key(),target.id],|r|r.get(0))?;
            job.stage = if pending_keys {RemovalStage::Cleanup} else {RemovalStage::Succeeded};
            job.error = None;
            removal::save(&tx, &mut job)?;
            tx.commit()?;
            Ok(job)
        }).await
    }
    pub async fn cleanup_jobs(&self) -> anyhow::Result<Vec<CredentialCleanup>> {
        self.run(|c| {
            Ok(
                c.prepare("SELECT kind,id,key FROM credential_cleanup ORDER BY kind,id,key")?
                    .query_map([], |r| {
                        Ok(CredentialCleanup {
                            target: ConnectionRef {
                                kind: if r.get::<_, String>(0)? == "account" {
                                    ConnectionKind::Account
                                } else {
                                    ConnectionKind::Calendar
                                },
                                id: r.get(1)?,
                            },
                            key: r.get(2)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?,
            )
        })
        .await
    }
    pub async fn finish_credential_cleanup(&self, job: CredentialCleanup) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            tx.execute(
                "DELETE FROM credential_cleanup WHERE kind=? AND id=? AND key=?",
                params![job.target.kind.key(), job.target.id, job.key],
            )?;
            changed(&tx)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn credential_in_use(&self, key: String) -> anyhow::Result<bool> {
        self.run(move |c| {
            let slotted = c
                .prepare("SELECT account FROM account_credential_slots")?
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<std::collections::HashSet<_>, _>>()?;
            Ok(super::account_setup::in_use(c, &key)?
                || get::<Vec<Account>>(c, "accounts")?.iter().any(|a| {
                    !slotted.contains(&a.id) && (a.id == key || format!("{}:smtp", a.id) == key)
                })
                || get::<Vec<CalendarSource>>(c, "calendars")?
                    .iter()
                    .any(|s| s.kind == CalendarKind::CalDav && s.id == key))
        })
        .await
    }
    pub async fn removed_google_calendars(&self) -> anyhow::Result<Vec<CalendarSource>> {
        self.run(|c| c.prepare("SELECT google_data FROM connection_tombstones WHERE google_data IS NOT NULL ORDER BY id")?
            .query_map([], |r| r.get::<_,String>(0))?.map(|row| Ok(serde_json::from_str(&row?)?)).collect()).await
    }
    pub async fn check_calendar_reconnect(
        &self,
        id: String,
        observed_revision: u64,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            // Keep the last removal generation after an explicit reconnect, so
            // an older queued setup cannot overwrite the newly saved credential.
            let revision: Option<i64> = c.query_row("SELECT revision FROM connection_removal_epochs WHERE kind='calendar' AND id=?", [&id], |r| r.get(0)).optional()?;
            anyhow::ensure!(revision.is_none_or(|revision| revision as u64 <= observed_revision), "This calendar was removed after setup started. Find calendars again to reconnect it.");
            Ok(())
        }).await
    }
}
