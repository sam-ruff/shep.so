//! Connection-local search statistics: FTS5 phrase document counts and typo
//! expansions for one snapshot of the mail index. A change through this
//! connection or a commit by another one starts a fresh snapshot, so cached
//! ranking input can never outlive the index it was measured on.
use rusqlite::{Connection, OptionalExtension, params};

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE scratch.search_snapshot(id INTEGER PRIMARY KEY CHECK(id=1),
            changes INTEGER NOT NULL, data_version INTEGER NOT NULL);
        CREATE TABLE scratch.search_phrase_docs(phrase TEXT PRIMARY KEY, docs INTEGER NOT NULL);
        CREATE TABLE scratch.search_expansions(token TEXT PRIMARY KEY, alternatives TEXT NOT NULL);",
    )?;
    Ok(())
}

pub(super) struct Snapshot {
    written: bool,
}

impl Snapshot {
    /// Keep cached statistics only while nothing has changed since they were
    /// sealed; otherwise start empty.
    pub fn open(c: &Connection) -> anyhow::Result<Self> {
        let stored = c
            .query_row(
                "SELECT changes,data_version FROM scratch.search_snapshot WHERE id=1",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        if stored == Some(current(c)?) {
            return Ok(Self { written: false });
        }
        c.execute_batch(
            "DELETE FROM scratch.search_phrase_docs; DELETE FROM scratch.search_expansions;",
        )?;
        Ok(Self { written: true })
    }

    /// Rows matching one FTS5 phrase, counted at most once per snapshot.
    pub fn phrase_docs(&mut self, c: &Connection, phrase: &str) -> anyhow::Result<i64> {
        let cached = c
            .prepare_cached("SELECT docs FROM scratch.search_phrase_docs WHERE phrase=?")?
            .query_row([phrase], |row| row.get::<_, i64>(0))
            .optional()?;
        if let Some(docs) = cached {
            return Ok(docs);
        }
        let docs: i64 = c
            .prepare_cached("SELECT COUNT(*) FROM mail_search WHERE mail_search MATCH ?")?
            .query_row([phrase], |row| row.get(0))?;
        c.prepare_cached(
            "INSERT OR REPLACE INTO scratch.search_phrase_docs(phrase,docs) VALUES(?,?)",
        )?
        .execute(params![phrase, docs])?;
        self.written = true;
        Ok(docs)
    }

    /// FTS5 alternatives for one query token, expanded at most once per snapshot.
    pub fn expansion(
        &mut self,
        c: &Connection,
        token: &str,
        expand: impl FnOnce(&Connection, &str) -> anyhow::Result<Vec<String>>,
    ) -> anyhow::Result<Vec<String>> {
        let cached = c
            .prepare_cached("SELECT alternatives FROM scratch.search_expansions WHERE token=?")?
            .query_row([token], |row| row.get::<_, String>(0))
            .optional()?;
        if let Some(alternatives) = cached {
            return Ok(serde_json::from_str(&alternatives)?);
        }
        let alternatives = expand(c, token)?;
        c.prepare_cached(
            "INSERT OR REPLACE INTO scratch.search_expansions(token,alternatives) VALUES(?,?)",
        )?
        .execute(params![token, serde_json::to_string(&alternatives)?])?;
        self.written = true;
        Ok(alternatives)
    }

    /// Whether this snapshot added statistics that need the enclosing
    /// transaction to commit.
    pub fn written(&self) -> bool {
        self.written
    }

    /// Record the index state the cache describes. The record's own insert is
    /// the last counted change, so an untouched connection reads it back equal.
    pub fn seal(self, c: &Connection) -> anyhow::Result<()> {
        if !self.written {
            return Ok(());
        }
        let (changes, data_version) = current(c)?;
        c.execute(
            "INSERT OR REPLACE INTO scratch.search_snapshot(id,changes,data_version) VALUES(1,?,?)",
            params![changes + 1, data_version],
        )?;
        Ok(())
    }
}

fn current(c: &Connection) -> anyhow::Result<(i64, i64)> {
    let changes = c.query_row("SELECT total_changes()", [], |row| row.get(0))?;
    let data_version = c.query_row("PRAGMA main.data_version", [], |row| row.get(0))?;
    Ok((changes, data_version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;
    use crate::store::Store;

    fn mail(id: usize, body: &str) -> StoredMail {
        parse_mail(
            "fixture",
            &id.to_string(),
            "INBOX",
            format!("Subject: Notes\r\n\r\n{body}").into_bytes(),
            false,
            false,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn counts_and_expansions_survive_reads_but_not_index_changes() {
        let store = Store::memory().unwrap();
        store
            .upsert(vec![mail(1, "milestone"), mail(2, "milestone plans")])
            .await
            .unwrap();
        store
            .run(|c| {
                let tx = c.transaction()?;
                let mut snapshot = Snapshot::open(&tx)?;
                assert_eq!(snapshot.phrase_docs(&tx, "\"milestone\"")?, 2);
                let mut expansions = 0;
                let alternatives = snapshot.expansion(&tx, "milestnoe", |_, _| {
                    expansions += 1;
                    Ok(vec!["\"milestone\"".into()])
                })?;
                assert_eq!(alternatives, ["\"milestone\""]);
                snapshot.seal(&tx)?;
                tx.commit()?;

                let tx = c.transaction()?;
                let mut snapshot = Snapshot::open(&tx)?;
                assert!(!snapshot.written, "a read-only gap keeps the snapshot");
                snapshot.expansion(&tx, "milestnoe", |_, _| {
                    expansions += 1;
                    Ok(Vec::new())
                })?;
                assert_eq!(expansions, 1, "the cached expansion is reused");
                snapshot.seal(&tx)?;
                tx.commit()?;
                Ok(())
            })
            .await
            .unwrap();
        store
            .upsert(vec![mail(3, "milestone again")])
            .await
            .unwrap();
        store
            .run(|c| {
                let tx = c.transaction()?;
                let mut snapshot = Snapshot::open(&tx)?;
                assert!(snapshot.written, "an index change starts a new snapshot");
                assert_eq!(snapshot.phrase_docs(&tx, "\"milestone\"")?, 3);
                let alternatives = snapshot.expansion(&tx, "milestnoe", |_, _| Ok(Vec::new()))?;
                assert!(alternatives.is_empty(), "stale expansions are dropped");
                snapshot.seal(&tx)?;
                tx.commit()?;
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_rolled_back_fill_does_not_leave_a_sealed_snapshot_behind() {
        let store = Store::memory().unwrap();
        store.upsert(vec![mail(1, "milestone")]).await.unwrap();
        store
            .run(|c| {
                let tx = c.transaction()?;
                let mut snapshot = Snapshot::open(&tx)?;
                snapshot.phrase_docs(&tx, "\"milestone\"")?;
                snapshot.seal(&tx)?;
                drop(tx);
                let tx = c.transaction()?;
                let snapshot = Snapshot::open(&tx)?;
                assert!(snapshot.written);
                Ok(())
            })
            .await
            .unwrap();
    }
}
