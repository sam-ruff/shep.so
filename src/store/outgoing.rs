use super::*;
use crate::outgoing::*;
use rusqlite::OptionalExtension;

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS outgoing (
        draft TEXT PRIMARY KEY, attempt TEXT NOT NULL UNIQUE, account TEXT NOT NULL,
        stage TEXT NOT NULL, created INTEGER NOT NULL, data TEXT NOT NULL, logical_id TEXT NOT NULL,
        config TEXT, envelope TEXT, raw BLOB);
        CREATE INDEX IF NOT EXISTS outgoing_status ON outgoing(stage,created);
        CREATE TABLE IF NOT EXISTS sent_folders(account TEXT PRIMARY KEY,folder TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS sent_folder_lookup ON sent_folders(folder,account);
        CREATE INDEX IF NOT EXISTS outgoing_account ON outgoing(account);
        CREATE INDEX IF NOT EXISTS outgoing_message ON outgoing(account,logical_id);
        CREATE INDEX IF NOT EXISTS conversation_logical_message ON conversation_members(account,logical_id);",
    )?;
    Ok(())
}
pub(super) fn changed(c: &Connection) -> anyhow::Result<()> {
    let current: u64 = get(c, "outgoing_revision")?;
    put(
        c,
        "outgoing_revision",
        &current
            .checked_add(1)
            .context("Outgoing revision overflow")?,
    )
}
fn info(c: &Connection, attempt: &str) -> anyhow::Result<OutgoingInfo> {
    let data: String = c
        .query_row(
            "SELECT data FROM outgoing WHERE attempt=?",
            [attempt],
            |r| r.get(0),
        )
        .context("This outgoing message changed. Reopen Outbox to review it.")?;
    Ok(serde_json::from_str(&data)?)
}
fn write(c: &Connection, info: &OutgoingInfo) -> anyhow::Result<()> {
    anyhow::ensure!(
        c.execute(
            "UPDATE outgoing SET stage=?,data=? WHERE attempt=?",
            params![
                format!("{:?}", info.delivery),
                serde_json::to_string(info)?,
                info.attempt
            ]
        )? == 1,
        "This outgoing attempt is no longer current."
    );
    changed(c)
}
pub(super) fn pending(c: &Connection) -> anyhow::Result<usize> {
    Ok(c.query_row(
        "SELECT COUNT(*) FROM outgoing WHERE stage NOT IN ('Complete','Released')",
        [],
        |r| r.get::<_, i64>(0),
    )? as usize)
}
impl Store {
    pub async fn outgoing_page(&self, offset: usize) -> anyhow::Result<OutgoingPage> {
        self.run(move |c| {
            let rows=c.prepare("SELECT data FROM outgoing WHERE stage NOT IN ('Complete','Released') ORDER BY created DESC,attempt LIMIT ? OFFSET ?")?
                .query_map(params![OUTGOING_PAGE_SIZE as i64, i64::try_from(offset)?],|r|r.get::<_,String>(0))?.map(|r|Ok(serde_json::from_str(&r?)?)).collect::<anyhow::Result<Vec<_>>>()?;
            Ok(OutgoingPage {revision:get(c,"outgoing_revision")?,offset,total:pending(c)?,rows})
        }).await
    }
    pub async fn outgoing_for_draft(&self, draft: String) -> anyhow::Result<Option<OutgoingInfo>> {
        self.run(move |c| {
            c.query_row("SELECT data FROM outgoing WHERE draft=?", [draft], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(|data| Ok(serde_json::from_str(&data)?))
            .transpose()
        })
        .await
    }
    pub async fn outgoing_info(&self, attempt: String) -> anyhow::Result<OutgoingInfo> {
        self.run(move |c| info(c, &attempt)).await
    }
    pub async fn outgoing_submission(&self, attempt: String) -> anyhow::Result<Submission> {
        self.run(move |c| {
            let info = info(c, &attempt)?;
            let (config, envelope, raw): (String, String, Vec<u8>) = c.query_row(
                "SELECT config,envelope,raw FROM outgoing WHERE attempt=?",
                [attempt],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            anyhow::ensure!(
                raw.len() <= MAX_MESSAGE_BYTES,
                "The saved outgoing message exceeds the size limit."
            );
            Ok(Submission {
                info,
                account: serde_json::from_str(&config)?,
                envelope: serde_json::from_str(&envelope)?,
                raw,
            })
        })
        .await
    }
    /// This commit must complete BEFORE calling SMTP. A surviving Submitting
    /// record always requires review; it is never an automatic retry instruction.
    pub async fn begin_outgoing(
        &self,
        submission: Submission,
        draft: Draft,
    ) -> anyhow::Result<OutgoingInfo> {
        self.run(move |c| {
            let tx=c.transaction()?;
            connections::allow(&tx,ConnectionKind::Account,&draft.account_id)?;
            anyhow::ensure!(!drafts::sent(&tx,&draft)?,"This draft version was already sent.");
            let latest=drafts::snapshot(&tx)?.drafts.into_iter().find(|d|d.id==draft.id).context("Save this draft before sending.")?;
            anyhow::ensure!(serde_json::to_string(&latest)?==serde_json::to_string(&draft)? && latest.attachments==draft.attachments,"The draft changed before sending. Review its latest text and attachments.");
            let i=&submission.info;
            anyhow::ensure!(i.account_id==draft.account_id && i.draft_id==draft.id && i.draft_revision==draft.revision && submission.account.id==draft.account_id && i.delivery==DeliveryState::Submitting && submission.raw.len()<=MAX_MESSAGE_BYTES,"Invalid outgoing attempt.");
            let previous:Option<String>=tx.query_row("SELECT data FROM outgoing WHERE draft=?",[&draft.id],|r|r.get(0)).optional()?;
            if let Some(previous)=previous { let previous:OutgoingInfo=serde_json::from_str(&previous)?;
                anyhow::ensure!(matches!(previous.delivery,DeliveryState::Rejected|DeliveryState::Released),"This draft already has a delivery record. Review it in Outbox before sending again.");
            }
            tx.execute("INSERT INTO outgoing(draft,attempt,account,stage,created,data,logical_id,config,envelope,raw) VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(draft) DO UPDATE SET attempt=excluded.attempt,account=excluded.account,stage=excluded.stage,created=excluded.created,data=excluded.data,logical_id=excluded.logical_id,config=excluded.config,envelope=excluded.envelope,raw=excluded.raw",params![draft.id,i.attempt,i.account_id,"Submitting",i.created,serde_json::to_string(i)?,logical_id(&i.message_id),serde_json::to_string(&submission.account)?,serde_json::to_string(&submission.envelope)?,submission.raw])?;
            changed(&tx)?; tx.commit()?; Ok(submission.info)
        }).await
    }
    pub async fn record_delivery(
        &self,
        attempt: String,
        state: DeliveryState,
        error: Option<String>,
    ) -> anyhow::Result<OutgoingInfo> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut info = info(&tx, &attempt)?;
            anyhow::ensure!(
                matches!(
                    state,
                    DeliveryState::Uncertain | DeliveryState::Rejected | DeliveryState::Accepted
                ),
                "Invalid delivery outcome."
            );
            anyhow::ensure!(
                matches!(
                    info.delivery,
                    DeliveryState::Submitting | DeliveryState::Uncertain
                ) || info.delivery == state,
                "This attempt already has a different delivery outcome."
            );
            info.delivery = state;
            info.error = error;
            write(&tx, &info)?;
            tx.commit()?;
            Ok(info)
        })
        .await
    }
    pub async fn release_outgoing(&self, attempt: String) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut info = info(&tx, &attempt)?;
            anyhow::ensure!(
                matches!(
                    info.delivery,
                    DeliveryState::Submitting | DeliveryState::Uncertain | DeliveryState::Rejected
                ),
                "A delivered message cannot be returned to its original draft."
            );
            info.delivery = DeliveryState::Released;
            info.error = None;
            write(&tx, &info)?;
            tx.execute(
                "UPDATE outgoing SET config=NULL,envelope=NULL,raw=NULL WHERE attempt=?",
                [attempt],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn outgoing_local_sent(
        &self,
        attempt: String,
        mail: StoredMail,
    ) -> anyhow::Result<drafts::DraftState> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let info = info(&tx, &attempt)?;
            anyhow::ensure!(
                info.delivery == DeliveryState::Accepted && mail.summary.id == info.local_id(),
                "This message has not been confirmed as sent."
            );
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?)",
                [&mail.summary.id],
                |r| r.get(0),
            )?;
            if !exists {
                upsert_message(&tx, &mail)?;
            }
            drafts::finish(
                &tx,
                Draft {
                    id: info.draft_id,
                    revision: info.draft_revision,
                    account_id: info.account_id,
                    ..Default::default()
                },
            )?;
            let state = drafts::snapshot(&tx)?;
            tx.commit()?;
            Ok(state)
        })
        .await
    }
    pub async fn record_sent_copy(
        &self,
        attempt: String,
        state: SentState,
        folder: Option<String>,
        error: Option<String>,
    ) -> anyhow::Result<OutgoingInfo> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut info = info(&tx, &attempt)?;
            anyhow::ensure!(
                info.delivery == DeliveryState::Accepted,
                "Sent-copy recovery cannot submit mail."
            );
            // An ambiguous APPEND cannot become a new APPEND without an explicit
            // reviewed reset to Pending by the recovery command.
            anyhow::ensure!(
                state != SentState::Appending || info.sent == SentState::Pending,
                "Review the unfinished Sent copy before uploading again."
            );
            info.sent = state;
            if let Some(folder)=&folder {tx.execute("INSERT INTO sent_folders(account,folder) VALUES(?,?) ON CONFLICT(account) DO UPDATE SET folder=excluded.folder",params![info.account_id,folder])?;}
            info.folder = folder;
            info.error = error;
            if matches!(state, SentState::Saved | SentState::LocalOnly) {
                let exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?)",
                    [info.local_id()],
                    |r| r.get(0),
                )?;
                anyhow::ensure!(
                    exists,
                    "Save the local Sent copy before finishing recovery."
                );
                info.delivery = DeliveryState::Complete;
                tx.execute(
                    "UPDATE outgoing SET config=NULL,envelope=NULL,raw=NULL WHERE attempt=?",
                    [&attempt],
                )?;
            }
            write(&tx, &info)?;
            if info.delivery == DeliveryState::Complete {
                reconcile_saved(&tx, &info)?;
            }
            tx.commit()?;
            Ok(info)
        })
        .await
    }
}

fn logical_id(message_id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(message_id.as_bytes()))
}
fn reconcile_saved(c: &Connection, info: &OutgoingInfo) -> anyhow::Result<()> {
    if info.sent != SentState::Saved {
        return Ok(());
    }
    if let Some(folder) = &info.folder {
        let copied:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM conversation_members cm JOIN messages m ON m.id=cm.id WHERE cm.account=? AND cm.logical_id=? AND m.folder=? AND m.id!=?)",params![info.account_id,logical_id(&info.message_id),folder,info.local_id()],|r|r.get(0))?;
        if copied {
            c.execute(
                "DELETE FROM messages WHERE id=? AND folder='Sent'",
                [info.local_id()],
            )?;
        }
    }
    Ok(())
}
pub(super) fn reconcile(c: &Connection, message: &Mail) -> anyhow::Result<()> {
    if message.remote_id.starts_with("local-sent-") {
        return Ok(());
    }
    let logical: Option<String> = c
        .query_row(
            "SELECT logical_id FROM conversation_members WHERE id=?",
            [&message.id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(logical) = logical {
        let deliveries = c
            .prepare(
                "SELECT data FROM outgoing WHERE account=? AND logical_id=? AND stage='Complete'",
            )?
            .query_map(params![message.account_id, logical], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for data in deliveries {
            reconcile_saved(c, &serde_json::from_str(&data)?)?;
        }
    }
    Ok(())
}
