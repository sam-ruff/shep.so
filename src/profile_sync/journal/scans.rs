//! Persist pagination proofs separately from profile merge/application. A partial
//! or looping Drive list can never be reported as a complete empty discovery.
use super::*;
use crate::profile_sync::drive::Page;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scan {
    id: Uuid,
    binding: Binding,
    profile: Option<(Uuid, Uuid)>,
    cursor: Option<String>,
    revision: u64,
    files: u64,
    complete: bool,
}
impl Scan {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub fn profile(&self) -> Option<(Uuid, Uuid)> {
        self.profile
    }
    pub fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }
    pub fn complete(&self) -> bool {
        self.complete
    }
    pub fn files(&self) -> u64 {
        self.files
    }
}
#[derive(Clone, Debug)]
pub struct ScanEntry {
    pub position: u64,
    pub record: RemoteRecord,
}

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS scans(
        id TEXT PRIMARY KEY, identity TEXT NOT NULL, namespace TEXT NOT NULL,
        profile TEXT NOT NULL, generation TEXT NOT NULL, cursor TEXT,
        revision INTEGER NOT NULL DEFAULT 0, files INTEGER NOT NULL DEFAULT 0,
        complete INTEGER NOT NULL DEFAULT 0 CHECK(complete IN (0,1)),
        UNIQUE(identity,namespace,profile,generation));
        CREATE TABLE IF NOT EXISTS scan_pages(scan TEXT NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
            token TEXT NOT NULL, PRIMARY KEY(scan,token));
        CREATE TABLE IF NOT EXISTS scan_files(scan TEXT NOT NULL REFERENCES scans(id) ON DELETE CASCADE,
            position INTEGER NOT NULL,file_id TEXT NOT NULL,profile TEXT NOT NULL,generation TEXT NOT NULL,operation TEXT NOT NULL,data TEXT NOT NULL,
            PRIMARY KEY(scan,position),UNIQUE(scan,file_id),UNIQUE(scan,profile,generation,operation));")?;
    Ok(())
}

impl Journal {
    pub async fn begin_scan(
        &self,
        binding: Binding,
        profile: Option<(Uuid, Uuid)>,
    ) -> anyhow::Result<Scan> {
        binding.validate()?;
        if let Some((p, g)) = profile {
            anyhow::ensure!(
                !p.is_nil() && !g.is_nil(),
                "Choose a valid profile generation."
            );
        }
        self.worker.run(move |c| {
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let (p,g) = scope(profile);
            tx.execute("DELETE FROM scans WHERE identity=? AND namespace=? AND profile=? AND generation=?",params![binding.identity,binding.namespace,p,g])?;
            let scan = Scan { id:Uuid::new_v4(),binding,profile,cursor:None,revision:0,files:0,complete:false };
            tx.execute("INSERT INTO scans(id,identity,namespace,profile,generation) VALUES(?,?,?,?,?)",params![scan.id.to_string(),scan.binding.identity,scan.binding.namespace,p,g])?;
            tx.commit()?;
            Ok(scan)
        }).await
    }
    pub async fn resume_scan(
        &self,
        binding: &Binding,
        profile: Option<(Uuid, Uuid)>,
    ) -> anyhow::Result<Option<Scan>> {
        binding.validate()?;
        let binding = binding.clone();
        self.worker.run(move |c| load(c, &binding, profile)).await
    }
    pub async fn append_page(&self, expected: &Scan, page: Page) -> anyhow::Result<Scan> {
        let expected = expected.clone();
        anyhow::ensure!(
            page.binding == expected.binding
                && page.cursor == expected.cursor
                && page.profile == expected.profile,
            "This Drive page belongs to an earlier or different discovery. Resume the current scan."
        );
        anyhow::ensure!(
            !expected.complete && page.records.len() <= 100,
            "This profile discovery is complete or its page is too large."
        );
        if let Some(next) = &page.next {
            crate::profile_sync::drive::check_token(next)?;
        }
        for record in &page.records {
            record.validate()?;
            anyhow::ensure!(
                expected
                    .profile
                    .is_none_or(|p| p == (record.key.profile, record.key.generation)),
                "This page contains another profile generation."
            );
        }
        self.worker.run(move |c| {
            let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let current = load(&tx,&expected.binding,expected.profile)?.context("This profile discovery was replaced. Resume the current scan.")?;
            anyhow::ensure!(current==expected,"This profile discovery advanced while the page was pending. Resume its saved position.");
            let scan_id = current.id.to_string();
            tx.execute("INSERT INTO scan_pages(scan,token) VALUES(?,?)",params![scan_id,current.cursor.as_deref().unwrap_or("")])?;
            if let Some(next) = &page.next {
                let seen: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM scan_pages WHERE scan=? AND token=?)",params![scan_id,next],|r|r.get(0))?;
                anyhow::ensure!(!seen,"Google Drive repeated an earlier profile page. The partial discovery was kept; restart discovery before applying it.");
            }
            for (offset,record) in page.records.iter().enumerate() {
                let position = current.files.checked_add(offset as u64).context("The discovery position overflowed")?;
                tx.execute("INSERT INTO scan_files(scan,position,file_id,profile,generation,operation,data) VALUES(?,?,?,?,?,?,?)",params![scan_id,i64::try_from(position)?,record.id,record.key.profile.to_string(),record.key.generation.to_string(),record.key.operation.to_string(),serde_json::to_string(record)?]).context("A profile file or operation repeated across Drive pages. Restart discovery; the local setup was kept.")?;
            }
            let mut saved = current;
            saved.files = saved.files.checked_add(page.records.len() as u64).context("The discovery count overflowed")?;
            saved.revision = saved.revision.checked_add(1).context("The discovery revision overflowed")?;
            saved.complete = page.next.is_none();
            saved.cursor = page.next;
            tx.execute("UPDATE scans SET cursor=?,revision=?,files=?,complete=? WHERE id=?",params![saved.cursor,i64::try_from(saved.revision)?,i64::try_from(saved.files)?,saved.complete,scan_id])?;
            tx.commit()?;
            Ok(saved)
        }).await
    }

    /// The complete list is read in metadata pages. Partial discoveries cannot
    /// accidentally enroll a device or be interpreted as an empty Google setup.
    pub async fn scan_entries(
        &self,
        expected: &Scan,
        after: Option<u64>,
    ) -> anyhow::Result<Vec<ScanEntry>> {
        anyhow::ensure!(
            expected.complete,
            "Finish profile discovery before applying its records."
        );
        let expected = expected.clone();
        let after = after.map(i64::try_from).transpose()?.unwrap_or(-1);
        self.worker.run(move |c| {
            let tx = c.transaction()?;
            let current = load(&tx,&expected.binding,expected.profile)?.context("This profile discovery is no longer current")?;
            anyhow::ensure!(current==expected,"Profile discovery changed. Read its current completed list.");
            let mut query = tx.prepare("SELECT position,data FROM scan_files WHERE scan=? AND position>? ORDER BY position LIMIT 50")?;
            let records = query.query_map(params![expected.id.to_string(),after],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?)))?.map(|row| {
                let (position,data) = row?;
                let record:RemoteRecord = serde_json::from_str(&data)?;
                record.validate()?;
                Ok(ScanEntry { position:u64::try_from(position)?,record })
            }).collect::<anyhow::Result<Vec<_>>>()?;
            Ok(records)
        }).await
    }

    /// Indexed lookup prevents a recovered local edit reserving another Drive
    /// ID when the completed listing already contains its original operation.
    pub async fn scan_record(
        &self,
        expected: &Scan,
        key: Key,
    ) -> anyhow::Result<Option<RemoteRecord>> {
        key.validate()?;
        anyhow::ensure!(
            expected.complete,
            "Finish discovery before publishing profile edits."
        );
        let expected = expected.clone();
        self.worker.run(move |c| {
            let tx = c.transaction()?;
            let current = load(&tx, &expected.binding, expected.profile)?
                .context("This profile discovery is no longer current")?;
            anyhow::ensure!(current == expected, "Profile discovery changed. Pull again before publishing.");
            let data:Option<String> = tx.query_row("SELECT data FROM scan_files WHERE scan=? AND profile=? AND generation=? AND operation=?",params![current.id.to_string(),key.profile.to_string(),key.generation.to_string(),key.operation.to_string()],|r|r.get(0)).optional()?;
            data.map(|data| {
                let record:RemoteRecord = serde_json::from_str(&data)?;
                record.validate()?;
                anyhow::ensure!(record.key == key, "The discovered operation identity changed.");
                Ok(record)
            }).transpose()
        }).await
    }
}

fn scope(profile: Option<(Uuid, Uuid)>) -> (String, String) {
    profile
        .map(|(p, g)| (p.to_string(), g.to_string()))
        .unwrap_or_default()
}
fn load(
    c: &Connection,
    binding: &Binding,
    profile: Option<(Uuid, Uuid)>,
) -> anyhow::Result<Option<Scan>> {
    let (p, g) = scope(profile);
    type Row = (String, Option<String>, i64, i64, bool);
    let row:Option<Row>=c.query_row("SELECT id,cursor,revision,files,complete FROM scans WHERE identity=? AND namespace=? AND profile=? AND generation=?",params![binding.identity,binding.namespace,p,g],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
    row.map(|(id, cursor, revision, files, complete)| {
        if let Some(cursor) = &cursor {
            crate::profile_sync::drive::check_token(cursor)?;
        }
        anyhow::ensure!(
            !complete || cursor.is_none(),
            "The saved discovery position is inconsistent."
        );
        Ok(Scan {
            id: canonical_uuid(Some(&id))?,
            binding: binding.clone(),
            profile,
            cursor,
            revision: u64::try_from(revision)?,
            files: u64::try_from(files)?,
            complete,
        })
    })
    .transpose()
}

#[cfg(test)]
mod tests;
