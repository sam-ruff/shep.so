use super::*;
use crate::mail_actions::Fingerprint;
use rusqlite::OptionalExtension;

impl Store {
    pub async fn mail_metadata(&self, id: String) -> anyhow::Result<Mail> {
        self.run(move |c| {
            let (data, unread, starred, folder): (String, bool, bool, String) = c.query_row(
                "SELECT data,unread,starred,folder FROM messages WHERE id=?",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            let mut mail: Mail = serde_json::from_str(&data)?;
            mail.unread = unread;
            mail.starred = starred;
            mail.folder = folder;
            Ok(mail)
        })
        .await
    }

    pub async fn message_fingerprint(&self, id: String) -> anyhow::Result<Fingerprint> {
        self.run(move |c| {
            let raw: Vec<u8> =
                c.query_row("SELECT raw FROM messages WHERE id=?", [id], |r| r.get(0))?;
            Ok(Fingerprint::of(&raw))
        })
        .await
    }

    /// Rekey only after a provider acknowledgment. Copy raw MIME inside SQLite,
    /// then retire the source and its indexes in the same transaction.
    pub async fn relocate_mail(&self, source: Mail, destination: Mail) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            relocate(&tx, &source, &destination)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
}

pub(super) fn relocate(c: &Connection, source: &Mail, destination: &Mail) -> anyhow::Result<()> {
    connections::allow(c, ConnectionKind::Account, &destination.account_id)?;
    folder_actions::idle(c, &source.account_id)?;
    folder_actions::idle(c, &destination.account_id)?;
    let present: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE id=? AND account=? AND folder=?)",
        params![source.id, source.account_id, source.folder],
        |r| r.get(0),
    )?;
    anyhow::ensure!(present, "The cached source changed. Refresh its folders.");
    if source.id == destination.id {
        anyhow::ensure!(
            source.account_id == destination.account_id
                && source.remote_id == destination.remote_id,
            "A local identity cannot change accounts."
        );
        c.execute(
            "UPDATE messages SET folder=?,data=? WHERE id=?",
            params![
                destination.folder,
                serde_json::to_string(&destination)?,
                source.id
            ],
        )?;
    } else {
        let collision: Option<bool> = c.query_row("SELECT raw=(SELECT raw FROM messages WHERE id=?1) AND account=?2 AND folder=?3 FROM messages WHERE id=?4", params![source.id, destination.account_id, destination.folder, destination.id], |r| r.get(0)).optional()?;
        anyhow::ensure!(
            collision != Some(false),
            "The destination identity contains a different message. Refresh its folders."
        );
        if collision.is_none() {
            c.execute("INSERT INTO messages(id,account,folder,sender,subject,body,timestamp,unread,starred,data,raw)
                        SELECT ?1,?2,?3,sender,subject,body,timestamp,unread,starred,?4,raw FROM messages WHERE id=?5",
                        params![destination.id, destination.account_id, destination.folder, serde_json::to_string(&destination)?, source.id])?;
        }
        c.execute("INSERT OR IGNORE INTO restored_messages(id) SELECT ?1 FROM restored_messages WHERE id=?2", params![destination.id, source.id])?;
        conversations::index_message(c, &destination.id)?;
        outgoing::reconcile(c, destination)?;
        c.execute("DELETE FROM messages WHERE id=?", [&source.id])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_actions::MoveReceipt;
    #[tokio::test]
    async fn acknowledged_relocation_preserves_content_search_flags_and_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mail.sqlite");
        let store = Store::open(&path).unwrap();
        let raw = b"Message-ID: <move@example.test>\r\nSubject: Keep this\r\nFrom: friend@example.test\r\n\r\nSearchable original body".to_vec();
        let message = parse_mail("work", "42.7", "INBOX", raw.clone(), false, true).unwrap();
        let source = message.summary.clone();
        store.upsert(vec![message]).await.unwrap();
        let fingerprint = store.message_fingerprint(source.id.clone()).await.unwrap();
        assert!(fingerprint.matches(&raw));
        assert!(!fingerprint.matches(b"different"));
        let moved = MoveReceipt::server(
            &source,
            "personal",
            "Keep",
            Some("91.38".into()),
            fingerprint,
        )
        .current
        .unwrap();
        store
            .relocate_mail(source.clone(), moved.clone())
            .await
            .unwrap();
        assert!(store.detail(source.id).await.is_err());
        let reopened = Store::open(&path).unwrap();
        let detail = reopened.detail(moved.id.clone()).await.unwrap();
        assert_eq!(detail.summary.folder, "Keep");
        assert_eq!(detail.summary.account_id, "personal");
        assert!(!detail.summary.unread);
        assert!(detail.summary.starred);
        assert_eq!(reopened.raw_message(moved.id.clone()).await.unwrap(), raw);
        let found = reopened
            .query(MailQuery {
                folder: "Keep".into(),
                search: "Searchable".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(found.total, 1);
        assert_eq!(found.rows[0].id, moved.id);
        let restored = MoveReceipt::server(
            &moved,
            "work",
            "INBOX",
            Some("42.99".into()),
            Fingerprint::of(&raw),
        )
        .current
        .unwrap();
        reopened
            .relocate_mail(moved.clone(), restored.clone())
            .await
            .unwrap();
        assert_eq!(
            reopened
                .mail_metadata(restored.id.clone())
                .await
                .unwrap()
                .remote_id,
            "42.99"
        );
        assert!(reopened.detail(moved.id).await.is_err());
    }
    #[tokio::test]
    async fn destination_collision_rolls_back_without_losing_either_message() {
        let store = Store::memory().unwrap();
        let source = parse_mail(
            "work",
            "42.7",
            "INBOX",
            b"Subject: Original\r\n\r\nKeep me".to_vec(),
            true,
            false,
        )
        .unwrap();
        let target = parse_mail(
            "work",
            "91.38",
            "Archive",
            b"Subject: Different\r\n\r\nKeep me too".to_vec(),
            true,
            false,
        )
        .unwrap();
        let (original, existing) = (source.summary.clone(), target.summary.clone());
        store.upsert(vec![source, target]).await.unwrap();
        assert!(
            store
                .relocate_mail(original.clone(), existing.clone())
                .await
                .is_err()
        );
        assert_eq!(
            store.detail(original.id).await.unwrap().summary.subject,
            "Original"
        );
        assert_eq!(
            store.detail(existing.id).await.unwrap().summary.subject,
            "Different"
        );
    }
}
