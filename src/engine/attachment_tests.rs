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
    ] {
        sender
            .try_send(command)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
    }
    let (mut preview, mut opened, mut refused, mut missing) = (None, None, None, None);
    tokio::time::timeout(crate::test_wait::HANG, async {
        while preview.is_none() || opened.is_none() || refused.is_none() || missing.is_none() {
            match events.next().await.expect("dispatcher stopped") {
                Event::AttachmentPreview(1, result) => preview = Some(result),
                Event::AttachmentOpened(2, result) => opened = Some(result),
                Event::AttachmentOpened(3, result) => refused = Some(result),
                Event::AttachmentPreview(4, result) => missing = Some(result),
                _ => {}
            }
        }
    })
    .await?;
    release.notify_waiters();

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
