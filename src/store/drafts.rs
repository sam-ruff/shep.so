use super::*;
use crate::compose::{FilePart, MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS};
use rusqlite::OptionalExtension;
use std::{io::Read, path::PathBuf};

#[derive(Debug, Clone, Default)]
pub struct DraftState {
    pub revision: u64,
    pub drafts: Vec<Draft>,
}
pub(super) fn changed(c: &Connection) -> anyhow::Result<()> {
    let revision: u64 = get(c, "drafts_revision")?;
    put(
        c,
        "drafts_revision",
        &revision.checked_add(1).context("Draft revision overflow")?,
    )
}
pub(super) fn snapshot(c: &Connection) -> anyhow::Result<DraftState> {
    let mut drafts: Vec<Draft> = get(c, "drafts")?;
    for draft in &mut drafts {
        draft.attachments = attachments(c, &draft.id)?;
    }
    Ok(DraftState {
        revision: get(c, "drafts_revision")?,
        drafts,
    })
}
fn attachments(c: &Connection, id: &str) -> anyhow::Result<Vec<DraftAttachment>> {
    Ok(c.prepare(
        "SELECT id,name,media_type,size FROM draft_attachments WHERE draft=? ORDER BY rowid",
    )?
    .query_map([id], |r| {
        Ok(DraftAttachment {
            id: r.get(0)?,
            name: r.get(1)?,
            media_type: r.get(2)?,
            size: r.get::<_, u32>(3)? as usize,
        })
    })?
    .collect::<Result<_, _>>()?)
}
fn sent(c: &Connection, draft: &Draft) -> anyhow::Result<bool> {
    let revision: Option<i64> = c
        .query_row(
            "SELECT revision FROM draft_sent WHERE id=?",
            [&draft.id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(revision.is_some_and(|revision| revision >= 0 && revision as u64 >= draft.revision))
}
pub(super) fn save(c: &Connection, mut draft: Draft) -> anyhow::Result<()> {
    connections::allow(c, ConnectionKind::Account, &draft.account_id)?;
    anyhow::ensure!(
        !draft.id.is_empty() && draft.id.len() <= 256 && draft.revision <= i64::MAX as u64,
        "The draft has an invalid identity or revision."
    );
    if sent(c, &draft)? {
        return Ok(());
    }
    let mut drafts: Vec<Draft> = get(c, "drafts")?;
    if let Some(current) = drafts.iter().find(|current| current.id == draft.id)
        && (current.revision > draft.revision
            || serde_json::to_string(current)? == serde_json::to_string(&draft)?)
    {
        return Ok(());
    }
    draft.attachments.clear();
    drafts.retain(|current| current.id != draft.id);
    drafts.push(draft);
    put(c, "drafts", &drafts)?;
    changed(c)
}

impl Store {
    pub async fn draft_state(&self) -> anyhow::Result<DraftState> {
        self.run(|c| snapshot(c)).await
    }
    pub async fn ensure_draft_unsent(&self, draft: Draft) -> anyhow::Result<()> {
        self.run(move |c| {
            anyhow::ensure!(!sent(c, &draft)?, "This version of the draft was already sent. Compose a new message to send another copy.");
            Ok(())
        }).await
    }
    pub async fn finish_draft_send(&self, draft: Draft) -> anyhow::Result<DraftState> {
        self.run(move |c| {
            let revision = i64::try_from(draft.revision).context("Invalid draft revision")?;
            let tx = c.transaction()?;
            tx.execute("INSERT INTO draft_sent(id,revision) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET revision=MAX(revision,excluded.revision)", params![draft.id, revision])?;
            let mut drafts: Vec<Draft> = get(&tx, "drafts")?;
            drafts.retain(|current| current.id != draft.id || current.revision > draft.revision);
            if !drafts.iter().any(|current| current.id == draft.id) {
                tx.execute("DELETE FROM draft_attachments WHERE draft=?", [&draft.id])?;
            }
            put(&tx, "drafts", &drafts)?;
            changed(&tx)?;
            let state = snapshot(&tx)?;
            tx.commit()?;
            Ok(state)
        }).await
    }
    pub async fn add_draft_files(
        &self,
        draft: Draft,
        paths: Vec<PathBuf>,
    ) -> anyhow::Result<DraftState> {
        let files = tokio::task::spawn_blocking(move || read_files(paths)).await??;
        self.run(move |c| {
            let tx = c.transaction()?;
            anyhow::ensure!(!sent(&tx, &draft)?, "This draft was already sent; files were not attached.");
            let existing = attachments(&tx, &draft.id)?;
            anyhow::ensure!(existing.len() + files.len() <= MAX_ATTACHMENTS, "Attach at most 32 files to one message.");
            let size: usize = existing.iter().map(|file| file.size).chain(files.iter().map(|file| file.bytes.len())).sum();
            anyhow::ensure!(size <= MAX_ATTACHMENT_BYTES, "Attachments must total 18 MiB or less.");
            let id = draft.id.clone();
            save(&tx, draft)?;
            for file in files {
                let info = file.attachment;
                tx.execute("INSERT INTO draft_attachments(id,draft,name,media_type,size,data) VALUES(?,?,?,?,?,?)",
                    params![info.id,id,info.name,info.media_type,info.size as i64,file.bytes])?;
            }
            changed(&tx)?;
            let state = snapshot(&tx)?;
            tx.commit()?;
            Ok(state)
        }).await
    }
    pub async fn remove_draft_file(&self, draft: String, id: String) -> anyhow::Result<DraftState> {
        self.run(move |c| {
            let tx = c.transaction()?;
            if tx.execute(
                "DELETE FROM draft_attachments WHERE draft=? AND id=?",
                params![draft, id],
            )? > 0
            {
                changed(&tx)?;
            }
            let state = snapshot(&tx)?;
            tx.commit()?;
            Ok(state)
        })
        .await
    }
    pub async fn draft_files(&self, draft: Draft) -> anyhow::Result<Vec<FilePart>> {
        self.run(move |c| {
            anyhow::ensure!(draft.attachments.len() <= MAX_ATTACHMENTS, "Too many attachments.");
            let mut files = Vec::new();
            let mut total = 0usize;
            for expected in draft.attachments {
                let (name, media_type, size, actual): (String,String,u32,u32) = c.query_row(
                    "SELECT name,media_type,size,length(data) FROM draft_attachments WHERE draft=? AND id=?",
                    params![draft.id,expected.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))
                    .context("An attachment is missing. Reopen the draft and attach it again.")?;
                total = total.checked_add(size as usize).context("Attachment size overflow")?;
                anyhow::ensure!(actual == size && total <= MAX_ATTACHMENT_BYTES && name == expected.name && media_type == expected.media_type && size as usize == expected.size, "An attachment changed. Reopen the draft and check its files.");
                let bytes = c.query_row("SELECT data FROM draft_attachments WHERE draft=? AND id=?", params![draft.id,expected.id], |r| r.get(0))?;
                files.push(FilePart { attachment:expected, bytes });
            }
            Ok(files)
        }).await
    }
}

fn read_files(paths: Vec<PathBuf>) -> anyhow::Result<Vec<FilePart>> {
    anyhow::ensure!(
        !paths.is_empty() && paths.len() <= MAX_ATTACHMENTS,
        "Choose between 1 and 32 files."
    );
    let mut total = 0usize;
    let mut files = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .context("Choose a file to attach.")?
            .to_string_lossy()
            .into_owned();
        anyhow::ensure!(
            !name.chars().any(char::is_control),
            "Rename the attachment to remove control characters from its filename."
        );
        let metadata = std::fs::metadata(&path)
            .context("An attachment could not be opened. Check the file and try again.")?;
        anyhow::ensure!(metadata.is_file(), "Choose regular files as attachments.");
        anyhow::ensure!(
            metadata.len() <= (MAX_ATTACHMENT_BYTES - total) as u64,
            "Attachments must total 18 MiB or less."
        );
        let file = std::fs::File::open(&path)
            .context("An attachment could not be opened. Check its permissions and try again.")?;
        anyhow::ensure!(
            file.metadata()?.is_file(),
            "Choose regular files as attachments."
        );
        let mut bytes = Vec::new();
        file.take((MAX_ATTACHMENT_BYTES - total + 1) as u64)
            .read_to_end(&mut bytes)?;
        total += bytes.len();
        anyhow::ensure!(
            total <= MAX_ATTACHMENT_BYTES,
            "Attachments must total 18 MiB or less."
        );
        files.push(FilePart {
            attachment: DraftAttachment {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                media_type: mime_guess::from_path(path)
                    .first_or_octet_stream()
                    .to_string(),
                size: bytes.len(),
            },
            bytes,
        });
    }
    Ok(files)
}
