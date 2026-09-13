//! Private, immutable raw-source staging for native incoming mail.
use crate::model::Mail;
use anyhow::{Context, Result};
use mailparse::{MailHeaderMap, ParsedMail};
use std::io::{Seek, SeekFrom, Write};

const PREVIEW_BYTES: usize = 128 * 1024;
const PREVIEW_TOTAL: usize = 1024 * 1024;

#[derive(Debug)]
pub struct Message {
    pub summary: Mail,
    pub text: String,
    pub raw_hash: String,
    pub header_prefix: Vec<u8>,
    source: tempfile::NamedTempFile,
    bytes: u64,
}

impl Message {
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    pub fn copy_to(&mut self, destination: &mut impl Write) -> Result<()> {
        self.source.as_file_mut().seek(SeekFrom::Start(0))?;
        let copied = std::io::copy(self.source.as_file_mut(), destination)?;
        anyhow::ensure!(
            copied == self.bytes,
            "The staged message changed before it was saved."
        );
        Ok(())
    }
}

pub fn prepare(
    source: tempfile::NamedTempFile,
    account: &str,
    remote: &str,
    folder: &str,
    unread: bool,
    starred: bool,
) -> Result<Message> {
    use sha2::{Digest, Sha256};
    let bytes = source.as_file().metadata()?.len();
    // The staging writer has closed before this function owns the private file.
    // No other handle is published, so its size and contents stay immutable.
    let mapped = unsafe { memmap2::MmapOptions::new().map(source.as_file()) }?;
    let mut parsed = shep_mail_content::mime::parse_paged(&mapped)?;
    let subject = parsed
        .headers
        .get_first_value("Subject")
        .unwrap_or_else(|| "(No subject)".into());
    let sender = parsed.headers.get_first_value("From").unwrap_or_default();
    let recipient = parsed.headers.get_first_value("To").unwrap_or_default();
    let timestamp = parsed
        .headers
        .get_first_value("Date")
        .and_then(|value| mailparse::dateparse(&value).ok())
        .unwrap_or_else(|| chrono::Utc::now().timestamp());
    let attachment_count = shep_mail_content::attachments::parts(&parsed).count();
    let mut remaining = PREVIEW_TOTAL;
    bound_preview(&mut parsed, &mut remaining, PREVIEW_BYTES)?;
    let text = shep_mail_content::reader::text(&parsed)?;
    let preview = text
        .split_whitespace()
        .flat_map(|word| word.chars().chain(std::iter::once(' ')))
        .take(180)
        .collect::<String>()
        .trim_end()
        .to_owned();
    let summary = Mail {
        id: format!("{account}:{folder}:{remote}"),
        account_id: account.into(),
        remote_id: remote.into(),
        folder: folder.into(),
        sender,
        recipient,
        subject,
        timestamp,
        unread,
        starred,
        attachment_count,
        preview,
    };
    let raw_hash = format!("{:x}", Sha256::digest(&mapped));
    let header_prefix = mapped[..mapped.len().min(64 * 1024)].to_vec();
    drop(parsed);
    drop(mapped);
    Ok(Message {
        summary,
        text,
        raw_hash,
        header_prefix,
        source,
        bytes,
    })
}

pub fn bound_reader_preview(part: &mut ParsedMail<'_>, characters: usize) -> Result<bool> {
    let bytes = characters.saturating_mul(8);
    bound_preview(part, &mut bytes.max(PREVIEW_TOTAL), bytes)
}

fn bound_preview<'a>(
    part: &mut ParsedMail<'a>,
    remaining: &mut usize,
    per_part: usize,
) -> Result<bool> {
    if shep_mail_content::reader::is_attachment(part) {
        return Ok(false);
    }
    if !part.subparts.is_empty() {
        let mut truncated = false;
        for child in &mut part.subparts {
            truncated |= bound_preview(child, remaining, per_part)?;
        }
        return Ok(truncated);
    }
    if !matches!(
        part.ctype.mimetype.as_str(),
        "text/plain" | "text/html" | "application/xhtml+xml"
    ) {
        return Ok(false);
    }
    let (_, headers) = mailparse::parse_headers(part.raw_bytes)?;
    let count = per_part.min(*remaining).min(part.raw_bytes.len() - headers);
    let mut end = headers + count;
    if end < part.raw_bytes.len() && count != 0 {
        let encoding = part
            .headers
            .get_first_value("Content-Transfer-Encoding")
            .unwrap_or_default();
        if encoding.eq_ignore_ascii_case("base64") {
            // MIME base64 may be one long line. Retain complete quanta while
            // preserving whitespace, which does not count towards a quantum.
            let mut significant = 0;
            end = headers;
            for (index, byte) in part.raw_bytes[headers..headers + count].iter().enumerate() {
                if !byte.is_ascii_whitespace() {
                    significant += 1;
                }
                if significant % 4 == 0 {
                    end = headers + index + 1;
                }
            }
        } else if encoding.eq_ignore_ascii_case("quoted-printable")
            && let Some(index) = part.raw_bytes[headers..end]
                .iter()
                .rposition(|byte| *byte == b'\n')
        {
            end = headers + index + 1;
        }
    }
    *remaining -= count;
    let truncated = end < part.raw_bytes.len();
    *part = shep_mail_content::mime::parse_paged(&part.raw_bytes[..end])
        .context("Could not prepare the message preview.")?;
    Ok(truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn complete_large_attachment_is_staged_with_readable_body_and_exact_original() -> Result<()> {
        let mut source = tempfile::NamedTempFile::new()?;
        source.write_all(b"From: sender@example.test\r\nSubject: Large attachment\r\nContent-Type: multipart/mixed; boundary=part\r\n\r\n--part\r\nContent-Type: text/plain\r\n\r\nThe complete short body.\r\n--part\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=large.bin\r\n\r\n")?;
        let block = [0x5a; 8192];
        for _ in 0..26 * 1024 * 1024 / block.len() {
            source.write_all(&block)?;
        }
        source.write_all(b"\r\n--part--\r\n")?;
        let bytes = source.as_file().metadata()?.len();
        let mut prepared = prepare(source, "account", "42.7", "INBOX", true, false)?;
        assert_eq!(prepared.summary.subject, "Large attachment");
        assert_eq!(prepared.summary.attachment_count, 1);
        assert_eq!(prepared.text, "The complete short body.");
        assert_eq!(prepared.bytes(), bytes);
        let mut copied = tempfile::NamedTempFile::new()?;
        prepared.copy_to(copied.as_file_mut())?;
        assert_eq!(copied.as_file().metadata()?.len(), bytes);
        copied.seek(SeekFrom::End(-13))?;
        let mut ending = String::new();
        copied.read_to_string(&mut ending)?;
        assert!(ending.ends_with("--part--\r\n"));
        Ok(())
    }

    #[test]
    fn reader_preview_accepts_encoded_source_above_the_old_raw_limit() -> Result<()> {
        let mut raw =
            b"Content-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\n".to_vec();
        raw.extend(b"eHh4".repeat(26 * 1024 * 1024 / 4));
        let mut parsed = shep_mail_content::mime::parse_paged(&raw)?;
        assert!(!bound_reader_preview(&mut parsed, 4 * 1024 * 1024)?);
        let text = shep_mail_content::reader::text(&parsed)?;
        assert_eq!(text.len(), 26 * 1024 * 1024 / 4 * 3);
        assert!(text.bytes().all(|byte| byte == b'x'));
        Ok(())
    }

    #[test]
    fn large_plain_body_keeps_complete_source_and_bounded_preview() -> Result<()> {
        let mut source = tempfile::NamedTempFile::new()?;
        source.write_all(b"Subject: Large plain body\r\n\r\n")?;
        let block = [b'x'; 8192];
        for _ in 0..26 * 1024 * 1024 / block.len() {
            source.write_all(&block)?;
        }
        let prepared = prepare(source, "account", "42.7", "INBOX", true, false)?;
        assert!(prepared.bytes() > crate::model::MAX_MESSAGE_BYTES as u64);
        assert_eq!(prepared.text.len(), PREVIEW_BYTES);
        assert_eq!(prepared.summary.attachment_count, 0);
        Ok(())
    }
}
