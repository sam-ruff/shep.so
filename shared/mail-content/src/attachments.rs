//! Explicit attachment downloads from cached MIME; no network or filesystem work.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentInfo {
    pub id: String,
    pub name: String,
    pub media_type: String,
    pub size: usize,
}

/// A suggested filename is never a path, even when the sender supplies one.
pub fn filename(value: &str) -> String {
    let name = value.rsplit(['/', '\\']).next().unwrap_or("");
    let name: String = name
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(180)
        .collect();
    let name = name.trim().trim_matches('.').trim();
    if name.is_empty() {
        "attachment.bin".into()
    } else {
        name.into()
    }
}

fn decoded(raw: &[u8], mut accept: impl FnMut(AttachmentInfo, Vec<u8>)) -> Result<()> {
    anyhow::ensure!(
        raw.len() <= crate::MAX_MESSAGE_BYTES,
        "This message exceeds the current 25 MiB limit."
    );
    let parsed = mailparse::parse_mail(raw)
        .context("Could not read this cached message. Refresh it and retry.")?;
    let mut stack = vec![(&parsed, 0)];
    let mut index = 0;
    let mut decoded = 0usize;
    while let Some((part, depth)) = stack.pop() {
        anyhow::ensure!(
            depth <= 128,
            "This message has too many nested MIME parts to save safely."
        );
        let disposition = part.get_content_disposition();
        let name = disposition
            .params
            .get("filename")
            .or_else(|| part.ctype.params.get("name"));
        let is_file =
            disposition.disposition == mailparse::DispositionType::Attachment || name.is_some();
        if is_file {
            let bytes = part.get_body_raw().map_err(|_| {
                anyhow::anyhow!("Could not decode an attachment. Refresh this message and retry.")
            })?;
            decoded = decoded
                .checked_add(bytes.len())
                .context("The decoded attachments are too large.")?;
            anyhow::ensure!(
                decoded <= crate::MAX_MESSAGE_BYTES,
                "The decoded attachments exceed the current 25 MiB limit."
            );
            let info = AttachmentInfo {
                id: format!("{index}.{:x}", Sha256::digest(&bytes)),
                name: filename(name.map(String::as_str).unwrap_or("attachment.bin")),
                media_type: part.ctype.mimetype.clone(),
                size: bytes.len(),
            };
            accept(info, bytes);
            index += 1;
        } else {
            stack.extend(part.subparts.iter().rev().map(|p| (p, depth + 1)));
        }
    }
    Ok(())
}

pub fn catalog(raw: &[u8]) -> Result<Vec<AttachmentInfo>> {
    let mut infos = Vec::new();
    decoded(raw, |info, _| infos.push(info))?;
    Ok(infos)
}

pub fn read(raw: &[u8], id: &str) -> Result<(AttachmentInfo, Vec<u8>)> {
    let mut found = None;
    decoded(raw, |info, bytes| {
        if info.id == id {
            found = Some((info, bytes));
        }
    })?;
    found.context("This attachment changed or is no longer cached. Reopen the message and retry.")
}
