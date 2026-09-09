use super::*;
use crate::backup::{
    BackupTarget,
    history::{Entry, Outcome, PAGE},
};
use rusqlite::OptionalExtension;

pub(crate) fn schema(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS backup_history (
        sequence INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, target TEXT NOT NULL, started INTEGER NOT NULL,
        copy TEXT, outcome TEXT NOT NULL, data TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS backup_history_target_time ON backup_history(target,started DESC,sequence DESC);
        CREATE INDEX IF NOT EXISTS backup_history_time ON backup_history(started DESC,sequence DESC);
        CREATE INDEX IF NOT EXISTS backup_history_copy ON backup_history(target,copy);")?;
    Ok(())
}
impl Store {
    pub async fn backup_history(&self, target: BackupTarget) -> anyhow::Result<Vec<Entry>> {
        self.run(move |conn| {
            let target = serde_json::to_string(&target)?;
            let rows = conn.prepare("SELECT data FROM backup_history WHERE target=?1 ORDER BY started DESC,sequence DESC LIMIT ?2")?
                .query_map(params![target, PAGE as i64], |r| r.get::<_,String>(0))?
                .collect::<Result<Vec<_>,_>>()?;
            rows.into_iter().map(|s| Ok(serde_json::from_str(&s)?)).collect()
        }).await
    }
    pub(crate) async fn write_backup_history(&self, mut entry: Entry) -> anyhow::Result<()> {
        // Bounded diagnostic text; providers already strip secrets from errors.
        entry.detail = entry.detail.chars().take(2048).collect();
        self.run(move |conn| {
            let tx = conn.transaction()?;
            let previous: Option<String> = tx.query_row("SELECT data FROM backup_history WHERE id=?1", [&entry.id], |r| r.get(0)).optional()?;
            if let Some(previous) = previous {
                let previous: Entry = serde_json::from_str(&previous)?;
                anyhow::ensure!(previous.target == entry.target && previous.started == entry.started,
                    "The backup history identity changed.");
                anyhow::ensure!(previous.copy.is_none() || previous.copy == entry.copy,
                    "The backup history copy identity changed.");
                anyhow::ensure!(!matches!(previous.outcome, Outcome::Saved | Outcome::SavedWithWarning | Outcome::Recovered)
                    || matches!(entry.outcome, Outcome::Saved | Outcome::SavedWithWarning | Outcome::Recovered),
                    "A confirmed backup receipt cannot become an unconfirmed attempt.");
            }
            let target = serde_json::to_string(&entry.target)?;
            if let Some(copy) = &entry.copy
                && matches!(entry.outcome, Outcome::Saved | Outcome::SavedWithWarning)
            {
                // A later acknowledged recovery resolves earlier uncertain attempts
                // for exactly this reserved copy, never unrelated failed uploads.
                let previous = tx.prepare("SELECT data FROM backup_history WHERE target=?1 AND copy=?2 AND id<>?3")?
                    .query_map(params![target, copy, entry.id], |r| r.get::<_,String>(0))?
                    .collect::<Result<Vec<_>,_>>()?;
                for raw in previous {
                    let mut previous: Entry = serde_json::from_str(&raw)?;
                    if matches!(previous.outcome, Outcome::Unfinished | Outcome::NeedsReview) {
                        previous.outcome = Outcome::Recovered;
                        previous.finished = entry.finished;
                        tx.execute("UPDATE backup_history SET outcome=?1,data=?2 WHERE id=?3",
                            params![serde_json::to_string(&previous.outcome)?, serde_json::to_string(&previous)?, previous.id])?;
                    }
                }
            }
            tx.execute("INSERT INTO backup_history(id,target,started,copy,outcome,data) VALUES(?1,?2,?3,?4,?5,?6)
                ON CONFLICT(id) DO UPDATE SET copy=excluded.copy,outcome=excluded.outcome,data=excluded.data",
                params![entry.id,target,entry.started,entry.copy,serde_json::to_string(&entry.outcome)?,serde_json::to_string(&entry)?])?;
            // Indexed, bounded pages; history never drives retry or ownership.
            tx.execute("DELETE FROM backup_history WHERE id IN (SELECT id FROM backup_history WHERE target=?1 ORDER BY started DESC,sequence DESC LIMIT -1 OFFSET ?2)", params![target,PAGE as i64])?;
            tx.execute("DELETE FROM backup_history WHERE id IN (SELECT id FROM backup_history ORDER BY started DESC,sequence DESC LIMIT -1 OFFSET 640)", [])?;
            tx.commit()?;
            Ok(())
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(target: &str) -> Entry {
        Entry::new(
            BackupTarget::Local(target.into()),
            target.into(),
            Default::default(),
        )
    }
    #[tokio::test]
    async fn backup_history_migration_bounds_ties_and_restart_keep_latest_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA user_version=3;").unwrap();
        drop(conn);
        let store = Store::open(&path).unwrap();
        let target = BackupTarget::Local("/first".into());
        let mut last = String::new();
        for _ in 0..25 {
            let mut value = entry("/first");
            value.started = 1; // Same-millisecond attempts retain insertion ordering.
            last = value.id.clone();
            store.write_backup_history(value).await.unwrap();
        }
        let other = entry("/second");
        store.write_backup_history(other.clone()).await.unwrap();
        let rows = store.backup_history(target.clone()).await.unwrap();
        assert_eq!(rows.len(), PAGE);
        assert_eq!(rows[0].id, last);
        drop(store);
        let store = Store::open(path).unwrap();
        let rows = store.backup_history(target).await.unwrap();
        assert_eq!(rows.len(), PAGE);
        assert_eq!(rows[0].outcome, Outcome::Unfinished);
        assert_eq!(
            store.backup_history(other.target).await.unwrap()[0].id,
            other.id
        );
    }
    #[tokio::test]
    async fn backup_history_recovery_preserves_copy_identity_and_confirmed_warning() {
        let store = Store::memory().unwrap();
        let mut old = entry("/first");
        old.copy = Some("reserved-copy".into());
        old.outcome = Outcome::NeedsReview;
        old.detail = "Connection lost after upload".into();
        store.write_backup_history(old.clone()).await.unwrap();
        let mut unrelated = entry("/first");
        unrelated.copy = Some("another-copy".into());
        unrelated.outcome = Outcome::NeedsReview;
        store.write_backup_history(unrelated.clone()).await.unwrap();
        let mut recovered = entry("/first");
        recovered.copy = old.copy.clone();
        recovered.outcome = Outcome::SavedWithWarning;
        recovered.detail = "Keychain is locked".into();
        store.write_backup_history(recovered.clone()).await.unwrap();
        let rows = store.backup_history(old.target.clone()).await.unwrap();
        assert_eq!(
            rows.iter().find(|r| r.id == old.id).unwrap().outcome,
            Outcome::Recovered
        );
        assert_eq!(
            rows.iter().find(|r| r.id == unrelated.id).unwrap().outcome,
            Outcome::NeedsReview
        );
        assert_eq!(rows[0].outcome, Outcome::SavedWithWarning);
        let mut wrong = recovered.clone();
        wrong.copy = Some("foreign-copy".into());
        assert!(store.write_backup_history(wrong).await.is_err());
        let mut wrong = recovered.clone();
        wrong.target = BackupTarget::Local("/other".into());
        assert!(store.write_backup_history(wrong).await.is_err());
        recovered.outcome = Outcome::Failed;
        assert!(store.write_backup_history(recovered).await.is_err());
    }
    #[tokio::test]
    async fn backup_history_global_bound_and_error_text_are_bounded() {
        let store = Store::memory().unwrap();
        for n in 0..645 {
            let mut row = entry(&format!("/{n}"));
            row.detail = "λ".repeat(3000);
            store.write_backup_history(row).await.unwrap();
        }
        let count: i64 = store
            .run(|c| Ok(c.query_row("SELECT count(*) FROM backup_history", [], |r| r.get(0))?))
            .await
            .unwrap();
        assert_eq!(count, 640);
        let rows = store
            .backup_history(BackupTarget::Local("/644".into()))
            .await
            .unwrap();
        assert_eq!(rows[0].detail.chars().count(), 2048);
    }
}
