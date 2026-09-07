//! Fictional letters and mixed-background thread for native reading-style checks.
use crate::{model::parse_mail, store::Store};

pub async fn seed(store: &Store) -> anyhow::Result<()> {
    let plain = "Column marker\n\nHello Alex,\n\nThis is a simple letter. Comfortable margins and a readable line length make longer correspondence easier to read. Resizing the window should keep every word selectable and let us find it again.\n\nThanks,\nMorgan";
    let html = "<p style='margin:0'>Column marker</p><p>Hello Alex,</p><p>This is a simple letter. Comfortable margins and a readable line length make longer correspondence easier to read. Resizing the window should keep every word selectable and let us find it again.</p><p>Thanks,<br>Morgan</p>";
    for (index, subject, folder, mime, body, references) in [
        (
            0,
            "Reading style plain letter",
            "INBOX",
            "text/plain",
            plain.to_owned(),
            "",
        ),
        (
            1,
            "Reading style HTML letter",
            "INBOX",
            "text/html",
            format!("<body style='background:#fff;color:#18181b;font-family:Arial'>{html}</body>"),
            "",
        ),
        (
            2,
            "Reading style conversation",
            "Archive",
            "text/html",
            format!("<body style='background:#172a3a;color:#f4f4f5'>{html}</body>"),
            "",
        ),
        (
            3,
            "Re: Reading style conversation",
            "INBOX",
            "text/html",
            format!("<body style='background:#fff;color:#18181b'>{html}</body>"),
            "References: <reading-2@example.test>\r\nIn-Reply-To: <reading-2@example.test>\r\n",
        ),
    ] {
        let raw = format!(
            "From: Morgan <morgan@example.test>\r\nTo: alex@studio.example\r\nSubject: {subject}\r\nMessage-ID: <reading-{index}@example.test>\r\n{references}Content-Type: {mime}; charset=utf-8\r\n\r\n{body}"
        );
        let mut message = parse_mail(
            "preview-work",
            &format!("reading-{index}"),
            folder,
            raw.into_bytes(),
            true,
            false,
        )?;
        message.summary.timestamp = chrono::Utc::now().timestamp()
            + match index {
                0 => 600,
                1 => 500,
                2 => 300,
                _ => 400,
            };
        store.upsert(vec![message]).await?;
    }
    Ok(())
}
