//! Persistent arrival identity, independent of unread counts or the open page.
//! Initial imports and UIDVALIDITY resets are quiet until their Inbox is complete.
use super::*;
use crate::notifications::Arrival;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS notification_mailboxes (
            account TEXT PRIMARY KEY, epoch TEXT NOT NULL, ready INTEGER NOT NULL);
         CREATE TABLE IF NOT EXISTS notification_seen (
            account TEXT NOT NULL, identity TEXT NOT NULL,
            PRIMARY KEY(account,identity));
         CREATE INDEX IF NOT EXISTS conversation_logical_identity
            ON conversation_members(account,logical_id);",
    )?;
    Ok(())
}

fn identity(raw: &[u8]) -> (String, Option<String>) {
    let headers = mailparse::parse_headers(&raw[..raw.len().min(64 * 1024)])
        .map(|(headers, _)| headers)
        .unwrap_or_default();
    let message_id = headers
        .iter()
        .filter(|header| header.get_key_ref().eq_ignore_ascii_case("Message-ID"))
        .find_map(|header| {
            crate::compose::message_ids(&header.get_value())
                .into_iter()
                .next()
        });
    if let Some(message_id) = message_id {
        // Same key as conversation_members.logical_id, without interpreting
        // References/In-Reply-To as proof that their messages were received.
        let logical = format!("{:x}", Sha256::digest(message_id.as_bytes()));
        (format!("message:{logical}"), Some(logical))
    } else {
        (format!("raw:{:x}", Sha256::digest(raw)), None)
    }
}

pub(super) fn remember(c: &Connection, mail: &StoredMail) -> anyhow::Result<()> {
    let (identity, _) = identity(&mail.raw);
    c.execute(
        "INSERT OR IGNORE INTO notification_seen(account,identity) VALUES(?,?)",
        params![mail.summary.account_id, identity],
    )?;
    Ok(())
}

pub(super) fn remove_account(c: &Connection, account: &str) -> anyhow::Result<()> {
    c.execute("DELETE FROM notification_seen WHERE account=?", [account])?;
    c.execute(
        "DELETE FROM notification_mailboxes WHERE account=?",
        [account],
    )?;
    Ok(())
}

impl Store {
    /// Called before any Inbox body is delivered. A new server epoch must not
    /// turn a redownload of the entire mailbox into new-mail notifications.
    pub async fn begin_notification_sync(
        &self,
        account: String,
        epoch: String,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            connections::allow(c, ConnectionKind::Account, &account)?;
            c.execute(
                "INSERT INTO notification_mailboxes(account,epoch,ready) VALUES(?,?,0)
                 ON CONFLICT(account) DO UPDATE SET
                    ready=CASE WHEN epoch=excluded.epoch THEN ready ELSE 0 END,
                    epoch=excluded.epoch",
                params![account, epoch],
            )?;
            Ok(())
        })
        .await
    }

    /// A failed/partial first import stays quiet across restart. Completing an
    /// obsolete epoch cannot arm notifications for its replacement.
    pub async fn finish_notification_sync(
        &self,
        account: String,
        epoch: String,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            connections::allow(c, ConnectionKind::Account, &account)?;
            c.execute(
                "UPDATE notification_mailboxes SET ready=1 WHERE account=? AND epoch=?",
                params![account, epoch],
            )?;
            Ok(())
        })
        .await
    }

    /// Atomically persist the mail and claim its arrival. Ordinary imports use
    /// upsert(), which remembers identity but never produces a notification.
    /// Claims are at most once: restarting never replays an old sound/popup.
    pub async fn sync_message(&self, mail: StoredMail) -> anyhow::Result<Option<Arrival>> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let m = &mail.summary;
            let ready: bool = tx.query_row(
                "SELECT ready FROM notification_mailboxes WHERE account=?",
                [&m.account_id], |r| r.get(0),
            ).optional()?.unwrap_or(false);
            let candidate = ready && m.unread && m.folder.eq_ignore_ascii_case("INBOX");
            let mut new = false;
            if candidate {
                let (key, logical) = identity(&mail.raw);
                let known: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1) OR
                        EXISTS(SELECT 1 FROM notification_seen WHERE account=?2 AND identity=?3) OR
                        EXISTS(SELECT 1 FROM conversation_members WHERE account=?2 AND logical_id=?4)",
                    params![m.id,m.account_id,key,logical], |r| r.get(0),
                )?;
                // Legacy messages without Message-ID have no stable logical
                // header identity. Match exact bytes among the same timestamp
                // using mail_time, rather than conflating equal subjects.
                let legacy_copy: bool = if !known && logical.is_none() {
                    tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM messages WHERE timestamp=? AND account=? AND raw=?)",
                        params![m.timestamp,m.account_id,mail.raw],|r|r.get(0),
                    )?
                } else { false };
                new = !known && !legacy_copy;
            }
            upsert_message(&tx, &mail)?;
            let arrival = new.then(|| Arrival::from_mail(m));
            tx.commit()?;
            Ok(arrival)
        }).await
    }
}
