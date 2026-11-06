//! Attachment preview and temporary-copy commands through the real dispatcher.
use super::*;

struct Running(tokio::task::JoinHandle<()>);
impl Drop for Running {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test]
async fn preview_and_open_run_beside_saturated_provider_work() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let mut engine = calendar_tests::engine();
    let raw = "Subject: Files\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
        --b\r\nContent-Type: text/plain\r\n\r\nBody\r\n\
        --b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=\"notes.txt\"\r\n\r\nMonday at nine.\r\n\
        --b\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"setup.exe\"\r\n\r\nMZ\r\n\
        --b--\r\n";
    let mail = parse_mail("test", "1", "INBOX", raw.into(), false, false)?;
    let id = mail.summary.id.clone();
    engine.store.upsert(vec![mail]).await?;
    engine.attachment_files = crate::attachment_files::Files::new(
        Some(directory.path().into()),
        Arc::new(crate::attachment_files::SystemLauncher),
    );
    assert!(engine.demo, "fixture engines never launch a desktop app");

    let (sender, input) = CommandSender::channel();
    let (output, mut events) = futures::channel::mpsc::channel(32);
    let _running = Running(tokio::spawn(engine.run(input, output)));
    let started = Arc::new(tokio::sync::Barrier::new(dispatch::NETWORK_CONCURRENCY + 1));
    let release = Arc::new(tokio::sync::Notify::new());
    for _ in 0..dispatch::NETWORK_CONCURRENCY {
        sender
            .try_send(Command::HoldBackend {
                started: started.clone(),
                release: release.clone(),
            })
            .map_err(|error| anyhow::anyhow!("{error}"))?;
    }
    tokio::time::timeout(crate::test_wait::HANG, started.wait()).await?;

    for command in [
        Command::PreviewAttachment(1, id.clone(), 0, 0),
        Command::OpenAttachment(2, id.clone(), 0),
        Command::OpenAttachment(3, id.clone(), 1),
        Command::PreviewAttachment(4, id.clone(), 9, 0),
        // Without attachments in the request, they are read from the cache.
        Command::PrepareAttachmentPreviews(id.clone(), None, vec![0, 1, 9]),
    ] {
        sender
            .try_send(command)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
    }
    let (mut preview, mut opened, mut refused, mut missing) = (None, None, None, None);
    let mut prepared = Vec::new();
    tokio::time::timeout(crate::test_wait::HANG, async {
        while preview.is_none()
            || opened.is_none()
            || refused.is_none()
            || missing.is_none()
            || prepared.len() < 3
        {
            match events.next().await.expect("dispatcher stopped") {
                Event::AttachmentPreview(1, result) => preview = Some(result),
                Event::AttachmentOpened(2, result) => opened = Some(result),
                Event::AttachmentOpened(3, result) => refused = Some(result),
                Event::AttachmentPreview(4, result) => missing = Some(result),
                Event::AttachmentPreviewPrepared(source, index, result) => {
                    assert_eq!(source, id);
                    prepared.push((index, result));
                }
                _ => {}
            }
        }
    })
    .await?;
    release.notify_waiters();

    // Prepared in order, one at a time; a missing attachment reports nothing.
    let order: Vec<_> = prepared.iter().map(|(index, _)| *index).collect();
    assert_eq!(order, [0, 1, 9]);
    assert_eq!(
        prepared[0].1.as_ref().map(|p| p.name.as_str()),
        Some("notes.txt")
    );
    assert!(prepared[1].1.as_ref().is_some_and(|p| !p.openable()));
    assert!(prepared[2].1.is_none());

    let preview = preview.and_then(Result::ok).expect("text preview");
    assert_eq!(preview.name, "notes.txt");
    assert!(matches!(
        &preview.content,
        crate::attachment_files::Content::Text { text, .. } if text == "Monday at nine."
    ));
    let path = opened.and_then(Result::ok).expect("temporary copy");
    assert!(path.starts_with(directory.path()));
    assert_eq!(std::fs::read(&path)?, b"Monday at nine.");
    assert!(refused.is_some_and(|result| result.is_err_and(|e| e.contains("Save a copy"))));
    assert!(missing.is_some_and(|result| result.is_err_and(|e| e.contains("no longer"))));
    Ok(())
}

#[tokio::test]
async fn download_all_writes_one_zip_and_never_overwrites() -> anyhow::Result<()> {
    let directory = tempfile::tempdir()?;
    let engine = calendar_tests::engine();
    let raw = "Subject: Files\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
        --b\r\nContent-Type: text/plain\r\n\r\nBody\r\n\
        --b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=\"notes.txt\"\r\n\r\nMonday at nine.\r\n\
        --b\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"setup.exe\"\r\n\r\nMZ\r\n\
        --b--\r\n";
    let mail = parse_mail("test", "1", "INBOX", raw.into(), false, false)?;
    let id = mail.summary.id.clone();
    engine.store.upsert(vec![mail]).await?;
    let (sender, input) = CommandSender::channel();
    let (output, mut events) = futures::channel::mpsc::channel(32);
    let _running = Running(tokio::spawn(engine.run(input, output)));
    let path = directory.path().join("Files attachments.zip");
    let path_text = path.to_string_lossy().into_owned();

    let mut next = async |command| -> anyhow::Result<Event> {
        sender
            .try_send(command)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        tokio::time::timeout(crate::test_wait::HANG, async {
            loop {
                if let event @ (Event::Notice(_) | Event::Error(_)) =
                    events.next().await.expect("dispatcher stopped")
                {
                    return event;
                }
            }
        })
        .await
        .map_err(Into::into)
    };
    let saved = next(Command::ExportAttachments(id.clone(), path_text.clone())).await?;
    assert!(matches!(saved, Event::Notice(text) if text.contains("2 attachments")));
    let mut archive = zip::ZipArchive::new(std::fs::File::open(&path)?)?;
    let names: Vec<_> = archive.file_names().collect::<Result<_, _>>()?;
    assert_eq!(names, ["notes.txt", "setup.exe"]);
    let mut notes = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("notes.txt")?, &mut notes)?;
    assert_eq!(notes, "Monday at nine.");

    let before = std::fs::read(&path)?;
    let repeated = next(Command::ExportAttachments(id, path_text)).await?;
    assert!(matches!(repeated, Event::Error(_)));
    assert_eq!(std::fs::read(&path)?, before);
    Ok(())
}
