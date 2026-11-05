//! A fictional unread message with one attachment of each preview kind, and an
//! unread quote just below it for preparing nearby previews.
use crate::{model::parse_mail, store::Store};
use base64::Engine as _;

pub const SUBJECT: &str = "Site visit photos and notes";

pub async fn seed(store: &Store) -> anyhow::Result<()> {
    let photo = tokio::task::spawn_blocking(photo).await??;
    let csv = "Room,Width (m),Length (m)\nKitchen,3.2,4.1\nStudio,4.8,5.6\nHall,1.2,6.0\n";
    let office = "application/vnd.openxmlformats-officedocument";
    let site = message(
        "attachments-0",
        "Robin Hale <robin@example.test>",
        SUBJECT,
        "Here are the photos and measurements from this morning.",
        900,
        vec![
            ("site-photo.png", "image/png".into(), photo),
            (
                "measurements.csv",
                "text/csv".into(),
                csv.as_bytes().to_vec(),
            ),
            (
                "floor-plan.pdf",
                "application/pdf".into(),
                crate::document_files::pdf(&["Ground floor plan", "First floor plan"], ""),
            ),
            (
                "room-schedule.xlsx",
                format!("{office}.spreadsheetml.sheet"),
                crate::document_files::xlsx(&[]),
            ),
            (
                "survey-notes.docx",
                format!("{office}.wordprocessingml.document"),
                crate::document_files::docx(&[]),
            ),
            (
                "budget-tool.docx",
                format!("{office}.wordprocessingml.document"),
                crate::document_files::docx(&[("word/vbaProject.bin", "fixture macro")]),
            ),
            (
                "install.sh",
                "application/x-sh".into(),
                b"#!/bin/sh\necho fixture\n".to_vec(),
            ),
        ],
    )?;
    let quote = message(
        "attachments-1",
        "Jordan Avery <jordan@example.test>",
        "Kitchen quote",
        "The quote for the kitchen work is attached.",
        600,
        vec![(
            "kitchen-quote.pdf",
            "application/pdf".into(),
            crate::document_files::pdf(&["Kitchen quote", "Terms"], ""),
        )],
    )?;
    store.upsert(vec![site, quote]).await
}

/// An unread message `ahead` seconds newer than now, so it sorts first.
fn message(
    remote: &str,
    from: &str,
    subject: &str,
    body: &str,
    ahead: i64,
    files: Vec<(&str, String, Vec<u8>)>,
) -> anyhow::Result<crate::model::StoredMail> {
    let mut raw = format!(
        "From: {from}\r\nTo: alex@studio.example\r\nSubject: {subject}\r\nMessage-ID: <{remote}@example.test>\r\nDate: {}\r\nContent-Type: multipart/mixed; boundary=files\r\n\r\n--files\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}\r\n",
        chrono::Utc::now().to_rfc2822()
    );
    for (name, media, bytes) in files {
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(bytes)
            .as_bytes()
            .chunks(76)
            .map(|line| String::from_utf8_lossy(line).into_owned())
            .collect::<Vec<_>>()
            .join("\r\n");
        raw.push_str(&format!(
            "--files\r\nContent-Type: {media}\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{encoded}\r\n"
        ));
    }
    raw.push_str("--files--\r\n");
    let mut message = parse_mail(
        "preview-work",
        remote,
        "INBOX",
        raw.into_bytes(),
        true,
        false,
    )?;
    message.summary.timestamp = chrono::Utc::now().timestamp() + ahead;
    Ok(message)
}

fn photo() -> anyhow::Result<Vec<u8>> {
    let image = image::RgbaImage::from_fn(640, 400, |x, y| {
        let sky = y < 240;
        let stripe = (x / 40 + y / 40) % 2 == 0;
        match (sky, stripe) {
            (true, _) => image::Rgba([120, 170, 230, 255]),
            (false, true) => image::Rgba([90, 140, 80, 255]),
            (false, false) => image::Rgba([110, 160, 95, 255]),
        }
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image).write_to(
        &mut std::io::Cursor::new(&mut bytes),
        image::ImageFormat::Png,
    )?;
    Ok(bytes)
}
