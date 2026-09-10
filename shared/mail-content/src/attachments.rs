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
    let parsed = crate::mime::parse(raw)?;
    decoded_parts(&parsed, &mut accept)
}

pub fn decoded_parts(
    parsed: &mailparse::ParsedMail<'_>,
    mut accept: impl FnMut(AttachmentInfo, Vec<u8>),
) -> Result<()> {
    crate::mime::validate_tree(parsed)?;
    let mut decoded = 0usize;
    for (index, part) in parts(parsed).enumerate() {
        let disposition = part.get_content_disposition();
        let name = disposition
            .params
            .get("filename")
            .or_else(|| part.ctype.params.get("name"));
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
    }
    Ok(())
}

/// Metadata-only traversal; explicit downloads separately validate every body.
/// Callers must pass a tree produced by `mime::parse` for untrusted raw input.
pub fn parts<'a, 'raw>(
    root: &'a mailparse::ParsedMail<'raw>,
) -> impl Iterator<Item = &'a mailparse::ParsedMail<'raw>> {
    let mut stack = vec![root];
    std::iter::from_fn(move || {
        while let Some(part) = stack.pop() {
            if crate::reader::is_attachment(part) {
                return Some(part);
            }
            stack.extend(part.subparts.iter().rev());
        }
        None
    })
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
