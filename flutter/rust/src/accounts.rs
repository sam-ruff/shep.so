//! Reviewed local account removal. No provider calls or credential bytes.
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Removal {
    pub id: String,
    pub name: String,
    pub email: String,
    pub messages: u64,
    pub drafts: u64,
    pub files: u64,
    pub outgoing: u64,
    pub unresolved: u64,
    pub moves: u64,
    #[serde(default)]
    pub groups: u64,
    pub fingerprint: String,
}

/// Group work that removal must discard explicitly: queued, in-flight,
/// failed, uncertain and inverse steps, never completed receipts.
pub(crate) const ACTIVE_GROUP_ITEMS: &str = "('pending','sending','undoing','reversing','failed','uncertain','undo_failed','undo_uncertain')";

pub fn available(db: &Connection, id: &str) -> Result<()> {
    let removed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM removed_accounts WHERE id=?1)",
        [id],
        |r| r.get(0),
    )?;
    anyhow::ensure!(
        !removed,
        "This account was removed. Add it again as a new account in Preferences."
    );
    Ok(())
}

pub fn preview(db: &Connection, id: &str) -> Result<Removal> {
    let account = crate::operations::stored_account(db, id)?;
    let mut digest = Sha256::new();
    digest.update(serde_json::to_vec(&account)?);
    digest.update(crate::connections::stored_slot(db, id)?);
    // Hash bounded metadata one row at a time. Mail/attachment bytes are not
    // loaded into the review, and draft-file revisions capture changed content.
    for query in [
        "SELECT json_array(id,remote_id,folder,unread,starred) FROM mail WHERE account_id=?1 ORDER BY id",
        "SELECT json_array(d.id,d.revision,d.content,COALESCE(f.revision,0)) FROM drafts d LEFT JOIN draft_file_revisions f ON d.id=f.id WHERE json_extract(d.content,'$.account_id')=?1 ORDER BY d.id",
        "SELECT json_array(o.id,o.state,m.recovery,s.state,s.receipt,s.complete) FROM outgoing o LEFT JOIN outgoing_meta m ON m.id=o.id LEFT JOIN outgoing_sent s ON s.id=o.id WHERE o.account_id=?1 ORDER BY o.id",
        "SELECT json_array(p.id,p.destination) FROM pending_moves p JOIN mail m ON m.id=p.id WHERE m.account_id=?1 ORDER BY p.id",
        "SELECT json_array(job,position,state) FROM group_items WHERE account=?1 ORDER BY job,position",
    ] {
        let mut statement = db.prepare(query)?;
        let mut rows = statement.query([id])?;
        while let Some(row) = rows.next()? {
            let value: String = row.get(0)?;
            digest.update((value.len() as u64).to_le_bytes());
            digest.update(value);
        }
    }
    let count = |query: &str| -> Result<u64> {
        let n: i64 = db.query_row(query, [id], |r| r.get(0))?;
        Ok(n.try_into()?)
    };
    Ok(Removal {
        id: id.into(),
        name: account.name,
        email: account.email,
        messages: count("SELECT COUNT(*) FROM mail WHERE account_id=?1")?,
        drafts: count("SELECT COUNT(*) FROM drafts WHERE json_extract(content,'$.account_id')=?1")?,
        files: count(
            "SELECT COUNT(*) FROM draft_files f JOIN drafts d ON f.draft_id=d.id WHERE json_extract(d.content,'$.account_id')=?1",
        )?,
        outgoing: count("SELECT COUNT(*) FROM outgoing WHERE account_id=?1")?,
        unresolved: count(
            "SELECT COUNT(*) FROM outgoing o LEFT JOIN outgoing_meta m ON m.id=o.id LEFT JOIN outgoing_sent s ON s.id=o.id WHERE o.account_id=?1 AND ((m.recovery IS NULL AND o.state NOT IN ('delivered','rejected','cancelled')) OR (s.id IS NOT NULL AND s.complete=0 AND (m.recovery IS NULL OR m.recovery='marked')))",
        )?,
        moves: count(
            "SELECT COUNT(*) FROM pending_moves p JOIN mail m ON m.id=p.id WHERE m.account_id=?1",
        )?,
        groups: count(&format!(
            "SELECT COUNT(*) FROM group_items WHERE account=?1 AND state IN {ACTIVE_GROUP_ITEMS}"
        ))?,
        fingerprint: format!("{:x}", digest.finalize()),
    })
}

pub fn remove(db: &mut Connection, expected: Removal, discard_unresolved: bool) -> Result<()> {
    let tx = db.transaction()?;
    // An exact retry acknowledges an already committed removal, including after
    // a lost bridge response. IDs are never reused by Add account.
    let previous: Option<String> = tx
        .query_row(
            "SELECT fingerprint FROM removed_accounts WHERE id=?1",
            [&expected.id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(fingerprint) = previous {
        anyhow::ensure!(
            fingerprint == expected.fingerprint,
            "This account removal changed. Reopen Preferences."
        );
        return Ok(());
    }
    let current = preview(&tx, &expected.id)?;
    anyhow::ensure!(
        current.fingerprint == expected.fingerprint,
        "Local data changed while this review was open. Reload the counts before removing this account."
    );
    anyhow::ensure!(
        discard_unresolved
            || (current.unresolved == 0 && current.moves == 0 && current.groups == 0),
        "Review and confirm discarding unfinished delivery, move and group action records first. Removal cannot undo a server operation."
    );
    let id = &expected.id;
    crate::groups::fence_account(&tx, id)?;
    tx.execute("INSERT INTO discarded_drafts SELECT id,9223372036854775807 FROM drafts WHERE json_extract(content,'$.account_id')=?1 ON CONFLICT(id) DO UPDATE SET revision=excluded.revision",[id])?;
    tx.execute("INSERT INTO discarded_drafts SELECT draft_id,9223372036854775807 FROM outgoing WHERE account_id=?1 ON CONFLICT(id) DO UPDATE SET revision=excluded.revision",[id])?;
    tx.execute(
        "DELETE FROM drafts WHERE json_extract(content,'$.account_id')=?1",
        [id],
    )?;
    tx.execute(
        "DELETE FROM outgoing_meta WHERE id IN (SELECT id FROM outgoing WHERE account_id=?1)",
        [id],
    )?;
    tx.execute(
        "DELETE FROM outgoing_sent WHERE id IN (SELECT id FROM outgoing WHERE account_id=?1)",
        [id],
    )?;
    tx.execute("DELETE FROM outgoing WHERE account_id=?1", [id])?;
    tx.execute("DELETE FROM mail WHERE account_id=?1", [id])?;
    tx.execute("DELETE FROM folders WHERE account_id=?1", [id])?;
    tx.execute("UPDATE credential_slots SET state='cleanup',settings=NULL,expected=NULL WHERE account_id=?1", [id])?;
    tx.execute("DELETE FROM accounts WHERE id=?1", [id])?;
    tx.execute(
        "INSERT INTO removed_accounts(id,fingerprint,cleanup) VALUES(?1,?2,1)",
        params![id, current.fingerprint],
    )?;
    tx.commit()
        .context("Could not commit account removal. Local data is unchanged; retry.")?;
    Ok(())
}
