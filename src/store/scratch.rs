//! Connection-local, disk-backed work tables. Persistent caches must not spill
//! selection identities into unencrypted SQLite TEMP files or unbounded memory.
use crate::cache_cipher::Key;
use anyhow::Context;
use rusqlite::Connection;
use std::path::Path;

pub(super) fn attach(
    connection: &Connection,
    key: Option<&Key>,
) -> anyhow::Result<Option<tempfile::TempDir>> {
    let Some(path) = connection.path().filter(|path| !path.is_empty()) else {
        connection.execute("ATTACH DATABASE ':memory:' AS scratch", [])?;
        return Ok(None);
    };
    let directory = tempfile::Builder::new()
        .prefix(".shep-cache-scratch-")
        .tempdir_in(
            Path::new(path)
                .parent()
                .context("The cache has no data directory")?,
        )?;
    let scratch = directory.path().join("scratch.sqlite");
    // The empty file is attached without executing a key-bearing SQL string.
    connection.execute(
        "ATTACH DATABASE ? AS scratch KEY ''",
        [scratch
            .to_str()
            .context("The cache path is not valid UTF-8")?],
    )?;
    let ephemeral;
    let key = match key {
        Some(key) => key,
        None => {
            // Legacy unencrypted stores also keep new scratch files private.
            ephemeral = Key::generate()?;
            &ephemeral
        }
    };
    let initialized = (|| -> anyhow::Result<()> {
        key.apply(connection, c"scratch")?;
        connection.execute_batch(
            "PRAGMA scratch.journal_mode=DELETE;
            PRAGMA scratch.synchronous=FULL; PRAGMA scratch.cache_size=-2048;
            CREATE TABLE scratch.owner(version INTEGER NOT NULL);
            INSERT INTO scratch.owner VALUES(1);",
        )?;
        Ok(())
    })();
    if let Err(error) = initialized {
        let _ = connection.execute_batch("DETACH DATABASE scratch");
        return Err(error);
    }
    Ok(Some(directory))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::*,
        store::{MailSelectionId, Store},
    };
    use futures::poll;
    use std::sync::Arc;
    use tokio::sync::oneshot;

    #[test]
    fn cache_initialization_failure_closes_before_removing_its_scratch() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("future.sqlite");
        let key = Arc::new(Key::generate().unwrap());
        let connection = key.open(&path, rusqlite::OpenFlags::default()).unwrap();
        connection.pragma_update(None, "user_version", 999).unwrap();
        drop(connection);
        let original = std::fs::read(&path).unwrap();
        assert!(Store::open_encrypted(&path, key).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn encrypted_selection_scratch_is_private_indexed_and_owned_until_writes_drain() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mail.sqlite");
        let key = Arc::new(Key::generate().unwrap());
        let store = Store::open_encrypted(&path, key.clone()).unwrap();
        let mail = (0..320).map(|id| {
            let mut mail = parse_mail("fictional", &format!("private-fixture-{id}"), "INBOX",
                format!("From: Sender {id} <fictional@example.test>\r\nSubject: test {id}\r\n\r\nprivate encrypted selection test").into_bytes(),false,false).unwrap();
            mail.summary.timestamp = id;
            mail
        }).collect();
        store.upsert(mail).await.unwrap();
        let selection = MailSelectionId::default();
        let selected = store
            .capture_selection(selection, 0, MailQuery::default(), true, vec![])
            .await
            .unwrap();
        assert_eq!(selected.selected, 320);
        let scratch = store.run(|c| {
            let path: String = c.query_row("SELECT file FROM pragma_database_list WHERE name='scratch'",[],|r|r.get(0))?;
            assert_eq!(c.query_row("PRAGMA scratch.cache_size",[],|r|r.get::<_,i64>(0))?,-2048);
            for direction in ["ASC","DESC"] {
                let plan = c.prepare(&format!("EXPLAIN QUERY PLAN SELECT id FROM scratch.mail_selection_order ORDER BY priority,missing,score,label COLLATE NOCASE,time {direction},id"))?
                    .query_map([],|r|r.get::<_,String>(3))?.collect::<Result<Vec<_>,_>>()?.join("\n");
                assert!(plan.contains("INDEX"),"{plan}");
                assert!(!plan.contains("TEMP B-TREE"),"{plan}");
            }
            Ok(std::path::PathBuf::from(path))
        }).await.unwrap();
        let bytes = std::fs::read(&scratch).unwrap();
        assert!(!bytes.starts_with(b"SQLite format 3\0"));
        assert!(!bytes.windows(15).any(|window| window == b"private-fixture"));
        let plain = Connection::open(&scratch).unwrap();
        assert!(
            plain
                .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                    .get::<_, i64>(0))
                .is_err()
        );
        drop(plain);
        let reader = key
            .open(&scratch, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
        assert_eq!(
            reader
                .query_row("SELECT count(*) FROM mail_selection_rows", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            320
        );
        drop(reader);
        let (started, waiting) = oneshot::channel();
        let (release, held) = oneshot::channel();
        let mut first = Box::pin(store.run(move |_| {
            started.send(()).unwrap();
            held.blocking_recv().unwrap();
            Ok(())
        }));
        assert!(poll!(first.as_mut()).is_pending());
        waiting.await.unwrap();
        let (committed, observed) = oneshot::channel();
        let mut write = Box::pin(store.run(move |c| {
            c.execute("INSERT INTO scratch.owner VALUES(2)", [])?;
            c.execute("INSERT INTO kv VALUES('drained','true')", [])?;
            committed.send(()).unwrap();
            Ok(())
        }));
        assert!(poll!(write.as_mut()).is_pending());
        drop(write);
        drop(first);
        drop(store);
        assert!(scratch.exists());
        release.send(()).unwrap();
        observed.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            // Removing the file precedes removing its directory. Observe the
            // complete owned cleanup, not an intermediate filesystem step.
            while scratch.parent().unwrap().exists() {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(!scratch.exists());
        assert!(!scratch.parent().unwrap().exists());
        let reopened = Store::open_encrypted(&path, key).unwrap();
        assert!(reopened.get::<bool>("drained").await.unwrap());
        assert!(
            reopened
                .selection_snapshot(selection, vec![])
                .await
                .is_err()
        );
    }
}
