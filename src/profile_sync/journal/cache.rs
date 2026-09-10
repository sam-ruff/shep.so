//! Optional verified record cache. It cannot replace a current, complete Drive
//! listing or the history owner's immutable-operation checks. One bounded record
//! crosses the owning channel at a time, including when reopening a large history.
use super::*;

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS downloads(
        identity TEXT NOT NULL,namespace TEXT NOT NULL,
        profile TEXT NOT NULL,generation TEXT NOT NULL,operation TEXT NOT NULL,
        drive_id TEXT NOT NULL,sha256 TEXT NOT NULL,bytes BLOB NOT NULL,
        PRIMARY KEY(identity,namespace,profile,generation,operation));",
    )?;
    Ok(())
}

fn load(
    c: &Connection,
    binding: &Binding,
    remote: &RemoteRecord,
) -> anyhow::Result<Option<Record>> {
    // Do not allocate externally corrupted or oversized blobs. A cache miss is
    // repaired only by another fully verified download, never guessed content.
    let bytes: Option<Option<Vec<u8>>> = c.query_row(
        "SELECT CASE WHEN length(bytes)=? AND length(bytes)<=? THEN bytes ELSE NULL END FROM downloads WHERE identity=? AND namespace=? AND profile=? AND generation=? AND operation=? AND drive_id=? AND sha256=?",
        params![i64::try_from(remote.size)?,MAX_RECORD_BYTES as i64,binding.identity,binding.namespace,remote.key.profile.to_string(),remote.key.generation.to_string(),remote.key.operation.to_string(),remote.id,remote.sha256],
        |r|r.get(0)).optional()?;
    let Some(bytes) = bytes.flatten() else {
        return Ok(None);
    };
    let Ok(record) = Record::decode(&binding.namespace, bytes) else {
        return Ok(None);
    };
    Ok(remote.verify(&record).is_ok().then_some(record))
}

impl Journal {
    pub(crate) async fn cached_download(
        &self,
        scan: &Scan,
        remote: &RemoteRecord,
    ) -> anyhow::Result<Option<Record>> {
        let scan = scan.clone();
        let remote = remote.clone();
        self.worker
            .run(move |c| {
                let tx = c.transaction()?;
                scan.require_record(&tx, &remote)?;
                load(&tx, scan.binding(), &remote)
            })
            .await
    }

    pub(crate) async fn cache_download(
        &self,
        scan: &Scan,
        remote: &RemoteRecord,
        record: Record,
    ) -> anyhow::Result<Record> {
        let scan = scan.clone();
        let remote = remote.clone();
        self.worker.run(move |c| {
            let tx = c.transaction()?;
            scan.require_record(&tx, &remote)?;
            remote.verify(&record)?;
            anyhow::ensure!(record.operation.namespace == scan.binding().namespace, "The cached profile record belongs to another namespace.");
            let binding = scan.binding();
            tx.execute("INSERT INTO downloads(identity,namespace,profile,generation,operation,drive_id,sha256,bytes) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(identity,namespace,profile,generation,operation) DO UPDATE SET drive_id=excluded.drive_id,sha256=excluded.sha256,bytes=excluded.bytes",
                params![binding.identity,binding.namespace,remote.key.profile.to_string(),remote.key.generation.to_string(),remote.key.operation.to_string(),remote.id,remote.sha256,record.bytes()])?;
            tx.commit()?;
            Ok(record)
        }).await
    }

    // Existing transcript helpers plan replies for cache misses. Production
    // callers must present a current Scan through cached_download instead.
    #[cfg(test)]
    pub(in crate::profile_sync) async fn fixture_cached_download(
        &self,
        binding: &Binding,
        remote: &RemoteRecord,
    ) -> bool {
        let binding = binding.clone();
        let remote = remote.clone();
        self.worker
            .run(move |c| load(c, &binding, &remote))
            .await
            .unwrap()
            .is_some()
    }
}

#[cfg(test)]
mod tests;
