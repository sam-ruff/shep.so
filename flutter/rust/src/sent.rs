//! Durable Sent-copy work is separate from SMTP delivery. Never repeat an
//! unacknowledged APPEND without a new, explicitly reviewed copy decision.
use crate::{api::MobileProfile, operations, outgoing};
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use secrecy::SecretString;
use serde_json::{Value, json};
use shep_mail_core::{
    model::*,
    providers::mail::sent::{self, SentConnection, SentMailbox, SentReceipt},
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Mutex as AsyncMutex;

pub(crate) type Open = Pin<Box<dyn Future<Output = Result<Box<dyn SentConnection>>> + Send>>;
pub(crate) trait Factory: Send + Sync {
    fn open(&self, account: Account, password: SecretString) -> Open;
}
struct Server;
impl Factory for Server {
    fn open(&self, account: Account, password: SecretString) -> Open {
        Box::pin(async move {
            Ok(Box::new(SentMailbox::open(&account, &password).await?) as Box<dyn SentConnection>)
        })
    }
}
pub(crate) struct Runtime {
    factory: Mutex<Arc<dyn Factory>>,
    pub(crate) receipts: AsyncMutex<HashMap<String, SentReceipt>>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            factory: Mutex::new(Arc::new(Server)),
            receipts: AsyncMutex::new(HashMap::new()),
        }
    }
}
impl Runtime {
    #[cfg(test)]
    pub(crate) fn set_factory(&self, factory: Arc<dyn Factory>) {
        *self.factory.lock().unwrap() = factory;
    }
}
struct Record {
    account: Account,
    state: String,
    folder: Option<String>,
    delivery: String,
    marked: bool,
    message_id: String,
    timestamp: i64,
}
async fn record(profile: &MobileProfile, id: &str) -> Result<Record> {
    let id = id.to_owned();
    profile.database.read(move|db|{
        let (account,state,folder,delivery,recovery,message_id,timestamp):(String,String,Option<String>,String,Option<String>,String,Option<i64>)=db.query_row("SELECT s.account,s.state,s.folder,o.state,m.recovery,o.message_id,m.created FROM outgoing o JOIN outgoing_sent s ON s.id=o.id LEFT JOIN outgoing_meta m ON m.id=o.id WHERE o.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?.context("This older submission has no saved account connection. Keep its local copy and review Sent with your provider.")?;
        anyhow::ensure!(recovery.as_deref().is_none_or(|r|r=="marked"),"This outgoing message was already reviewed. Open its draft or local Sent copy.");
        Ok(Record{account:serde_json::from_str(&account)?,state,folder,delivery,marked:recovery.as_deref()==Some("marked"),message_id,timestamp:timestamp.context("The original submission date is unavailable. Keep the local copy.")?})
    }).await
}
async fn finish_local(profile: &MobileProfile, id: &str) -> Result<()> {
    let id = id.to_owned();
    profile.database.write(move|db|{
        outgoing::local_sent(db,&id,None)?;
        reconcile_receipt(db,&id)?;
        db.execute("UPDATE outgoing_sent SET complete=1,error=NULL WHERE id=?1",[id])?;
        Ok(())
    }).await.context("The Sent copy is saved, but the local cache needs repair. Check Outbox to finish; do not send or upload again.")
}
async fn save_receipt(profile: &MobileProfile, id: &str, receipt: SentReceipt) -> Result<()> {
    profile
        .operations
        .sent
        .receipts
        .lock()
        .await
        .insert(id.to_owned(), receipt.clone());
    let attempt = id.to_owned();
    profile.database.write(move|db|{
        anyhow::ensure!(db.execute("UPDATE outgoing_sent SET state='saved',folder=?2,receipt=?3,error=NULL WHERE id=?1",params![attempt,receipt.folder,serde_json::to_string(&receipt)?])?==1,"The outgoing Sent record is unavailable. Keep the local copy.");
        db.execute("INSERT OR IGNORE INTO known_sent_folders(account_id,folder) SELECT account_id,?2 FROM outgoing WHERE id=?1",params![attempt,receipt.folder])?;
        Ok(())
    }).await.context("The server saved the Sent copy, but its acknowledgment could not be stored. Check Outbox to save the result; do not upload another copy.")?;
    profile.operations.sent.receipts.lock().await.remove(id);
    finish_local(profile, id).await
}
async fn copy_inner(
    profile: &MobileProfile,
    id: &str,
    copy: bool,
    confirmed: bool,
    password: Option<SecretString>,
    automatic: bool,
) -> Result<Value> {
    let saved = record(profile, id).await?;
    let pending = profile
        .operations
        .sent
        .receipts
        .lock()
        .await
        .get(id)
        .cloned();
    if let Some(receipt) = pending {
        save_receipt(profile, id, receipt).await?;
        return Ok(json!({"sent":"saved","notice":"Sent copy saved. No message was resent."}));
    }
    anyhow::ensure!(
        saved.delivery != "submitting",
        "SMTP is still running. Check Outbox after it finishes."
    );
    anyhow::ensure!(
        matches!(saved.delivery.as_str(), "delivered" | "uncertain"),
        "This message was not sent. Return it to drafts before making a new Send decision."
    );
    if matches!(saved.state.as_str(), "saved" | "local") {
        finish_local(profile, id).await?;
        return Ok(json!({"sent":saved.state,"notice":"Sent copy saved. No message was resent."}));
    }
    let known_delivery = saved.delivery == "delivered" || saved.marked;
    anyhow::ensure!(
        !copy || known_delivery,
        "Review delivery before saving a server Sent copy. A copy upload does not send mail to recipients."
    );
    if automatic
        && (saved.account.protocol == Protocol::Pop3
            || saved.account.sent_copy == SentCopyPolicy::LocalOnly)
    {
        let id = id.to_owned();
        profile
            .database
            .write(move |db| {
                outgoing::local_sent(db, &id, None)?;
                db.execute(
                    "UPDATE outgoing_sent SET state='local',complete=1,error=NULL WHERE id=?1",
                    [id],
                )?;
                Ok(())
            })
            .await?;
        return Ok(json!({"sent":"local","notice":"Sent copy kept locally."}));
    }
    anyhow::ensure!(
        saved.account.protocol == Protocol::Imap,
        "POP3 keeps Sent copies locally. Review delivery with your provider."
    );
    let identity = saved.account.id.clone();
    let mut account = profile
        .database
        .read(move |db| operations::stored_account(db, &identity))
        .await?;
    anyhow::ensure!(
        sent::same_incoming(&account, &saved.account),
        "This account changed after sending. Restore the original incoming connection or keep the local Sent copy."
    );
    let uncertain = matches!(saved.state.as_str(), "appending" | "uncertain");
    if uncertain {
        account.sent_folder=saved.folder.clone().context("The unacknowledged Sent upload has no destination. Keep the local copy and review the provider folder.")?;
        anyhow::ensure!(
            !copy || confirmed,
            "The previous Sent upload was not acknowledged. Review the server folder and confirm before uploading another copy."
        );
    }
    anyhow::ensure!(
        !copy || account.sent_copy != SentCopyPolicy::LocalOnly,
        "This account keeps Sent locally. Change its Sent-copy preference before uploading to the server."
    );
    let password = password
        .context("Unlock device credentials or reconnect this account, then check Sent again.")?;
    let factory = profile
        .operations
        .sent
        .factory
        .lock()
        .map_err(|_| {
            anyhow::anyhow!("Sent connection setup failed. Reopen Shep and check Outbox.")
        })?
        .clone();
    let mut connection=tokio::time::timeout(Duration::from_secs(60),factory.open(account.clone(),password)).await.context("Connecting to Sent timed out. Check the connection and retry.")?.map_err(|_|anyhow::anyhow!("Could not open the server Sent folder. Check the incoming connection, credentials and Sent-folder preference."))?;
    let found=tokio::time::timeout(Duration::from_secs(60),connection.find(&saved.message_id)).await.context("Checking Sent timed out. The delivery record was kept.")?.map_err(|_|anyhow::anyhow!("The server could not complete the Sent lookup. Keep the local copy and retry checking Sent."))?;
    if let Some(receipt) = found {
        anyhow::ensure!(
            receipt.folder == connection.folder(),
            "The Sent lookup returned another folder. Keep the local copy and retry."
        );
        save_receipt(profile, id, receipt).await?;
        return Ok(
            json!({"sent":"saved","notice":"Matching message found in Sent. No message was resent."}),
        );
    }
    anyhow::ensure!(
        copy,
        if known_delivery {
            "No matching copy was found in Sent. Delivery is confirmed; saving a copy does not resend it."
        } else {
            "No matching copy was found. This does not prove the message was not sent. Review delivery before returning it to drafts."
        }
    );
    anyhow::ensure!(
        account.sent_copy != SentCopyPolicy::ServerManaged,
        "Your server is configured to save Sent automatically. Check again after syncing, or keep the local copy."
    );
    let folder = connection.folder().to_owned();
    let attempt = id.to_owned();
    let pinned_folder = folder.clone();
    let raw = profile
        .database
        .write(move |db| {
            // Commit destination and uncertain-upload protection before any bytes.
            let raw = db.query_row("SELECT raw FROM outgoing WHERE id=?1", [&attempt], |r| {
                r.get::<_, Vec<u8>>(0)
            })?;
            db.execute(
                "UPDATE outgoing_sent SET state='appending',folder=?2,error=NULL WHERE id=?1",
                params![attempt, pinned_folder],
            )?;
            Ok(raw)
        })
        .await?;
    let outcome = tokio::time::timeout(
        Duration::from_secs(60),
        connection.append(&raw, saved.timestamp),
    )
    .await;
    match outcome {
        Ok(Ok(receipt)) => {
            // APPEND OK is authoritative, including when a later cache save fails.
            anyhow::ensure!(
                receipt.folder == folder,
                "The Sent receipt returned another folder. Review Outbox before another upload."
            );
            save_receipt(profile, id, receipt).await?;
            Ok(json!({"sent":"saved","notice":"Sent copy saved. No message was resent."}))
        }
        _ => {
            let id = id.to_owned();
            let _ = profile
                .database
                .write(move |db| {
                    db.execute(
                        "UPDATE outgoing_sent SET state='uncertain' WHERE id=?1",
                        [id],
                    )?;
                    Ok(())
                })
                .await;
            anyhow::bail!(
                "The server did not acknowledge the Sent copy. Check the server folder before uploading another copy."
            )
        }
    }
}
/// Caller owns the account lock and provider admission for the whole operation.
pub(crate) async fn run_locked(
    profile: &MobileProfile,
    id: &str,
    copy: bool,
    confirmed: bool,
    password: Option<SecretString>,
    automatic: bool,
) -> Result<Value> {
    let result = copy_inner(profile, id, copy, confirmed, password, automatic).await;
    if let Err(error) = &result {
        let id = id.to_owned();
        let text = error.to_string();
        let _ = profile
            .database
            .write(move |db| {
                db.execute(
                    "UPDATE outgoing_sent SET error=?2 WHERE id=?1",
                    params![id, text],
                )?;
                Ok(())
            })
            .await;
    }
    result
}
pub(crate) async fn recover(
    profile: &MobileProfile,
    id: String,
    copy: bool,
    confirmed: bool,
    password: Option<SecretString>,
) -> Result<Value> {
    let attempt = id.clone();
    let account = profile
        .database
        .read(move |db| {
            Ok(db.query_row(
                "SELECT account_id FROM outgoing WHERE id=?1",
                [attempt],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await?;
    let _guard = profile.operations.try_account(&account).await?;
    run_locked(profile, &id, copy, confirmed, password, false).await
}

fn message_id(raw: &[u8]) -> Option<String> {
    use mailparse::MailHeaderMap;
    let (headers, _) = mailparse::parse_headers(&raw[..raw.len().min(64 * 1024)]).ok()?;
    let ids = headers.get_all_values("Message-ID");
    (ids.len() == 1).then(|| ids[0].trim().to_owned())
}
/// Adopt the provider identity without invalidating the local UI identifier.
/// Existing edits remain a separate retained local copy; aliases keep requests
/// made from a previously displayed provider row valid as well.
pub(crate) fn reconcile(db: &rusqlite::Connection, mail: &Mail, raw: &[u8]) -> Result<()> {
    if mail.remote_id.starts_with("local-") {
        return Ok(());
    }
    if db.is_autocommit() {
        let tx = db.unchecked_transaction()?;
        reconcile(&tx, mail, raw)?;
        tx.commit()?;
        return Ok(());
    }
    let Some(message_id) = message_id(raw) else {
        return Ok(());
    };
    let mut query=db.prepare("SELECT o.id,s.receipt FROM outgoing o JOIN outgoing_sent s ON s.id=o.id WHERE o.account_id=?1 AND o.message_id=?2 AND s.folder=?3 AND s.state='saved' AND s.local_edited=0 LIMIT 2")?;
    let attempts = query
        .query_map(params![mail.account_id, message_id, mail.folder], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let [(attempt, receipt)] = attempts.as_slice() else {
        return Ok(());
    };
    let receipt: SentReceipt =
        serde_json::from_str(receipt.as_deref().context(
            "The Sent acknowledgment is incomplete. Check Outbox before another upload.",
        )?)?;
    if receipt.folder != mail.folder {
        return Ok(());
    }
    if receipt
        .remote_id
        .as_ref()
        .is_some_and(|id| *id != mail.remote_id)
    {
        return Ok(());
    }
    let local = format!("{}:Sent:local-sent-{attempt}", mail.account_id);
    let eligible:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM mail WHERE id=?1 AND account_id=?2 AND folder='Sent' AND remote_id=?3 AND moved=0) AND NOT EXISTS(SELECT 1 FROM pending_moves WHERE id IN (?1,?4)) AND NOT EXISTS(SELECT 1 FROM move_receipts WHERE id IN (?1,?4))",params![local,mail.account_id,format!("local-sent-{attempt}"),mail.id],|r|r.get(0))?;
    if !eligible || local == mail.id {
        return Ok(());
    }
    let body: String = db.query_row("SELECT body FROM mail WHERE id=?1", [&mail.id], |r| {
        r.get(0)
    })?;
    // Move existing aliases before removing the duplicate, otherwise its FK
    // cascade could invalidate an older pending action. Keep every alias direct.
    db.execute(
        "UPDATE mail_aliases SET id=?2 WHERE id=?1",
        params![mail.id, local],
    )?;
    db.execute("DELETE FROM mail WHERE id=?1", [&mail.id])?;
    db.execute(
        "INSERT INTO mail_aliases(alias,id) VALUES(?1,?2)",
        params![mail.id, local],
    )?;
    db.execute("UPDATE mail SET remote_id=?2,folder=?3,sender=?4,recipient=?5,subject=?6,preview=?7,timestamp=?8,unread=?9,starred=?10,attachment_count=?11,body=?12,raw=?13 WHERE id=?1",params![local,mail.remote_id,mail.folder,mail.sender,mail.recipient,mail.subject,mail.preview,mail.timestamp,mail.unread,mail.starred,u32::try_from(mail.attachment_count)?,body,raw])?;
    Ok(())
}
fn reconcile_receipt(db: &rusqlite::Connection, id: &str) -> Result<()> {
    db.execute("INSERT OR IGNORE INTO known_sent_folders(account_id,folder) SELECT o.account_id,s.folder FROM outgoing o JOIN outgoing_sent s ON s.id=o.id WHERE o.id=?1 AND s.state='saved' AND s.folder IS NOT NULL",[id])?;
    let (account,receipt):(String,Option<String>)=db.query_row("SELECT o.account_id,s.receipt FROM outgoing o JOIN outgoing_sent s ON s.id=o.id WHERE o.id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?)))?;
    if let Some(receipt) = receipt {
        let receipt: SentReceipt = serde_json::from_str(&receipt)?;
        if let Some(remote) = receipt.remote_id {
            let cached: Option<(String, Vec<u8>)> = db
                .query_row(
                    "SELECT id,raw FROM mail WHERE account_id=?1 AND folder=?2 AND remote_id=?3",
                    params![account, receipt.folder, remote],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((id, raw)) = cached {
                reconcile(db, &operations::stored_mail(db, &id)?, &raw)?;
            }
        }
    }
    Ok(())
}
