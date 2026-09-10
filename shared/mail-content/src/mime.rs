//! Check nesting without recursion before handing bytes to mailparse. This is
//! deliberately coupled to the pinned 0.16.1 multipart traversal (including its
//! permissive boundary-prefix and missing-closing-boundary behavior).
use anyhow::{Context, Result, ensure};
use mailparse::{MailHeaderMap, ParsedMail};

pub const MAX_DEPTH: usize = 128;

pub fn parse(raw: &[u8]) -> Result<ParsedMail<'_>> {
    ensure!(
        raw.len() <= crate::MAX_MESSAGE_BYTES,
        "This message exceeds the current 25 MiB limit."
    );
    validate(raw)?;
    mailparse::parse_mail(raw)
        .map_err(|_| anyhow::anyhow!("Could not read this cached message. Refresh it and retry."))
}

/// Each stack frame is a borrowed slice plus the remaining siblings cursor. A
/// wide multipart does not allocate a second vector containing every child.
fn validate(raw: &[u8]) -> Result<()> {
    let mut stack = vec![(raw, 0, None)];
    while let Some((raw, depth, cursor)) = stack.pop() {
        ensure!(
            depth <= MAX_DEPTH,
            "This message has too many nested MIME parts to read safely."
        );
        let mut cursor = match cursor {
            Some(cursor) => cursor,
            None => {
                let (headers, start) = mailparse::parse_headers(raw)
                    .map_err(|_| anyhow::anyhow!("Could not read this message's MIME headers."))?;
                let Some(value) = headers.get_first_value("Content-Type") else {
                    continue;
                };
                let ctype = mailparse::parse_content_type(&value);
                if !ctype.mimetype.starts_with("multipart/") || start >= raw.len() {
                    continue;
                }
                let Some(boundary) = ctype.params.get("boundary") else {
                    continue;
                };
                let boundary = format!("--{boundary}").into_bytes();
                let Some(first) = line_prefix(raw, start, &boundary) else {
                    continue;
                };
                Cursor {
                    end: first + boundary.len(),
                    boundary,
                }
            }
        };
        let Some(start) = raw[cursor.end..]
            .iter()
            .position(|b| *b == b'\n')
            .map(|offset| cursor.end + offset + 1)
        else {
            continue;
        };
        let next = line_prefix(raw, start, &cursor.boundary);
        let end = next
            .map(|offset| strip_crlf(raw, start, offset))
            .unwrap_or(raw.len());
        cursor.end = next
            .map(|offset| offset + cursor.boundary.len())
            .unwrap_or(raw.len());
        let closed = cursor.end + 2 > raw.len() || &raw[cursor.end..cursor.end + 2] == b"--";
        if !closed {
            stack.push((raw, depth, Some(cursor)));
        }
        stack.push((&raw[start..end], depth + 1, None));
    }
    Ok(())
}

struct Cursor {
    end: usize,
    boundary: Vec<u8>,
}
fn line_prefix(raw: &[u8], start: usize, boundary: &[u8]) -> Option<usize> {
    raw[start..]
        .windows(boundary.len())
        .enumerate()
        .find_map(|(offset, bytes)| {
            (bytes == boundary && (offset == 0 || raw[start + offset - 1] == b'\n'))
                .then_some(start + offset)
        })
}
fn strip_crlf(raw: &[u8], start: usize, mut end: usize) -> usize {
    if end > start && raw[end - 1] == b'\n' {
        end -= 1;
        if end > start && raw[end - 1] == b'\r' {
            end -= 1;
        }
    }
    end
}

/// MIME body extraction also accepts already parsed native messages; reject a
/// deep tree before its own recursive selection. Raw callers must use `parse`.
pub(crate) fn validate_tree(root: &ParsedMail<'_>) -> Result<()> {
    let mut stack = vec![(std::slice::from_ref(root).iter(), 0)];
    while let Some((children, depth)) = stack.last_mut() {
        if let Some(part) = children.next() {
            ensure!(
                *depth <= MAX_DEPTH,
                "This message has too many nested MIME parts to read safely."
            );
            if !part.subparts.is_empty() {
                let depth = *depth + 1;
                stack.push((part.subparts.iter(), depth));
            }
        } else {
            stack.pop();
        }
    }
    Ok(())
}

pub(crate) fn decoded(total: &mut usize, amount: usize) -> Result<()> {
    *total = total
        .checked_add(amount)
        .context("The decoded message is too large.")?;
    ensure!(
        *total <= crate::MAX_MESSAGE_BYTES,
        "The decoded message exceeds the current 25 MiB limit."
    );
    Ok(())
}
