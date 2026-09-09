//! Durable Drive upload/discovery state, independent of mail and merge workers.
//! It contains portable metadata only; OAuth/credential state stays in keychain.
use super::*;
use crate::store::worker::Worker;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
mod scans;
pub use scans::{Scan, ScanEntry};

const APPLICATION_ID: i32 = 0x5348_5055;
#[derive(Clone)]
pub struct Journal {
    worker: Arc<Worker>,
}

/// Minted only after the exact reservation and bytes commit in SQLite. Private
/// fields prevent a caller constructing an unjournaled transport upload.
#[derive(Clone, Debug)]
pub struct DurableUpload {
    upload: ReservedUpload,
    acknowledged: bool,
}
impl DurableUpload {
    pub fn upload(&self) -> &ReservedUpload {
        &self.upload
    }
    pub fn acknowledged(&self) -> bool {
        self.acknowledged
    }
}

impl Journal {
    /// Open off the UI thread. None is truly in-memory and creates no lock files.
    pub fn open(path: Option<&Path>) -> anyhow::Result<Self> {
        let mut c = match path {
            Some(path) => Connection::open(path)?,
            None => Connection::open_in_memory()?,
        };
        c.busy_timeout(std::time::Duration::from_secs(5))?;
        let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        let identity: i32 = tx.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        let tables: i64 = tx.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        anyhow::ensure!(
            (identity == APPLICATION_ID && version == 1)
                || (identity == 0 && version == 0 && tables == 0),
            "This file is not a supported profile-upload journal. Its contents were kept."
        );
        tx.execute_batch("CREATE TABLE IF NOT EXISTS uploads(
            identity TEXT NOT NULL, namespace TEXT NOT NULL,
            profile TEXT NOT NULL, generation TEXT NOT NULL, operation TEXT NOT NULL,
            drive_id TEXT NOT NULL, sha256 TEXT NOT NULL, bytes BLOB NOT NULL,
            acknowledged INTEGER NOT NULL DEFAULT 0 CHECK(acknowledged IN (0,1)),
            PRIMARY KEY(identity,namespace,profile,generation,operation),
            UNIQUE(identity,namespace,drive_id));
            CREATE INDEX IF NOT EXISTS pending_uploads ON uploads(identity,namespace,acknowledged,profile,generation,operation);")?;
        scans::schema(&tx)?;
        tx.pragma_update(None, "application_id", APPLICATION_ID)?;
        tx.pragma_update(None, "user_version", 1)?;
        tx.commit()?;
        c.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;",
        )?;
        Ok(Self {
            worker: Arc::new(Worker::named(c, "shep-profile-drive")?),
        })
    }

    pub async fn prepare(&self, upload: ReservedUpload) -> anyhow::Result<DurableUpload> {
        upload.validate()?;
        self.worker.run(move |c| {
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            if let Some(previous) = read(&tx, &upload.binding, upload.remote.key)? {
                anyhow::ensure!(previous.upload.remote == upload.remote && previous.upload.record.bytes() == upload.record.bytes(), "This operation already has another saved reservation. Retry the original pending change; it was not replaced.");
                return Ok(previous);
            }
            let key = upload.remote.key;
            tx.execute("INSERT INTO uploads(identity,namespace,profile,generation,operation,drive_id,sha256,bytes) VALUES(?,?,?,?,?,?,?,?)", params![upload.binding.identity,upload.binding.namespace,key.profile.to_string(),key.generation.to_string(),key.operation.to_string(),upload.remote.id,upload.remote.sha256,upload.record.bytes()])?;
            tx.commit()?;
            Ok(DurableUpload { upload, acknowledged: false })
        }).await
    }

    pub async fn load(&self, binding: &Binding, key: Key) -> anyhow::Result<Option<DurableUpload>> {
        binding.validate()?;
        key.validate()?;
        let binding = binding.clone();
        self.worker.run(move |c| read(c, &binding, key)).await
    }

    /// Receipt storage is separate from remote commitment. On a failed local
    /// acknowledgment, retry verifies the exact Drive file without another ID.
    pub async fn acknowledge(
        &self,
        durable: &DurableUpload,
        receipt: &RemoteRecord,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            receipt == &durable.upload.remote,
            "The upload receipt belongs to another profile operation."
        );
        let upload = durable.upload.clone();
        self.worker.run(move |c| {
            let key = upload.remote.key;
            let changed = c.execute("UPDATE uploads SET acknowledged=1 WHERE identity=? AND namespace=? AND profile=? AND generation=? AND operation=? AND drive_id=? AND sha256=? AND bytes=?", params![upload.binding.identity,upload.binding.namespace,key.profile.to_string(),key.generation.to_string(),key.operation.to_string(),upload.remote.id,upload.remote.sha256,upload.record.bytes()])?;
            anyhow::ensure!(changed == 1, "The profile upload was confirmed remotely, but its local journal changed. Keep the saved change and retry its acknowledgment.");
            Ok(())
        }).await
    }
}

fn read(c: &Connection, binding: &Binding, key: Key) -> anyhow::Result<Option<DurableUpload>> {
    // Check the persisted byte length before allocating a BLOB, including when
    // opening an externally modified journal. No arbitrary whole-history cap.
    type Stored = (String, String, i64, Option<Vec<u8>>, bool);
    let row: Option<Stored> = c.query_row("SELECT drive_id,sha256,length(bytes),CASE WHEN length(bytes)<=? THEN bytes ELSE NULL END,acknowledged FROM uploads WHERE identity=? AND namespace=? AND profile=? AND generation=? AND operation=?", params![MAX_RECORD_BYTES as i64,binding.identity,binding.namespace,key.profile.to_string(),key.generation.to_string(),key.operation.to_string()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    let Some((id, sha256, size, bytes, acknowledged)) = row else {
        return Ok(None);
    };
    let record = Record::decode(
        &binding.namespace,
        bytes.context(
            "The saved profile record exceeds its supported size. The journal was kept.",
        )?,
    )?;
    let upload = ReservedUpload {
        binding: binding.clone(),
        remote: RemoteRecord {
            id,
            key,
            size: u64::try_from(size)?,
            sha256,
        },
        record,
    };
    upload.validate()?;
    Ok(Some(DurableUpload {
        upload,
        acknowledged,
    }))
}

#[cfg(test)]
mod tests;
