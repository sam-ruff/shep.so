//! Draft text and file associations have independent ownership. All entry points
//! run on the database/background worker, never Dart's rendering isolate.
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};
use shep_mail_core::{
    compose::{FilePart, MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS},
    model::*,
};
use std::{io::Read, path::PathBuf};

#[derive(Deserialize)]
pub struct SelectedFile {
    pub path: PathBuf,
    pub name: String,
}

pub fn editable(db: &Connection, id: &str) -> Result<Draft> {
    let locked:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE draft_id=?1) OR EXISTS(SELECT 1 FROM discarded_drafts WHERE id=?1)",[id],|r|r.get(0))?;
    anyhow::ensure!(
        !locked,
        "This draft has been submitted or discarded. Its recovery record was preserved."
    );
    let text: String = db
        .query_row("SELECT content FROM drafts WHERE id=?1", [id], |r| r.get(0))
        .context("Save this draft before attaching files.")?;
    Ok(serde_json::from_str(&text)?)
}
pub fn attachments(db: &Connection, id: &str) -> Result<Vec<DraftAttachment>> {
    Ok(db.prepare("SELECT f.id,f.name,f.media_type,length(f.bytes),i.content_id FROM draft_files f LEFT JOIN draft_inline i ON i.file_id=f.id WHERE f.draft_id=?1 ORDER BY f.rowid")?
        .query_map([id],|r|Ok(DraftAttachment{content_id:r.get(4)?,id:r.get(0)?,name:r.get(1)?,media_type:r.get(2)?,size:r.get::<_,u32>(3)? as usize}))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}
pub fn snapshot(db: &Connection, id: &str) -> Result<Value> {
    let revision: i64 = db
        .query_row(
            "SELECT revision FROM draft_file_revisions WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok(json!({"attachments":attachments(db,id)?,"file_revision":revision}))
}
pub fn list(db: &Connection) -> Result<Value> {
    let mut rows = db.prepare("SELECT id,content FROM drafts ORDER BY rowid")?;
    let drafts = rows
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut values = Vec::new();
    for (id, text) in drafts {
        let mut value: Value = serde_json::from_str(&text)?;
        let files = snapshot(db, &id)?;
        value["attachments"] = files["attachments"].clone();
        value["file_revision"] = files["file_revision"].clone();
        values.push(value);
    }
    Ok(json!(values))
}
fn changed(db: &Connection, id: &str) -> Result<()> {
    db.execute("INSERT INTO draft_file_revisions VALUES(?1,1) ON CONFLICT(id) DO UPDATE SET revision=revision+1",[id])?;
    Ok(())
}
pub fn read_files(paths: Vec<SelectedFile>) -> Result<Vec<FilePart>> {
    anyhow::ensure!(
        !paths.is_empty() && paths.len() <= MAX_ATTACHMENTS,
        "Choose between 1 and 32 files."
    );
    let mut total = 0;
    let mut files = Vec::new();
    for selected in paths {
        anyhow::ensure!(
            selected.path.is_absolute()
                && !selected.name.is_empty()
                && selected.name.len() <= 1024
                && !selected.name.chars().any(char::is_control),
            "Choose a file with a valid name."
        );
        anyhow::ensure!(
            std::fs::metadata(&selected.path)
                .context("The selected file is unavailable. Choose it again.")?
                .is_file(),
            "Choose regular files as attachments."
        );
        let file = std::fs::File::open(&selected.path)
            .context("An attachment could not be opened. Choose the file again.")?;
        let metadata = file.metadata()?;
        anyhow::ensure!(metadata.is_file(), "Choose regular files as attachments.");
        anyhow::ensure!(
            metadata.len() <= (MAX_ATTACHMENT_BYTES - total) as u64,
            "Attachments must total 18 MiB or less."
        );
        let mut bytes = Vec::new();
        file.take((MAX_ATTACHMENT_BYTES - total + 1) as u64)
            .read_to_end(&mut bytes)?;
        total += bytes.len();
        anyhow::ensure!(
            total <= MAX_ATTACHMENT_BYTES,
            "Attachments must total 18 MiB or less."
        );
        let attachment = DraftAttachment {
            content_id: None,
            id: uuid::Uuid::new_v4().to_string(),
            media_type: mime_guess::from_path(&selected.name)
                .first_or_octet_stream()
                .to_string(),
            name: selected.name,
            size: bytes.len(),
        };
        files.push(FilePart { attachment, bytes });
    }
    Ok(files)
}
pub fn add(db: &mut Connection, id: &str, files: Vec<FilePart>) -> Result<Value> {
    let tx = db.transaction()?;
    editable(&tx, id)?;
    let existing = attachments(&tx, id)?;
    anyhow::ensure!(
        existing.len() + files.len() <= MAX_ATTACHMENTS,
        "Attach at most 32 files to one message."
    );
    let total: usize = existing
        .iter()
        .map(|a| a.size)
        .chain(files.iter().map(|a| a.bytes.len()))
        .sum();
    anyhow::ensure!(
        total <= MAX_ATTACHMENT_BYTES,
        "Attachments must total 18 MiB or less."
    );
    for file in files {
        insert_file(&tx, id, file)?;
    }
    changed(&tx, id)?;
    let result = snapshot(&tx, id)?;
    tx.commit()?;
    Ok(result)
}
pub fn remove(db: &mut Connection, id: &str, file: &str) -> Result<Value> {
    let tx = db.transaction()?;
    editable(&tx, id)?;
    if tx.execute(
        "DELETE FROM draft_files WHERE draft_id=?1 AND id=?2",
        params![id, file],
    )? > 0
    {
        changed(&tx, id)?;
    }
    let result = snapshot(&tx, id)?;
    tx.commit()?;
    Ok(result)
}
pub fn files(db: &Connection, draft: &mut Draft) -> Result<Vec<FilePart>> {
    draft.attachments = attachments(db, &draft.id)?;
    anyhow::ensure!(
        draft.attachments.len() <= MAX_ATTACHMENTS
            && draft.attachments.iter().map(|a| a.size).sum::<usize>() <= MAX_ATTACHMENT_BYTES,
        "The saved attachments exceed the sending limit."
    );
    draft
        .attachments
        .iter()
        .map(|a| {
            let bytes: Vec<u8> = db.query_row(
                "SELECT bytes FROM draft_files WHERE draft_id=?1 AND id=?2",
                params![draft.id, a.id],
                |r| r.get(0),
            )?;
            Ok(FilePart {
                attachment: a.clone(),
                bytes,
            })
        })
        .collect()
}

pub fn insert_file(db: &Connection, id: &str, file: FilePart) -> Result<()> {
    let a = file.attachment;
    db.execute(
        "INSERT INTO draft_files(id,draft_id,name,media_type,bytes) VALUES(?1,?2,?3,?4,?5)",
        params![a.id, id, a.name, a.media_type, file.bytes],
    )?;
    if let Some(cid) = a.content_id {
        db.execute(
            "INSERT INTO draft_inline(file_id,content_id) VALUES(?1,?2)",
            params![a.id, cid],
        )?;
    }
    Ok(())
}

pub fn value(db: &Connection, id: &str) -> Result<Value> {
    let mut value = serde_json::to_value(editable(db, id)?)?;
    let files = snapshot(db, id)?;
    value["attachments"] = files["attachments"].clone();
    value["file_revision"] = files["file_revision"].clone();
    Ok(value)
}

/// Lost acknowledgments retry the same draft identity without replacing newer
/// text/files or depending on the source still being present in the cache.
pub fn forwarded(db: &Connection, id: &str, source: &str) -> Result<Option<Value>> {
    let saved: Option<String> = db
        .query_row(
            "SELECT source_id FROM draft_forwards WHERE draft_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(saved) = saved {
        anyhow::ensure!(
            saved == source,
            "This forward belongs to another message. Open it from Drafts."
        );
        return Ok(Some(value(db, id)?));
    }
    let used: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM drafts WHERE id=?1) OR EXISTS(SELECT 1 FROM outgoing WHERE draft_id=?1) OR EXISTS(SELECT 1 FROM discarded_drafts WHERE id=?1)", [id], |r|r.get(0))?;
    anyhow::ensure!(
        !used,
        "This draft has already been used. Open Drafts or review Outbox before forwarding again."
    );
    Ok(None)
}

pub fn create_forward(
    db: &mut Connection,
    source: &str,
    draft: Draft,
    files: Vec<FilePart>,
) -> Result<Value> {
    let tx = db.transaction()?;
    if let Some(saved) = forwarded(&tx, &draft.id, source)? {
        return Ok(saved);
    }
    let connected: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1)",
        [&draft.account_id],
        |r| r.get(0),
    )?;
    anyhow::ensure!(
        connected,
        "This account was removed. Forward from a connected account."
    );
    tx.execute(
        "INSERT INTO drafts(id,revision,content) VALUES(?1,?2,?3)",
        params![
            draft.id,
            crate::operations::positive_revision(draft.revision)?,
            serde_json::to_string(&draft)?
        ],
    )?;
    for file in files {
        insert_file(&tx, &draft.id, file)?;
    }
    changed(&tx, &draft.id)?;
    tx.execute(
        "INSERT INTO draft_forwards(draft_id,source_id) VALUES(?1,?2)",
        params![draft.id, source],
    )?;
    let result = value(&tx, &draft.id)?;
    tx.commit()?;
    Ok(result)
}

pub fn save_text(db: &mut Connection, mut draft: Draft) -> Result<()> {
    let revision = crate::operations::positive_revision(draft.revision)?;
    let tx = db.transaction()?;
    let locked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE draft_id=?1) OR EXISTS(SELECT 1 FROM discarded_drafts WHERE id=?1)",[&draft.id],|r|r.get(0))?;
    anyhow::ensure!(
        !locked,
        "This draft has been submitted or discarded. Its delivery/discard record was preserved."
    );
    let current: Option<String> = tx
        .query_row("SELECT content FROM drafts WHERE id=?1", [&draft.id], |r| {
            r.get(0)
        })
        .optional()?;
    // Creation owns the immutable original quote. Text autosave can change its
    // displayed body but cannot replace or erase the hidden formatting source.
    draft.forward = current
        .map(|s| serde_json::from_str::<Draft>(&s))
        .transpose()?
        .and_then(|d| d.forward);
    draft.attachments = attachments(&tx, &draft.id)?;
    tx.execute("INSERT INTO drafts(id,revision,content) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,content=excluded.content WHERE excluded.revision>=drafts.revision",params![draft.id,revision,serde_json::to_string(&draft)?])?;
    tx.commit()?;
    Ok(())
}
