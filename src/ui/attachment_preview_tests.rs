//! Result ordering for the attachment preview dialog. Preparation itself is
//! covered in `attachment_files`; native input is covered by the MCP suite.
use super::attachment_preview::Message as Preview;
use super::*;
use crate::attachment_files::{Content, Preview as Prepared};
use tokio::sync::mpsc::Receiver;

async fn reader() -> anyhow::Result<(App, Receiver<Command>, Receiver<Command>, String)> {
    let (app, attachments, network, _, id) = reader_lanes().await?;
    Ok((app, attachments, network, id))
}

/// The reader plus the speculative preparation lane.
async fn reader_lanes() -> anyhow::Result<(
    App,
    Receiver<Command>,
    Receiver<Command>,
    Receiver<Command>,
    String,
)> {
    let store = crate::store::Store::memory()?;
    let raw = "Subject: Files\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n\
        --b\r\nContent-Type: text/plain\r\n\r\nBody\r\n\
        --b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=\"notes.txt\"\r\n\r\nMonday at nine.\r\n\
        --b\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"setup.exe\"\r\n\r\nMZ\r\n\
        --b--\r\n";
    let mail = crate::model::parse_mail("test", "1", "INBOX", raw.into(), false, false)?;
    let id = mail.summary.id.clone();
    store.upsert(vec![mail]).await?;
    let (mut app, _) = App::new();
    let (sender, attachments, network, preparing) =
        engine::CommandSender::attachment_test_channels();
    app.tx = Some(sender);
    app.selected = Some(id.clone());
    app.detail = Some(Arc::new(store.detail(id.clone()).await?));
    Ok((app, attachments, network, preparing, id))
}

type PrepareRequest = (
    String,
    Option<Arc<Vec<crate::model::Attachment>>>,
    Vec<usize>,
);

fn prepare_request(commands: &mut Receiver<Command>) -> Option<PrepareRequest> {
    match commands.try_recv().ok()? {
        Command::PrepareAttachmentPreviews(source, attachments, indices) => {
            Some((source, attachments, indices))
        }
        _ => None,
    }
}

fn prepared(name: &str, openable: bool) -> Arc<Prepared> {
    Arc::new(Prepared {
        name: name.into(),
        size: 15,
        kind: "Text",
        content: Content::Text {
            text: "Monday at nine.".into(),
            truncated: false,
        },
        refusal: (!openable).then_some(crate::attachment_files::Refusal::Program),
    })
}

fn pdf_page(page: usize, pages: usize) -> Arc<Prepared> {
    Arc::new(Prepared {
        name: "plan.pdf".into(),
        size: 900,
        kind: "PDF document",
        content: Content::Pages {
            handle: iced::widget::image::Handle::from_rgba(1, 1, vec![255; 4]),
            width: 1,
            height: 1,
            page,
            pages,
        },
        refusal: None,
    })
}

fn preview_request(commands: &mut Receiver<Command>) -> Option<(u64, String, usize)> {
    page_request(commands).map(|(request, id, index, _)| (request, id, index))
}

fn page_request(commands: &mut Receiver<Command>) -> Option<(u64, String, usize, usize)> {
    match commands.try_recv().ok()? {
        Command::PreviewAttachment(request, id, index, page) => Some((request, id, index, page)),
        _ => None,
    }
}

#[tokio::test]
async fn preview_opens_for_the_clicked_attachment_and_ignores_older_results() -> anyhow::Result<()>
{
    let (mut app, mut attachments, _, id) = reader().await?;
    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id.clone(), 0)));
    assert_eq!(app.dialog, Some(Dialog::Attachment));
    assert_eq!(app.attachment_preview_title(), "notes.txt");
    let (first, source, index) = preview_request(&mut attachments).expect("first request");
    assert_eq!((source.as_str(), index), (id.as_str(), 0));

    let _ = app.handle(Message::AttachmentPreview(Preview::Step(true)));
    let (second, _, index) = preview_request(&mut attachments).expect("second request");
    assert_eq!(index, 1);
    assert_eq!(app.attachment_preview_title(), "setup.exe");

    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        first,
        Ok(prepared("notes.txt", true)),
    )));
    assert!(app.attachment_preview.preview.is_none());
    assert!(app.attachment_preview.loading);

    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        second,
        Ok(prepared("setup.exe", false)),
    )));
    assert_eq!(
        app.attachment_preview
            .preview
            .as_ref()
            .map(|p| p.name.as_str()),
        Some("setup.exe")
    );
    assert!(!app.attachment_preview.loading);

    // The last attachment has no next step.
    let _ = app.handle(Message::AttachmentPreview(Preview::Step(true)));
    assert!(preview_request(&mut attachments).is_none());
    Ok(())
}

#[tokio::test]
async fn a_result_after_closing_is_dropped() -> anyhow::Result<()> {
    let (mut app, mut attachments, _, id) = reader().await?;
    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id, 0)));
    let (request, ..) = preview_request(&mut attachments).expect("request");
    let _ = app.handle(Message::Close);
    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        request,
        Ok(prepared("notes.txt", true)),
    )));
    assert_eq!(app.dialog, None);
    assert!(app.attachment_preview.preview.is_none());
    assert!(app.attachment_preview.target.is_none());
    Ok(())
}

#[tokio::test]
async fn preview_errors_offer_retry() -> anyhow::Result<()> {
    let (mut app, mut attachments, _, id) = reader().await?;
    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id, 0)));
    let (request, ..) = preview_request(&mut attachments).expect("request");
    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        request,
        Err("Could not prepare the preview. Try again.".into()),
    )));
    assert!(app.attachment_preview.error.is_some());
    let _ = app.handle(Message::AttachmentPreview(Preview::Retry));
    let (retry, _, index) = preview_request(&mut attachments).expect("retry");
    assert!(retry > request);
    assert_eq!(index, 0);
    assert!(app.attachment_preview.error.is_none());
    Ok(())
}

#[tokio::test]
async fn open_uses_the_previewed_file_and_ignores_stale_replies() -> anyhow::Result<()> {
    let (mut app, mut attachments, _, id) = reader().await?;
    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id.clone(), 0)));
    let (request, ..) = preview_request(&mut attachments).expect("request");

    // Open does not wait for the preview; the engine checks the file itself.
    let _ = app.handle(Message::AttachmentPreview(Preview::Open));
    let Ok(Command::OpenAttachment(open, source, 0)) = attachments.try_recv() else {
        anyhow::bail!("expected an open request for the first attachment");
    };
    assert_eq!(source, id);
    assert!(app.attachment_preview.opening);
    assert!(
        app.attachment_preview.loading,
        "the preview keeps preparing"
    );

    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        request,
        Ok(prepared("notes.txt", true)),
    )));
    assert!(app.attachment_preview.opening);

    // A second click while opening is ignored.
    let _ = app.handle(Message::AttachmentPreview(Preview::Open));
    assert!(attachments.try_recv().is_err());

    let _ = app.handle(Message::Backend(Event::AttachmentOpened(
        open + 1,
        Ok("/tmp/other.txt".into()),
    )));
    assert!(app.attachment_preview.opening);

    let path = std::path::PathBuf::from("/tmp/shep-attachments-x/a/notes.txt");
    let _ = app.handle(Message::Backend(Event::AttachmentOpened(
        open,
        Ok(path.clone()),
    )));
    assert!(!app.attachment_preview.opening);
    assert_eq!(app.attachment_preview.opened, Some(path));
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|(message, error, _)| !error && message.contains("temporary copy"))
    );
    Ok(())
}

#[tokio::test]
async fn programs_cannot_be_opened_from_the_dialog() -> anyhow::Result<()> {
    let (mut app, mut attachments, _, id) = reader().await?;
    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id, 1)));
    let (request, ..) = preview_request(&mut attachments).expect("request");

    // Clicked before the preview is ready, the engine's refusal is shown.
    let _ = app.handle(Message::AttachmentPreview(Preview::Open));
    let Ok(Command::OpenAttachment(open, _, 1)) = attachments.try_recv() else {
        anyhow::bail!("expected an early open request");
    };
    let _ = app.handle(Message::Backend(Event::AttachmentOpened(
        open,
        Err("Shep does not open programs or scripts such as setup.exe.".into()),
    )));
    assert!(!app.attachment_preview.opening);
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|(message, error, _)| *error && message.contains("setup.exe"))
    );

    // Once the preview says so, the button no longer sends anything.
    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        request,
        Ok(prepared("setup.exe", false)),
    )));
    let _ = app.handle(Message::AttachmentPreview(Preview::Open));
    assert!(attachments.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn open_failures_stay_visible() -> anyhow::Result<()> {
    let (mut app, mut attachments, _, id) = reader().await?;
    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id, 0)));
    let (request, ..) = preview_request(&mut attachments).expect("request");
    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        request,
        Ok(prepared("notes.txt", true)),
    )));
    let _ = app.handle(Message::AttachmentPreview(Preview::Open));
    let Ok(Command::OpenAttachment(open, ..)) = attachments.try_recv() else {
        anyhow::bail!("expected an open request");
    };
    let _ = app.handle(Message::Backend(Event::AttachmentOpened(
        open,
        Err("Could not open the attachment with your default app.".into()),
    )));
    assert!(!app.attachment_preview.opening);
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|(message, error, _)| *error && message.contains("default app"))
    );
    Ok(())
}

#[tokio::test]
async fn save_exports_the_previewed_message_even_after_selection_changes() -> anyhow::Result<()> {
    let (mut app, mut attachments, mut network, id) = reader().await?;
    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id.clone(), 0)));
    let _ = preview_request(&mut attachments);
    let _ = app.handle(Message::AttachmentPreview(Preview::Save));
    assert_eq!(app.dialog, Some(Dialog::Export));
    assert!(app.field("path").ends_with("notes.txt"));
    assert!(app.attachment_preview.target.is_none());

    app.selected = Some("another-message".into());
    let _ = app.handle(Message::SaveExport);
    let Ok(Command::ExportAttachment(source, 0, path)) = network.try_recv() else {
        anyhow::bail!("expected an attachment export");
    };
    assert_eq!(source, id);
    assert!(path.ends_with("notes.txt"));
    Ok(())
}

#[tokio::test]
async fn download_all_saves_a_zip_of_the_reader_message() -> anyhow::Result<()> {
    let (mut app, _, mut network, id) = reader().await?;
    let _ = app.handle(Message::ExportAttachments(id.clone()));
    assert_eq!(app.dialog, Some(Dialog::Export));
    assert!(app.field("path").ends_with("Files attachments.zip"));

    app.selected = Some("another-message".into());
    let _ = app.handle(Message::SaveExport);
    let Ok(Command::ExportAttachments(source, path)) = network.try_recv() else {
        anyhow::bail!("expected a zip export");
    };
    assert_eq!(source, id);
    assert!(path.ends_with(".zip"));

    // An ordinary message export afterwards is not a zip.
    let _ = app.handle(Message::Open(Dialog::Export));
    let _ = app.handle(Message::SaveExport);
    assert!(matches!(network.try_recv(), Ok(Command::ExportMessage(..))));
    Ok(())
}

#[test]
fn zip_names_come_from_the_subject() {
    assert_eq!(
        zip_name("Re: Q3 / budget, final?"),
        "Re Q3 budget final attachments.zip"
    );
    assert_eq!(zip_name("  ::  "), "attachments.zip");
}

#[tokio::test]
async fn a_click_for_a_message_no_longer_in_the_reader_is_ignored() -> anyhow::Result<()> {
    let (mut app, mut attachments, ..) = reader().await?;
    let _ = app.handle(Message::AttachmentPreview(Preview::Show("gone".into(), 0)));
    assert_eq!(app.dialog, None);
    assert!(attachments.try_recv().is_err());
    Ok(())
}

#[tokio::test]
async fn pdf_pages_keep_the_shown_page_until_the_next_arrives() -> anyhow::Result<()> {
    let (mut app, mut attachments, _, id) = reader().await?;
    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id.clone(), 0)));
    let (first, ..) = page_request(&mut attachments).expect("first page");
    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        first,
        Ok(pdf_page(0, 3)),
    )));

    // Previous is unavailable on the first page.
    let _ = app.handle(Message::AttachmentPreview(Preview::Page(false)));
    assert!(page_request(&mut attachments).is_none());

    let _ = app.handle(Message::AttachmentPreview(Preview::Page(true)));
    let (second, source, index, page) = page_request(&mut attachments).expect("second page");
    assert_eq!((source.as_str(), index, page), (id.as_str(), 0, 1));
    assert!(app.attachment_preview.loading);
    assert!(
        app.attachment_preview.preview.is_some(),
        "page 1 stays visible"
    );

    // A late reply for the first page cannot replace the request in flight.
    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        first,
        Ok(pdf_page(0, 3)),
    )));
    assert!(app.attachment_preview.loading);
    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        second,
        Ok(pdf_page(1, 3)),
    )));
    assert!(!app.attachment_preview.loading);
    assert_eq!(app.attachment_preview.page, 1);

    // Moving to another attachment starts again at its first page.
    let _ = app.handle(Message::AttachmentPreview(Preview::Step(true)));
    let (_, _, index, page) = page_request(&mut attachments).expect("next attachment");
    assert_eq!((index, page), (1, 0));
    Ok(())
}

#[tokio::test]
async fn sheet_tabs_switch_locally_and_reset_for_another_file() -> anyhow::Result<()> {
    let (mut app, mut attachments, _, id) = reader().await?;
    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id, 0)));
    let (request, ..) = page_request(&mut attachments).expect("request");
    let sheet = |name: &str| crate::attachment_files::Sheet {
        name: name.into(),
        rows: vec![vec!["A".into()]],
        truncated: false,
    };
    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        request,
        Ok(Arc::new(Prepared {
            name: "rooms.xlsx".into(),
            size: 10,
            kind: "Excel spreadsheet",
            content: Content::Sheets(vec![sheet("Rooms"), sheet("Notes")]),
            refusal: None,
        })),
    )));
    let _ = app.handle(Message::AttachmentPreview(Preview::Sheet(1)));
    assert_eq!(app.attachment_preview.sheet, 1);
    assert!(
        page_request(&mut attachments).is_none(),
        "tabs need no backend work"
    );
    let _ = app.handle(Message::AttachmentPreview(Preview::Sheet(7)));
    assert_eq!(app.attachment_preview.sheet, 1);

    let _ = app.handle(Message::AttachmentPreview(Preview::Step(true)));
    assert_eq!(app.attachment_preview.sheet, 0);
    Ok(())
}

#[tokio::test]
async fn the_open_message_is_prepared_with_the_attachments_it_already_has() -> anyhow::Result<()> {
    let (mut app, _, _, mut preparing, id) = reader_lanes().await?;
    app.prepare_attachment_previews();
    let (source, attachments, indices) = prepare_request(&mut preparing).expect("prepare");
    assert_eq!(
        (source.as_str(), indices.as_slice()),
        (id.as_str(), [0, 1].as_slice())
    );
    let reader = app.detail.as_ref().map(|d| d.attachments.clone());
    assert!(
        attachments
            .zip(reader)
            .is_some_and(|(a, b)| Arc::ptr_eq(&a, &b)),
        "the reader's bytes are shared, not copied or read again"
    );

    // Nothing is requested twice while it is in flight.
    app.prepare_attachment_previews();
    assert!(prepare_request(&mut preparing).is_none());
    Ok(())
}

#[tokio::test]
async fn a_prepared_preview_opens_without_waiting() -> anyhow::Result<()> {
    let (mut app, mut attachments, _, mut preparing, id) = reader_lanes().await?;
    app.prepare_attachment_previews();
    let _ = prepare_request(&mut preparing);
    let _ = app.handle(Message::Backend(Event::AttachmentPreviewPrepared(
        id.clone(),
        0,
        Some(prepared("notes.txt", true)),
    )));
    let _ = app.handle(Message::Backend(Event::AttachmentPreviewPrepared(
        id.clone(),
        1,
        None,
    )));

    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id.clone(), 0)));
    assert!(
        preview_request(&mut attachments).is_none(),
        "no work after a click"
    );
    let state = &app.attachment_preview;
    assert!(state.from_cache && !state.loading);
    assert_eq!(
        state.preview.as_ref().map(|p| p.name.as_str()),
        Some("notes.txt")
    );

    // An attachment that could not be prepared is requested as before.
    let _ = app.handle(Message::AttachmentPreview(Preview::Step(true)));
    let (_, _, index) = preview_request(&mut attachments).expect("interactive request");
    assert_eq!(index, 1);
    assert!(!app.attachment_preview.from_cache);
    // A failed attempt is not retried on every selection.
    app.prepare_attachment_previews();
    assert!(prepare_request(&mut preparing).is_none());
    Ok(())
}

#[tokio::test]
async fn a_click_waiting_for_its_preview_takes_the_prepared_one() -> anyhow::Result<()> {
    let (mut app, mut attachments, _, _, id) = reader_lanes().await?;
    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id.clone(), 0)));
    let (request, ..) = preview_request(&mut attachments).expect("request");

    // Another attachment's preparation leaves the click waiting.
    let _ = app.handle(Message::Backend(Event::AttachmentPreviewPrepared(
        id.clone(),
        1,
        Some(prepared("setup.exe", false)),
    )));
    assert!(app.attachment_preview.loading);

    let _ = app.handle(Message::Backend(Event::AttachmentPreviewPrepared(
        id.clone(),
        0,
        Some(prepared("notes.txt", true)),
    )));
    assert!(!app.attachment_preview.loading);
    // The slower interactive reply cannot replace it afterwards.
    let _ = app.handle(Message::Backend(Event::AttachmentPreview(
        request,
        Ok(prepared("stale.txt", true)),
    )));
    assert_eq!(
        app.attachment_preview
            .preview
            .as_ref()
            .map(|p| p.name.as_str()),
        Some("notes.txt")
    );
    Ok(())
}

#[tokio::test]
async fn nearby_unread_messages_are_prepared_after_the_open_one() -> anyhow::Result<()> {
    let (mut app, _, _, mut preparing, id) = reader_lanes().await?;
    let summary = app
        .detail
        .as_ref()
        .map(|d| d.summary.clone())
        .expect("reader");
    let neighbour = crate::model::Mail {
        id: "neighbour".into(),
        unread: true,
        attachment_count: 3,
        ..summary.clone()
    };
    let read = crate::model::Mail {
        id: "read".into(),
        unread: false,
        attachment_count: 3,
        ..summary.clone()
    };
    app.page = Arc::new(MailPage {
        rows: vec![summary, read, neighbour],
        ..Default::default()
    });
    app.prepare_attachment_previews();
    let (first, ..) = prepare_request(&mut preparing).expect("open message");
    assert_eq!(first, id);
    let (source, attachments, indices) = prepare_request(&mut preparing).expect("neighbour");
    assert_eq!(source, "neighbour");
    assert!(
        attachments.is_none(),
        "not in the body cache, so read in the engine"
    );
    assert_eq!(indices, [0, 1, 2]);
    Ok(())
}

#[tokio::test]
async fn a_full_preparation_queue_is_dropped_quietly_and_retried() -> anyhow::Result<()> {
    let (mut app, _, _, mut preparing, id) = reader_lanes().await?;
    for _ in 0..2 {
        app.tx
            .as_ref()
            .expect("sender")
            .try_send(Command::PrepareAttachmentPreviews(
                "other".into(),
                None,
                vec![],
            ))
            .map_err(|error| anyhow::anyhow!("{error}"))?;
    }
    app.prepare_attachment_previews();
    assert!(
        app.notice.is_none(),
        "speculative work never shows a busy notice"
    );

    while prepare_request(&mut preparing).is_some() {}
    app.prepare_attachment_previews();
    let (source, ..) = prepare_request(&mut preparing).expect("retried");
    assert_eq!(source, id);
    Ok(())
}

#[tokio::test]
async fn full_screen_survives_stepping_and_escape_leaves_it_before_closing() -> anyhow::Result<()> {
    use iced::keyboard::{self, Key};
    let (mut app, mut attachments, _, id) = reader().await?;
    // Full screen means nothing until a preview is open.
    let _ = app.handle(Message::AttachmentPreview(Preview::FullScreen(true)));
    assert!(!app.attachment_preview.full_screen);

    let _ = app.handle(Message::AttachmentPreview(Preview::Show(id, 0)));
    let _ = preview_request(&mut attachments);
    let _ = app.handle(Message::AttachmentPreview(Preview::FullScreen(true)));
    assert!(app.attachment_preview.full_screen);
    let _ = app.handle(Message::AttachmentPreview(Preview::Step(true)));
    assert!(app.attachment_preview.full_screen);

    let escape = || Key::Named(keyboard::key::Named::Escape);
    let _ = app.key(escape(), keyboard::Modifiers::default(), false);
    assert!(!app.attachment_preview.full_screen);
    assert_eq!(app.dialog, Some(Dialog::Attachment));
    let _ = app.key(escape(), keyboard::Modifiers::default(), false);
    assert_eq!(app.dialog, None);

    // Reopening starts at the dialog size again.
    let _ = app.handle(Message::AttachmentPreview(Preview::FullScreen(true)));
    assert!(!app.attachment_preview.full_screen);
    Ok(())
}
