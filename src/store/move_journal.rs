//! Original MIME remains in messages; the journal protects it until the actual
//! destination identity has been validated and its cache relocation committed.
use super::*;
use crate::mail_actions::{
    MoveReceipt,
    journal::{MoveRecord, MoveStage},
};
use rusqlite::OptionalExtension;

pub(super) mod relocation;

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS mail_moves(
        token TEXT PRIMARY KEY, source_id TEXT NOT NULL UNIQUE,
        source_account TEXT NOT NULL, destination_account TEXT NOT NULL,
        cache_id TEXT REFERENCES messages(id) ON DELETE RESTRICT,
        stage TEXT NOT NULL, data TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS mail_moves_source ON mail_moves(source_account,stage,token);
        CREATE INDEX IF NOT EXISTS mail_moves_destination ON mail_moves(destination_account,stage,token);
        CREATE INDEX IF NOT EXISTS mail_moves_cache ON mail_moves(cache_id);
        CREATE INDEX IF NOT EXISTS mail_moves_lookup ON mail_moves(stage,COALESCE(json_extract(data,'$.attempted'),0),token);
        CREATE TRIGGER IF NOT EXISTS mail_moves_preserve_raw BEFORE UPDATE OF raw ON messages
        WHEN EXISTS(SELECT 1 FROM mail_moves WHERE cache_id=old.id) AND new.raw!=old.raw
        BEGIN SELECT RAISE(ABORT,'This original belongs to an unfinished move. Resolve it before replacing its content.'); END;
        CREATE TRIGGER IF NOT EXISTS mail_moves_preserve_identity BEFORE UPDATE OF account,folder,data ON messages
        WHEN EXISTS(SELECT 1 FROM mail_moves WHERE cache_id=old.id) AND (new.account!=old.account OR new.folder!=old.folder OR json_extract(new.data,'$.remote_id') IS NOT json_extract(old.data,'$.remote_id'))
        BEGIN SELECT RAISE(ABORT,'This message belongs to an unfinished move. Resolve it before changing its identity.'); END;
        CREATE VIEW IF NOT EXISTS selectable_mail AS SELECT m.* FROM messages m
        WHERE NOT EXISTS(SELECT 1 FROM mail_moves j WHERE j.cache_id=m.id);
        CREATE VIEW IF NOT EXISTS recovered_mail AS
        SELECT m.rowid AS rowid,m.id,m.account,m.folder,m.sender,m.subject,m.body,m.timestamp,m.unread,m.starred,m.data,m.raw,0 AS pending_move
        FROM messages m WHERE NOT EXISTS(SELECT 1 FROM mail_moves j WHERE j.cache_id=m.id AND j.stage='committed')
        UNION ALL
        SELECT m.rowid AS rowid,m.id,j.destination_account,json_extract(j.data,'$.receipt.folder'),m.sender,m.subject,m.body,m.timestamp,m.unread,m.starred,m.data,m.raw,1 AS pending_move
        FROM mail_moves j JOIN messages m ON m.id=j.cache_id WHERE j.stage='committed';
        CREATE VIEW IF NOT EXISTS recovered_bulk AS SELECT m.rowid AS rowid,m.id,
        COALESCE(e.account,m.account) AS account,COALESCE(e.folder,m.folder) AS folder,
        m.sender,m.subject,m.body,m.timestamp,COALESCE(e.unread,m.unread) AS unread,
        COALESCE(e.starred,m.starred) AS starred,m.data,m.raw,m.pending_move
        FROM recovered_mail m LEFT JOIN bulk_effects e ON e.id=m.id;")?;
    Ok(())
}
pub(super) fn has_projection(c: &Connection) -> anyhow::Result<bool> {
    Ok(c.query_row(
        "SELECT EXISTS(SELECT 1 FROM mail_moves WHERE stage='committed')",
        [],
        |r| r.get(0),
    )?)
}
pub(super) fn pending(c: &Connection) -> anyhow::Result<usize> {
    Ok(c.query_row(
        "SELECT COUNT(*) FROM mail_moves WHERE cache_id IS NOT NULL",
        [],
        |r| r.get::<_, i64>(0),
    )? as usize)
}
pub(super) fn for_cache(c: &Connection, id: &str) -> anyhow::Result<Option<MoveRecord>> {
    c.query_row("SELECT data FROM mail_moves WHERE cache_id=?", [id], |r| {
        r.get::<_, String>(0)
    })
    .optional()?
    .map(|s| serde_json::from_str(&s).map_err(Into::into))
    .transpose()
}
pub(super) fn project_detail(c: &Connection, mail: &mut Mail) -> anyhow::Result<()> {
    if let Some(record) = for_cache(c, &mail.id)? {
        if record.stage == MoveStage::Committed {
            mail.account_id = record.receipt.account;
            mail.folder = record.receipt.folder;
        }
        mail.remote_id.clear();
    }
    Ok(())
}
fn read(c: &Connection, token: &str) -> anyhow::Result<MoveRecord> {
    let data: String = c.query_row("SELECT data FROM mail_moves WHERE token=?", [token], |r| {
        r.get(0)
    })?;
    Ok(serde_json::from_str(&data)?)
}
fn replace(c: &Connection, expected: &MoveRecord, next: &MoveRecord) -> anyhow::Result<()> {
    let n = c.execute(
        "UPDATE mail_moves SET stage=?,data=? WHERE token=? AND data=?",
        params![
            next.stage.key(),
            serde_json::to_string(next)?,
            expected.token,
            serde_json::to_string(expected)?
        ],
    )?;
    anyhow::ensure!(
        n == 1,
        "The move recovery changed. Refresh it before trying again."
    );
    Ok(())
}

fn located(c: &Connection, expected: MoveRecord, mail: Mail) -> anyhow::Result<MoveRecord> {
    anyhow::ensure!(
        expected.stage == MoveStage::Committed,
        "The server has not confirmed this move."
    );
    let mut next = expected.clone();
    next.stage = MoveStage::Located;
    next.receipt.current = Some(mail.clone());
    next.error = None;
    expected.validate_receipt(&next.receipt)?;
    replace(c, &expected, &next)?;
    c.execute(
        "UPDATE mail_moves SET cache_id=NULL WHERE token=?",
        [&expected.token],
    )?;
    super::mail_actions::relocate(c, &expected.original, &mail)?;
    c.execute(
        "UPDATE messages SET unread=?,starred=? WHERE id=?",
        params![mail.unread, mail.starred, mail.id],
    )?;
    Ok(next)
}

pub(super) fn account_review(
    c: &Connection,
    account: &str,
    digest: &mut sha2::Sha256,
) -> anyhow::Result<usize> {
    use sha2::Digest;
    let mut statement=c.prepare("SELECT data,stage FROM mail_moves WHERE source_account=?1 OR destination_account=?1 ORDER BY token")?;
    let mut rows = statement.query([account])?;
    let mut pending = 0;
    while let Some(row) = rows.next()? {
        let data: String = row.get(0)?;
        digest.update((data.len() as u64).to_le_bytes());
        digest.update(data);
        pending += usize::from(!matches!(
            row.get::<_, String>(1)?.as_str(),
            "located" | "kept"
        ));
    }
    Ok(pending)
}
pub(super) fn remove_account(c: &Connection, account: &str) -> anyhow::Result<()> {
    // Preserve other accounts' originals using local identities. The old UID
    // may already have been expunged and must never reach another provider call.
    loop {
        let record:Option<String>=c.query_row("SELECT data FROM mail_moves WHERE destination_account=?1 AND source_account!=?1 AND cache_id IS NOT NULL LIMIT 1",[account],|r|r.get(0)).optional()?;
        let Some(record) = record else {
            break;
        };
        keep_local(c, serde_json::from_str(&record)?)?;
    }
    c.execute(
        "DELETE FROM mail_moves WHERE source_account=?1 OR destination_account=?1",
        [account],
    )?;
    Ok(())
}

fn prepare(c: &Connection, record: &MoveRecord) -> anyhow::Result<()> {
    anyhow::ensure!(
        record.stage == MoveStage::Started
            && record.receipt.current.is_none()
            && record.receipt.fingerprint.is_some(),
        "A new move requires its original identity proof."
    );
    record.validate_receipt(&record.receipt)?;
    for account in [&record.original.account_id, &record.receipt.account] {
        connections::allow(c, ConnectionKind::Account, account)?;
        folder_actions::idle(c, account)?;
    }
    let (data, raw): (String, Vec<u8>) = c.query_row(
        "SELECT data,raw FROM messages WHERE id=? AND account=? AND folder=?",
        params![
            record.original.id,
            record.original.account_id,
            record.original.folder
        ],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let saved: Mail = serde_json::from_str(&data)?;
    anyhow::ensure!(
        saved.remote_id == record.original.remote_id
            && record
                .receipt
                .fingerprint
                .as_ref()
                .is_some_and(|f| f.matches(&raw)),
        "The cached message changed before moving. Refresh its folder."
    );
    c.execute("INSERT INTO mail_moves(token,source_id,source_account,destination_account,cache_id,stage,data) VALUES(?,?,?,?,?,?,?)",
                params![record.token,record.original.id,record.original.account_id,record.receipt.account,record.original.id,record.stage.key(),serde_json::to_string(&record)?])
                .context("This message already has a move record. Review its recovery before moving again.")?;
    Ok(())
}

fn keep_local(c: &Connection, expected: MoveRecord) -> anyhow::Result<MoveRecord> {
    let mut mail = expected.original.clone();
    let (unread, starred) = c.query_row(
        "SELECT unread,starred FROM messages WHERE id=?",
        [&mail.id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    mail.unread = unread;
    mail.starred = starred;
    mail.remote_id = format!("local-recovered-{}", expected.token);
    mail.id = format!("{}:{}:{}", mail.account_id, mail.folder, mail.remote_id);
    let mut next = expected.clone();
    next.stage = MoveStage::Kept;
    next.retained = Some(mail.clone());
    next.error = None;
    replace(c, &expected, &next)?;
    c.execute(
        "UPDATE mail_moves SET cache_id=NULL WHERE token=?",
        [&expected.token],
    )?;
    super::mail_actions::relocate(c, &expected.original, &mail)?;
    Ok(next)
}

impl Store {
    pub async fn mail_move_for_undo(
        &self,
        token: String,
        original: Mail,
        receipt: MoveReceipt,
    ) -> anyhow::Result<MoveRecord> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let record = read(&tx, &token)?;
            record.validate_receipt(&receipt)?;
            anyhow::ensure!(
                matches!(record.stage, MoveStage::Committed | MoveStage::Located),
                "Finish recovering the original move before Undo."
            );
            let same = |a: &Mail, b: &Mail| {
                a.id == b.id
                    && a.account_id == b.account_id
                    && a.folder == b.folder
                    && a.remote_id == b.remote_id
            };
            if !same(&record.original, &original) {
                let previous: Option<String> = tx
                    .query_row(
                        "SELECT data FROM mail_moves WHERE source_id=? AND stage='located'",
                        [&original.id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let previous: MoveRecord = serde_json::from_str(
                    &previous.context("This recovery belongs to a different original message.")?,
                )?;
                anyhow::ensure!(
                    same(&previous.original, &original)
                        && previous.receipt.account == original.account_id
                        && previous
                            .receipt
                            .current
                            .as_ref()
                            .is_some_and(|current| same(current, &record.original))
                        && previous.receipt.fingerprint == record.receipt.fingerprint
                        && previous
                            .receipt
                            .connections
                            .iter()
                            .all(|connection| record.receipt.connections.contains(connection)),
                    "The earlier move does not prove this original message's destination."
                );
            }
            tx.commit()?;
            Ok(record)
        })
        .await
    }
    /// User-reviewed resolution keeps a local original. It never declares what
    /// happened on the server and cannot reuse the old provider identity.
    pub async fn keep_mail_move(
        &self,
        expected: MoveRecord,
        confirmed: bool,
    ) -> anyhow::Result<MoveRecord> {
        self.run(move |c| {
            anyhow::ensure!(
                confirmed,
                "Confirm keeping a local copy without changing server copies."
            );
            let tx = c.transaction()?;
            anyhow::ensure!(!expected.finished(), "This move was already resolved.");
            let next = keep_local(&tx, expected)?;
            tx.commit()?;
            Ok(next)
        })
        .await
    }
    pub async fn mail_move_lookups(&self, now: i64) -> anyhow::Result<Vec<MoveRecord>> {
        self.run(move |c| {
            c.prepare("SELECT data FROM mail_moves WHERE stage IN ('started','committed') AND COALESCE(json_extract(data,'$.attempted'),0)<=? ORDER BY COALESCE(json_extract(data,'$.attempted'),0),token LIMIT 3")?
                .query_map([now.saturating_sub(60)],|r|r.get::<_,String>(0))?
                .map(|r|Ok(serde_json::from_str(&r?)?)).collect()
        }).await
    }
    pub async fn begin_mail_move_lookup(
        &self,
        expected: MoveRecord,
        now: i64,
    ) -> anyhow::Result<MoveRecord> {
        self.run(move |c| {
            let tx = c.transaction()?;
            anyhow::ensure!(
                matches!(expected.stage, MoveStage::Started | MoveStage::Committed),
                "Only pending or confirmed moves can be checked automatically."
            );
            let mut next = expected.clone();
            next.attempted = now;
            replace(&tx, &expected, &next)?;
            tx.commit()?;
            Ok(next)
        })
        .await
    }
    pub async fn commit_mail_move_cache(&self, expected: MoveRecord) -> anyhow::Result<MoveRecord> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let current = expected
                .receipt
                .current
                .clone()
                .context("The moved message needs a destination lookup.")?;
            let next = located(&tx, expected, current)?;
            tx.commit()?;
            Ok(next)
        })
        .await
    }
    pub async fn resolve_mail_move(
        &self,
        expected: MoveRecord,
        resolved: StoredMail,
    ) -> anyhow::Result<MoveRecord> {
        self.run(move |c| {
            let tx = c.transaction()?;
            anyhow::ensure!(
                expected
                    .receipt
                    .fingerprint
                    .as_ref()
                    .is_some_and(|f| f.matches(&resolved.raw)),
                "The located message has different content. The original was kept."
            );
            let same: bool = tx.query_row(
                "SELECT raw=? FROM messages WHERE id=?",
                params![resolved.raw, expected.original.id],
                |r| r.get(0),
            )?;
            anyhow::ensure!(
                same,
                "The cached original changed. The recovered copy was not substituted."
            );
            let next = located(&tx, expected, resolved.summary)?;
            tx.commit()?;
            Ok(next)
        })
        .await
    }
    pub async fn prepare_mail_move(&self, record: MoveRecord) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            prepare(&tx, &record)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    /// Migrate the old cross-account tuple atomically, retaining its confirmed
    /// copy or unconfirmed upload. Never turn either into a new APPEND request.
    pub async fn adopt_legacy_mail_move(&self, record: MoveRecord) -> anyhow::Result<MoveRecord> {
        self.run(move |c| {
            let tx=c.transaction()?;
            let key=format!("transfer:{}",record.original.id);
            let saved: Option<(String,String,String)>=get(&tx,&key)?;
            let (account,folder,stage)=saved.context("The legacy transfer changed. Refresh recovery.")?;
            anyhow::ensure!(account==record.receipt.account && folder==record.receipt.folder
                && account!=record.original.account_id && matches!(stage.as_str(),"copied"|"uploading"),
                "A transfer is pending for a different destination. Review the original transfer.");
            prepare(&tx,&record)?;
            let mut next=record.clone();
            next.stage=if stage=="copied" {MoveStage::Copied} else {MoveStage::Started};
            next.error=Some(if stage=="copied" {"An earlier transfer saved its copy without a destination UID. Retry recovery to find the copy before cleaning up the source."} else {"The earlier upload result is unconfirmed. Review both folders before resolving it."}.into());
            replace(&tx,&record,&next)?;
            put(&tx,&key,&Option::<(String,String,String)>::None)?;
            tx.commit()?;
            Ok(next)
        }).await
    }
    pub async fn mail_move(&self, token: String) -> anyhow::Result<MoveRecord> {
        self.run(move |c| read(c, &token)).await
    }
    pub async fn mail_move_for_source(&self, id: String) -> anyhow::Result<Option<MoveRecord>> {
        self.run(move |c| {
            c.query_row("SELECT data FROM mail_moves WHERE source_id=?", [id], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(|s| serde_json::from_str(&s).map_err(Into::into))
            .transpose()
        })
        .await
    }
    pub async fn checkpoint_mail_move(
        &self,
        expected: MoveRecord,
        stage: MoveStage,
        receipt: MoveReceipt,
    ) -> anyhow::Result<MoveRecord> {
        self.run(move |c| {
            let tx = c.transaction()?;
            anyhow::ensure!(
                expected.stage.allows(stage) && stage != MoveStage::Located,
                "This move cannot enter the requested recovery phase."
            );
            expected.validate_receipt(&receipt)?;
            anyhow::ensure!(
                expected.receipt.current.is_none()
                    || serde_json::to_string(&expected.receipt.current)?
                        == serde_json::to_string(&receipt.current)?,
                "An acknowledged destination identity cannot be replaced or forgotten."
            );
            let mut next = expected.clone();
            next.stage = stage;
            next.receipt = receipt;
            next.error = None;
            replace(&tx, &expected, &next)?;
            tx.commit()?;
            Ok(next)
        })
        .await
    }
    pub async fn fail_mail_move(
        &self,
        expected: MoveRecord,
        error: String,
    ) -> anyhow::Result<MoveRecord> {
        self.run(move |c| {
            let tx = c.transaction()?;
            anyhow::ensure!(!expected.finished(), "This move is already recovered.");
            let mut next = expected.clone();
            next.error = Some(error);
            replace(&tx, &expected, &next)?;
            tx.commit()?;
            Ok(next)
        })
        .await
    }
    /// Only a definite rejection of the first command permits releasing a new
    /// record. An APPEND acknowledgment must survive subsequent cleanup failure.
    pub async fn reject_mail_move(&self, expected: MoveRecord) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            anyhow::ensure!(
                expected.stage == MoveStage::Started,
                "A committed copy cannot be classified as rejected."
            );
            let n = tx.execute(
                "DELETE FROM mail_moves WHERE token=? AND data=?",
                params![expected.token, serde_json::to_string(&expected)?],
            )?;
            anyhow::ensure!(
                n == 1,
                "The move recovery changed. Refresh before trying again."
            );
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn pending_mail_moves(
        &self,
        account: Option<String>,
        after: Option<String>,
    ) -> anyhow::Result<Vec<MoveRecord>> {
        self.run(move |c| {
            c.prepare("SELECT data FROM mail_moves WHERE cache_id IS NOT NULL AND (?1 IS NULL OR source_account=?1 OR destination_account=?1) AND (?2 IS NULL OR token>?2) ORDER BY token LIMIT 50")?
                .query_map(params![account,after],|r|r.get::<_,String>(0))?
                .map(|r|Ok(serde_json::from_str(&r?)?)).collect()
        }).await
    }
}
