use super::*;
use crate::notifications::Arrival;
use rusqlite::OptionalExtension;
use shep_mail_core::providers::mail::staging::Message;

impl Store {
    pub async fn sync_staged_message(&self, mail: Message) -> anyhow::Result<Option<Arrival>> {
        self.sync_staged_message_since(mail, None).await
    }
    /// As `sync_staged_message`, dropping a body the user moved away after
    /// the check that fetched it began.
    pub async fn sync_staged_message_since(
        &self,
        mut mail: Message,
        epoch: Option<SyncEpoch>,
    ) -> anyhow::Result<Option<Arrival>> {
        anyhow::ensure!(
            self.connection_key().is_none(),
            "Encrypted profiles require encrypted large-message staging."
        );
        self.run(move |c| {
            let tx = c.transaction()?;
            if arrival_moved_away(&tx, &mail.summary.id, epoch)? {
                return Ok(None);
            }
            connections::allow(&tx, ConnectionKind::Account, &mail.summary.account_id)?;
            folder_actions::idle(&tx, &mail.summary.account_id)?;
            let (mut identity, logical) = notifications::identity(&mail.header_prefix);
            if logical.is_none() { identity = format!("raw:{}", mail.raw_hash); }
            let m = &mail.summary;
            let ready: bool = tx.query_row("SELECT ready FROM notification_mailboxes WHERE account=?", [&m.account_id], |r| r.get(0)).optional()?.unwrap_or(false);
            let known: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?1) OR EXISTS(SELECT 1 FROM notification_seen WHERE account=?2 AND identity=?3) OR EXISTS(SELECT 1 FROM conversation_members WHERE account=?2 AND logical_id=?4)",
                params![m.id,m.account_id,identity,logical], |r| r.get(0))?;
            let bytes = i32::try_from(mail.bytes()).context("This message exceeds SQLite's supported blob size.")?;
            let inserted = tx.execute("INSERT INTO messages(id,account,folder,sender,subject,body,timestamp,unread,starred,data,raw)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,zeroblob(?11)) ON CONFLICT(id) DO NOTHING",
                params![m.id,m.account_id,m.folder,m.sender,m.subject,mail.text,m.timestamp,m.unread,m.starred,serde_json::to_string(m)?,bytes])?;
            let candidate = inserted == 1 && !known && ready && m.unread && m.folder.eq_ignore_ascii_case("INBOX");
            if inserted == 1 {
                let row: i64 = tx.query_row("SELECT rowid FROM messages WHERE id=?", [&m.id], |r| r.get(0))?;
                let mut blob = tx.blob_open("main", "messages", "raw", row, false)?;
                mail.copy_to(&mut blob)?;
                blob.close()?;
            }
            let m = &mail.summary;
            tx.execute("UPDATE messages SET unread=?,starred=? WHERE id=?", params![m.unread,m.starred,m.id])?;
            conversations::index_message(&tx, &m.id)?;
            outgoing::reconcile(&tx, m)?;
            tx.execute("INSERT OR IGNORE INTO notification_seen(account,identity) VALUES(?,?)", params![m.account_id,identity])?;
            let arrival = candidate.then(|| Arrival::from_mail(m));
            tx.commit()?;
            Ok(arrival)
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn staged_large_attachment_mail_opens_with_body_and_exact_attachment()
    -> anyhow::Result<()> {
        let store = Store::open(":memory:")?;
        let mut source = tempfile::NamedTempFile::new()?;
        source.write_all(b"From: sender@example.test\r\nSubject: Large attachment\r\nContent-Type: multipart/mixed; boundary=part\r\n\r\n--part\r\nContent-Type: text/html\r\n\r\n<html><body><p>Readable large mail.</p></body></html>\r\n--part\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=big.bin\r\n\r\n")?;
        let block = [b'x'; 8192];
        for _ in 0..26 * 1024 * 1024 / block.len() {
            source.write_all(&block)?;
        }
        source.write_all(b"\r\n--part--\r\n")?;
        let prepared = shep_mail_core::providers::mail::staging::prepare(
            source, "fixture", "42.8", "INBOX", true, false,
        )?;
        let id = prepared.summary.id.clone();
        store.sync_staged_message(prepared).await?;
        let detail = store.detail(id).await?;
        assert_eq!(detail.summary.subject, "Large attachment");
        assert!(detail.body.contains("Readable large mail."));
        assert!(detail.html.is_some());
        assert_eq!(detail.attachments.len(), 1);
        assert_eq!(detail.attachments[0].name, "big.bin");
        assert_eq!(detail.attachments[0].bytes.len(), 26 * 1024 * 1024);
        assert!(detail.attachments[0].bytes.iter().all(|byte| *byte == b'x'));
        Ok(())
    }

    #[tokio::test]
    async fn staged_large_mail_commits_exact_raw() -> anyhow::Result<()> {
        let store = Store::open(":memory:")?;
        let mut source = tempfile::NamedTempFile::new()?;
        source.write_all(b"Subject: Large source\r\n\r\n")?;
        let block = [b'x'; 8192];
        for _ in 0..26 * 1024 * 1024 / block.len() {
            source.write_all(&block)?;
        }
        let prepared = shep_mail_core::providers::mail::staging::prepare(
            source, "fixture", "42.7", "INBOX", true, false,
        )?;
        let bytes = prepared.bytes();
        let id = prepared.summary.id.clone();
        store.sync_staged_message(prepared).await?;
        let first = store.detail(id.clone()).await?;
        assert_eq!(first.body.len(), READER_BODY_PAGE);
        assert!(first.body_truncated);
        let next = store
            .detail_limited(id.clone(), READER_BODY_PAGE * 2)
            .await?;
        assert_eq!(next.body.len(), READER_BODY_PAGE * 2);
        assert!(next.body_truncated);
        store
            .run(move |c| {
                let (size, prefix): (i64, String) = c.query_row(
                    "SELECT length(raw),cast(substr(raw,1,23) AS TEXT) FROM messages WHERE id=?",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?;
                assert_eq!(size as u64, bytes);
                assert_eq!(prefix, "Subject: Large source\r\n");
                Ok(())
            })
            .await?;
        Ok(())
    }
}
