use crate::store::Store;
use std::io::Write;

pub async fn seed(store: &Store) -> anyhow::Result<()> {
    for attachment in [true, false] {
        let prepared = tokio::task::spawn_blocking(move || {
            let mut source = tempfile::NamedTempFile::new()?;
            let (headers, remote) = if attachment {
                ("From: Large mail fixture <large@example.test>\r\nSubject: Large incoming attachment\r\nContent-Type: multipart/mixed; boundary=part\r\n\r\n--part\r\nContent-Type: text/html\r\n\r\n<html><body><h1>Large incoming message</h1><p>This message has a complete 26 MiB attachment.</p><p>The body is readable, and the mail can be moved or deleted.</p></body></html>\r\n--part\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=large-fixture.bin\r\n\r\n", "1.9001")
            } else {
                ("From: Large mail fixture <large@example.test>\r\nSubject: Large incoming plain text\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n", "1.9002")
            };
            source.write_all(headers.as_bytes())?;
            let block = if attachment { vec![b'x';8192] } else {
                b"Large incoming plain text remains readable. More text follows in the next reader page.\r\n".repeat(96)
            };
            let mut remaining = 26 * 1024 * 1024;
            while remaining != 0 {
                let length = remaining.min(block.len());
                source.write_all(&block[..length])?;
                remaining -= length;
            }
            if attachment { source.write_all(b"\r\n--part--\r\n")?; }
            shep_mail_core::providers::mail::staging::prepare(source, "preview-work", remote, "INBOX", true, false)
        }).await??;
        store.sync_staged_message(prepared).await?;
    }
    Ok(())
}
