//! Durable upload records owned by a bounded FIFO worker, separate from the mail cache.
use super::*;
use rusqlite::{Connection, OptionalExtension, params};
use std::{path::Path, sync::Arc};

#[derive(Clone)]
pub(crate) struct Journal(Arc<crate::store::worker::Worker>);
pub(crate) struct Pending {
    pub upload: PreparedUpload,
    pub data: Vec<u8>,
    pub committed: bool,
}
impl Journal {
    pub fn open(path: Option<&Path>) -> anyhow::Result<Self> {
        let connection = match path {
            Some(path) => Connection::open(path)?,
            None => Connection::open_in_memory()?,
        };
        Self::from_connection(connection)
    }
    pub fn open_encrypted(path: &Path, key: &crate::cache_cipher::Key) -> anyhow::Result<Self> {
        Self::from_connection(key.open(path, rusqlite::OpenFlags::default())?)
    }
    fn from_connection(connection: Connection) -> anyhow::Result<Self> {
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS uploads (
                target TEXT PRIMARY KEY, id TEXT NOT NULL, name TEXT NOT NULL,
                size INTEGER NOT NULL, sha256 TEXT NOT NULL, session TEXT,
                committed INTEGER NOT NULL DEFAULT 0, archive BLOB NOT NULL);",
        )?;
        Ok(Self(Arc::new(crate::store::worker::Worker::named(
            connection,
            "shep-backup-journal",
        )?)))
    }
    async fn run<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Connection) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        // Worker admission is bounded at 32 commands. Accepted SQL jobs drain
        // even if this observation is cancelled or the final handle is dropped.
        self.0.run(job).await
    }
    pub async fn pending(&self, target: &BackupTarget) -> anyhow::Result<Option<Pending>> {
        let target = serde_json::to_string(target)?;
        self.run(move |connection| read(connection, &target)).await
    }
    pub async fn prepare(
        &self,
        target: &BackupTarget,
        upload: PreparedUpload,
        data: Vec<u8>,
    ) -> anyhow::Result<Pending> {
        let target = serde_json::to_string(target)?;
        self.run(move |connection| {
            upload.verify(&data)?;
            let transaction = connection.transaction()?;
            transaction.execute("INSERT INTO uploads(target,id,name,size,sha256,session,archive) VALUES(?,?,?,?,?,?,?) ON CONFLICT(target) DO NOTHING",
                params![target, upload.id, upload.name, upload.size as i64, upload.sha256, upload.session, data])?;
            let pending = read(&transaction, &target)?.context("Backup journal did not retain the archive")?;
            transaction.commit()?;
            Ok(pending)
        }).await
    }
    pub async fn checkpoint(
        &self,
        target: &BackupTarget,
        upload: &PreparedUpload,
    ) -> anyhow::Result<()> {
        let target = serde_json::to_string(target)?;
        let upload = upload.clone();
        self.run(move |connection| {
            let changed = connection.execute("UPDATE uploads SET session=? WHERE target=? AND id=? AND name=? AND size=? AND sha256=?",
                params![upload.session, target, upload.id, upload.name, upload.size as i64, upload.sha256])?;
            anyhow::ensure!(changed == 1, "The pending backup changed. Its session was not replaced.");
            Ok(())
        }).await
    }
    pub async fn committed(&self, target: &BackupTarget, id: &str) -> anyhow::Result<()> {
        let target = serde_json::to_string(target)?;
        let id = id.to_owned();
        self.run(move |connection| {
            let changed = connection.execute(
                "UPDATE uploads SET committed=1 WHERE target=? AND id=?",
                params![target, id],
            )?;
            anyhow::ensure!(
                changed == 1,
                "The acknowledged backup was not found in its upload journal."
            );
            Ok(())
        })
        .await
    }
    pub async fn remove(&self, target: &BackupTarget, id: &str) -> anyhow::Result<()> {
        let target = serde_json::to_string(target)?;
        let id = id.to_owned();
        self.run(move |connection| {
            connection.execute(
                "DELETE FROM uploads WHERE target=? AND id=? AND committed=1",
                params![target, id],
            )?;
            Ok(())
        })
        .await
    }
}

fn read(connection: &Connection, target: &str) -> anyhow::Result<Option<Pending>> {
    let info = connection.query_row("SELECT id,name,size,sha256,session,committed,length(archive) FROM uploads WHERE target=?", [target], |row| {
        Ok((PreparedUpload { id: row.get(0)?, name: row.get(1)?, size: row.get::<_, i64>(2)? as u64, sha256: row.get(3)?, session: row.get(4)? }, row.get::<_, bool>(5)?, row.get::<_, i64>(6)? as u64))
    }).optional()?;
    let Some((upload, committed, size)) = info else {
        return Ok(None);
    };
    anyhow::ensure!(
        size <= MAX_DECODED && size == upload.size,
        "The pending backup exceeds its recorded size. Its upload was not retried."
    );
    let data: Vec<u8> = connection.query_row(
        "SELECT archive FROM uploads WHERE target=?",
        [target],
        |row| row.get(0),
    )?;
    upload.verify(&data)?;
    Ok(Some(Pending {
        upload,
        data,
        committed,
    }))
}

pub(crate) struct Checkpoint {
    pub journal: Journal,
    pub target: BackupTarget,
}
#[async_trait]
impl UploadCheckpoint for Checkpoint {
    async fn save(&self, upload: &PreparedUpload) -> anyhow::Result<()> {
        self.journal.checkpoint(&self.target, upload).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn upload(id: &str, data: &[u8]) -> PreparedUpload {
        PreparedUpload::new(
            id.into(),
            format!("shep-20260906T120000Z-{}.shepbackup", uuid::Uuid::nil()),
            data,
        )
    }
    mod ownership;

    #[tokio::test]
    async fn encrypted_journal_retains_exact_upload_and_session_with_the_cache_key() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("uploads.sqlite");
        let key = crate::cache_cipher::Key::generate().unwrap();
        let target = BackupTarget::Local("private-fictional-destination".into());
        let bytes = b"Fixture archive bytes which may include unencrypted backup output";
        let mut prepared = upload("fixture", bytes);
        let journal = Journal::open_encrypted(&path, &key).unwrap();
        journal
            .prepare(&target, prepared.clone(), bytes.to_vec())
            .await
            .unwrap();
        prepared.session = Some("fictional-private-upload-session".into());
        journal.checkpoint(&target, &prepared).await.unwrap();
        drop(journal);
        assert!(Journal::open(Some(&path)).is_err());
        assert!(
            Journal::open_encrypted(&path, &crate::cache_cipher::Key::generate().unwrap()).is_err()
        );
        let reopened = Journal::open_encrypted(&path, &key).unwrap();
        let pending = reopened.pending(&target).await.unwrap().unwrap();
        assert_eq!(pending.data, bytes);
        assert_eq!(pending.upload.session, prepared.session);
        reopened.committed(&target, "fixture").await.unwrap();
        drop(reopened);
        let original = std::fs::read(&path).unwrap();
        for private in [
            bytes.as_slice(),
            b"private-fictional-destination",
            b"fictional-private-upload-session",
        ] {
            assert!(!original.windows(private.len()).any(|part| part == private));
        }
        let reopened = Journal::open_encrypted(&path, &key).unwrap();
        assert!(reopened.pending(&target).await.unwrap().unwrap().committed);
    }
    #[tokio::test]
    async fn journal_reopen_retains_exact_bytes_and_reservation_before_remote_work() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("uploads.sqlite");
        let target = BackupTarget::Local("fixture".into());
        let first = upload("first", b"ciphertext");
        let journal = Journal::open(Some(&path)).unwrap();
        journal
            .prepare(&target, first.clone(), b"ciphertext".to_vec())
            .await
            .unwrap();
        // A competing preparation must use the winner's archive and ID.
        let winner = journal
            .prepare(
                &target,
                upload("second", b"different"),
                b"different".to_vec(),
            )
            .await
            .unwrap();
        assert_eq!(winner.upload.id, "first");
        assert_eq!(winner.data, b"ciphertext");
        let mut session = first.clone();
        session.session = Some("persisted-session".into());
        journal.checkpoint(&target, &session).await.unwrap();
        let mut stale = session.clone();
        stale.id = "other".into();
        assert!(journal.checkpoint(&target, &stale).await.is_err());
        assert!(journal.committed(&target, "other").await.is_err());
        journal.remove(&target, "first").await.unwrap();
        drop(journal);
        let reopened = Journal::open(Some(&path)).unwrap();
        let pending = reopened.pending(&target).await.unwrap().unwrap();
        assert_eq!(pending.upload.session.as_deref(), Some("persisted-session"));
        assert!(!pending.committed);
        reopened.committed(&target, "first").await.unwrap();
        assert!(reopened.pending(&target).await.unwrap().unwrap().committed);
        reopened.remove(&target, "first").await.unwrap();
        assert!(reopened.pending(&target).await.unwrap().is_none());
        reopened
            .prepare(
                &target,
                upload("second", b"different"),
                b"different".to_vec(),
            )
            .await
            .unwrap();
        reopened.remove(&target, "first").await.unwrap();
        assert_eq!(
            reopened.pending(&target).await.unwrap().unwrap().upload.id,
            "second"
        );
    }
    #[tokio::test]
    async fn journal_rejects_modified_ciphertext_and_invalid_recorded_sizes() {
        let target = BackupTarget::Local("fixture".into());
        for query in [
            "UPDATE uploads SET archive=x'616263'",
            "UPDATE uploads SET size=-1",
            "UPDATE uploads SET size=2147483647",
        ] {
            let journal = Journal::open(None).unwrap();
            journal
                .prepare(&target, upload("one", b"xyz"), b"xyz".to_vec())
                .await
                .unwrap();
            journal
                .run(move |connection| {
                    connection.execute_batch(query)?;
                    Ok(())
                })
                .await
                .unwrap();
            assert!(journal.pending(&target).await.is_err());
        }
    }
}
