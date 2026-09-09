//! The first migration phase creates and verifies an encrypted candidate while
//! keeping the plaintext source untouched. Publication belongs to the separate
//! data-root recovery state machine; dropping a candidate only deletes itself.
use super::Key;
use anyhow::{Context, ensure};
use rusqlite::OpenFlags;
use std::path::Path;
use tokio::sync::oneshot;

pub struct Candidate {
    file: tempfile::NamedTempFile,
}
impl Candidate {
    pub fn path(&self) -> &Path {
        self.file.path()
    }
}

/// Must run on a background worker with the data-root migration lease held.
/// Cancellation may interrupt copying, never publish a partially copied cache.
pub fn stage_plaintext(
    source: &Path,
    directory: &Path,
    key: &Key,
    mut cancel: oneshot::Receiver<()>,
) -> anyhow::Result<Candidate> {
    stage(source, directory, key, move || {
        !matches!(cancel.try_recv(), Err(oneshot::error::TryRecvError::Empty))
    })
}

fn stage(
    source: &Path,
    directory: &Path,
    key: &Key,
    mut cancelled: impl FnMut() -> bool + Send + 'static,
) -> anyhow::Result<Candidate> {
    ensure!(
        !cancelled(),
        "Cache encryption was cancelled. The original database was kept."
    );
    let file = tempfile::Builder::new()
        .prefix(".shep-encrypted-")
        .suffix(".partial")
        .tempfile_in(directory)
        .context(
            "Could not create an encrypted cache candidate. Check available storage and retry.",
        )?;
    let c = key.open(file.path(), OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    c.busy_timeout(std::time::Duration::from_secs(5))?;
    c.execute_batch(
        "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA cache_size=-2048;",
    )?;
    let source = source.canonicalize()?;
    let mut uri = url::Url::from_file_path(source)
        .map_err(|_| anyhow::anyhow!("This cache path cannot be represented for SQLite"))?;
    uri.query_pairs_mut().append_pair("mode", "ro");
    // Source is explicitly read-only and unkeyed. The destination connection is
    // keyed before ATTACH; no key material passes through traced SQL text.
    c.execute("ATTACH DATABASE ? AS plaintext KEY ''", [uri.as_str()])?;
    c.progress_handler(1000, Some(cancelled))?;
    // One read transaction pins source pages. SQLCipher's logical export copies
    // all schema/BLOBs/virtual tables without loading the database into Rust RAM.
    let result = (|| -> anyhow::Result<()> {
        let tx = c.unchecked_transaction()?;
        tx.query_row("SELECT count(*) FROM plaintext.sqlite_schema", [], |r| {
            r.get::<_, i64>(0)
        })?;
        let version: i64 = tx.query_row("PRAGMA plaintext.user_version", [], |r| r.get(0))?;
        let application: i64 = tx.query_row("PRAGMA plaintext.application_id", [], |r| r.get(0))?;
        tx.query_row(
            "SELECT sqlcipher_export('main','plaintext')",
            [],
            |_| Ok(()),
        )?;
        tx.pragma_update(None, "user_version", version)?;
        tx.pragma_update(None, "application_id", application)?;
        tx.commit()?;
        Ok(())
    })();
    c.progress_handler(0, None::<fn() -> bool>)?;
    result.context(
        "Could not finish the encrypted cache candidate. The original database was kept.",
    )?;
    c.execute_batch("DETACH DATABASE plaintext;")?;
    drop(c);
    let check = key.open(file.path(), OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let integrity: String = check.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(
        integrity == "ok",
        "The encrypted cache failed validation. The original database was kept."
    );
    let mut cipher = check.prepare("PRAGMA cipher_integrity_check")?;
    ensure!(
        cipher.query([])?.next()?.is_none(),
        "The encrypted cache failed authentication. The original database was kept."
    );
    drop(cipher);
    drop(check);
    file.as_file().sync_all()?;
    Ok(Candidate { file })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    fn fixture(path: &Path) {
        let c = Connection::open(path).unwrap();
        c.execute_batch("PRAGMA user_version=3; PRAGMA application_id=1397245264;
            CREATE TABLE mail(id INTEGER PRIMARY KEY, body TEXT, original BLOB);
            CREATE VIRTUAL TABLE search USING fts5(body);
            CREATE TRIGGER inserted AFTER INSERT ON mail BEGIN INSERT INTO search(rowid,body) VALUES(new.id,new.body); END;
            INSERT INTO mail(body,original) VALUES('Private fictional body',x'00010203FF');
            WITH RECURSIVE rows(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM rows WHERE n<1000) INSERT INTO mail(body,original) SELECT 'Fictional '||n,zeroblob(1024) FROM rows;").unwrap();
    }

    #[test]
    fn staged_encryption_preserves_schema_versions_fts_blobs_and_original() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("plain.sqlite");
        fixture(&source);
        let original = std::fs::read(&source).unwrap();
        let key = Key::generate().unwrap();
        let (_send, receive) = oneshot::channel();
        let candidate = stage_plaintext(&source, dir.path(), &key, receive).unwrap();
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let c = key
            .open(candidate.path(), OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
        assert_eq!(
            c.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            c.query_row("PRAGMA application_id", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            1397245264
        );
        assert_eq!(
            c.query_row("SELECT original FROM mail WHERE id=1", [], |r| r
                .get::<_, Vec<u8>>(0))
                .unwrap(),
            vec![0, 1, 2, 3, 255]
        );
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM search WHERE search MATCH 'fictional'",
                [],
                |r| r.get::<_, i32>(0)
            )
            .unwrap(),
            1001
        );
        drop(c);
        let candidate_path = candidate.path().to_owned();
        drop(candidate);
        assert!(!candidate_path.exists());
        assert_eq!(std::fs::read(&source).unwrap(), original);
    }

    #[test]
    fn cancelled_copy_leaves_only_the_unchanged_original() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("plain.sqlite");
        fixture(&source);
        let original = std::fs::read(&source).unwrap();
        let mut calls = 0;
        let result = stage(&source, dir.path(), &Key::generate().unwrap(), move || {
            calls += 1;
            calls > 10
        });
        assert!(result.is_err());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
