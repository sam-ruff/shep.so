use super::*;
use crate::document_files as fixture;
use mockall::predicate::function;

fn preview(name: &str, bytes: &[u8]) -> Preview {
    super::preview(name, bytes, 0)
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let image = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([(x % 255) as u8, (y % 255) as u8, 90, 255])
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .expect("encode fixture image");
    bytes
}

#[test]
fn images_decode_at_their_own_size() {
    let preview = preview("photo.png", &png(40, 30));
    assert_eq!(preview.kind, "Image");
    assert!(preview.openable());
    assert!(matches!(
        preview.content,
        Content::Image {
            width: 40,
            height: 30,
            ..
        }
    ));
}

#[test]
fn large_images_are_scaled_to_the_bounded_side() {
    let preview = preview("scan.png", &png(IMAGE_SIDE * 2, 100));
    let Content::Image { width, height, .. } = preview.content else {
        panic!("expected an image, got {:?}", preview.content);
    };
    assert_eq!(width, IMAGE_SIDE);
    assert!(height <= 100);
}

#[test]
fn image_type_comes_from_content_rather_than_name() {
    let preview = preview("photo.dat", &png(4, 4));
    assert_eq!(preview.kind, "Image");
}

#[test]
fn corrupt_images_fall_back_to_a_file_card() {
    let mut bytes = png(8, 8);
    bytes.truncate(20);
    let preview = preview("broken.png", &bytes);
    assert!(matches!(preview.content, Content::Unsupported));
    assert_eq!(preview.size, 20);
}

#[test]
fn text_files_preview_with_normalised_line_endings() {
    let preview = preview("notes.txt", b"\xef\xbb\xbfFirst\r\nSecond\r\n");
    let Content::Text { text, truncated } = preview.content else {
        panic!("expected text");
    };
    assert_eq!(text, "First\nSecond\n");
    assert!(!truncated);
}

#[test]
fn tabs_become_spaces_at_eight_column_stops() {
    let Content::Text { text, .. } = preview("rooms.tsv", b"Room\tWidth\nKitchen\t3.2\n").content
    else {
        panic!("expected text");
    };
    assert_eq!(text, "Room    Width\nKitchen 3.2\n");
}

#[test]
fn long_text_is_truncated_by_bytes_and_lines() {
    let bytes = "é".repeat(TEXT_LIMIT);
    let Content::Text { text, truncated } = preview("long.txt", bytes.as_bytes()).content else {
        panic!("expected text");
    };
    assert!(truncated);
    assert!(text.len() <= TEXT_LIMIT);
    assert!(!text.ends_with('\u{fffd}'));

    let lines = "line\n".repeat(TEXT_LINE_LIMIT * 2);
    let Content::Text { text, truncated } = preview("long.log", lines.as_bytes()).content else {
        panic!("expected text");
    };
    assert!(truncated);
    assert_eq!(text.lines().count(), TEXT_LINE_LIMIT);
}

#[test]
fn unnamed_text_is_recognised_but_binary_is_not() {
    assert_eq!(preview("README", b"Plain words\n").kind, "Text");
    let binary = preview("blob", &[0, 159, 146, 150, 1, 2]);
    assert!(matches!(binary.content, Content::Unsupported));
    assert_eq!(binary.kind, "File");
}

#[test]
fn known_document_types_get_readable_labels() {
    assert_eq!(preview("Invoice.PDF", b"%PDF-1.7").kind, "PDF document");
    assert_eq!(preview("report.docx", b"PK\x03\x04").kind, "Document");
}

#[test]
fn programs_and_scripts_are_never_openable() {
    for name in [
        "setup.exe",
        "invoice.pdf.EXE",
        "run.sh",
        "start.desktop",
        "page.html",
    ] {
        assert!(!openable(name, b"data"), "{name} should not be openable");
    }
    assert!(!openable("photo.jpg", b"\x7fELF\x02"));
    assert!(!openable("notes.txt", b"#!/bin/sh\nrm -rf ~"));
    assert!(!openable("report.pdf", b"MZ\x90\x00"));
    assert!(openable("report.pdf", &fixture::pdf(&["Plan"], "")));
    assert!(openable(".bashrc", b"export A=1"));
}

#[test]
fn pdf_pages_render_one_at_a_time() {
    let bytes = fixture::pdf(&["First floor", "Second floor"], "");
    let first = super::preview("plan.pdf", &bytes, 0);
    assert_eq!(first.kind, "PDF document");
    assert!(first.openable());
    let Content::Pages {
        width,
        height,
        page,
        pages,
        ..
    } = first.content
    else {
        panic!("expected a page, got {:?}", first.content);
    };
    assert_eq!((page, pages), (0, 2));
    // Letter pages scale so the long side fills the bounded size.
    assert!((height as f32 - pdf::PAGE_SIDE).abs() <= 1., "{height}");
    assert!(width < height);

    let Content::Pages { page, .. } = super::preview("plan.pdf", &bytes, 9).content else {
        panic!("expected the last page");
    };
    assert_eq!(page, 1);
}

#[test]
fn pdf_pages_draw_their_text() {
    let Content::Pages { handle, .. } = preview("plan.pdf", &fixture::pdf(&["HELLO"], "")).content
    else {
        panic!("expected a page");
    };
    let iced::advanced::image::Handle::Rgba { pixels, .. } = handle else {
        panic!("expected raw pixels");
    };
    // Text leaves dark pixels on the white page.
    assert!(pixels.chunks(4).any(|pixel| pixel[0] < 100));
    assert!(pixels.chunks(4).all(|pixel| pixel[3] == 255));
}

#[test]
fn unreadable_and_protected_pdfs_fall_back_to_a_card() {
    let broken = preview("broken.pdf", b"%PDF-1.7\nnot really a document");
    assert!(matches!(broken.content, Content::Unsupported));
    assert_eq!(broken.refusal, Some(Refusal::Unchecked));
}

#[test]
fn pdf_scripts_and_launch_actions_block_opening() {
    for extra in [
        "/OpenAction << /S /JavaScript /JS (app.alert(1)) >>",
        "/OpenAction << /S /Launch /F (calc.exe) >>",
        // Name escapes hide the word from a plain byte search.
        "/OpenAction << /S /J#61vaScript /J#53 (app.alert(1)) >>",
        "/AcroForm << /XFA 9 0 R >>",
    ] {
        let bytes = fixture::pdf(&["Invoice"], extra);
        assert_eq!(
            safety::refusal("invoice.pdf", &bytes),
            Some(Refusal::ActiveContent),
            "{extra}"
        );
        // The preview still renders: hayro never runs the script.
        assert!(matches!(
            preview("invoice.pdf", &bytes).content,
            Content::Pages { .. }
        ));
    }
    let plain = fixture::pdf(&["Invoice"], "/PageMode /UseNone");
    assert_eq!(safety::refusal("invoice.pdf", &plain), None);
}

#[test]
fn word_documents_show_headings_and_paragraphs() {
    let preview = preview("survey.docx", &fixture::docx(&[]));
    assert_eq!(preview.kind, "Word document");
    assert!(preview.openable());
    let Content::Document { blocks, truncated } = preview.content else {
        panic!("expected a document, got {:?}", preview.content);
    };
    assert!(!truncated);
    assert_eq!(
        blocks,
        [
            Block::Heading("Site survey".into()),
            Block::Paragraph("Walls & ceilings need a second coat.".into()),
            Block::Paragraph("Kitchen    3.2 m".into()),
        ]
    );
}

#[test]
fn presentations_list_slides_in_number_order() {
    let preview = preview("deck.pptx", &fixture::pptx());
    assert_eq!(preview.kind, "PowerPoint presentation");
    let Content::Document { blocks, .. } = preview.content else {
        panic!("expected slides");
    };
    let headings: Vec<_> = blocks
        .iter()
        .filter_map(|block| match block {
            Block::Heading(text) => Some(text.as_str()),
            Block::Paragraph(_) => None,
        })
        .collect();
    assert_eq!(headings, ["Slide 1", "Slide 2", "Slide 3"]);
    assert_eq!(blocks[1], Block::Paragraph("Project timeline".into()));
    assert_eq!(blocks[7], Block::Paragraph("Questions".into()));
}

#[test]
fn opendocument_text_skips_comments() {
    let preview = preview("notes.odt", &fixture::odt(&[]));
    assert_eq!(preview.kind, "OpenDocument text");
    let Content::Document { blocks, .. } = preview.content else {
        panic!("expected a document");
    };
    assert_eq!(
        blocks,
        [
            Block::Heading("Meeting notes".into()),
            Block::Paragraph("Agreed  the new layout.".into()),
            Block::Paragraph("Next visit in May.".into()),
        ]
    );
}

#[test]
fn spreadsheets_stream_cells_into_a_bounded_grid() {
    // The fixture has a cell at XFD5000; a dense reader would allocate the
    // whole span.
    let preview = preview("rooms.xlsx", &fixture::xlsx(&[]));
    assert_eq!(preview.kind, "Excel spreadsheet");
    let Content::Sheets(sheets) = preview.content else {
        panic!("expected sheets, got {:?}", preview.content);
    };
    assert_eq!(sheets.len(), 2);
    assert_eq!(sheets[0].name, "Rooms");
    assert!(sheets[0].truncated);
    assert_eq!(
        sheets[0].rows,
        [
            vec!["Room".to_owned(), "Width".into()],
            vec!["Kitchen".into(), "3.2".into()],
            vec!["Studio".into(), "4".into()],
        ]
    );
    assert_eq!(sheets[1].rows, [vec!["Paint arrives Friday".to_owned()]]);
    assert!(!sheets[1].truncated);
}

#[test]
fn opendocument_sheets_expand_repeats_within_bounds() {
    let preview = preview("budget.ods", &fixture::ods());
    let Content::Sheets(sheets) = preview.content else {
        panic!("expected sheets");
    };
    let sheet = &sheets[0];
    assert_eq!(sheet.name, "Budget");
    assert_eq!(sheet.rows[0], ["Item", "Cost"]);
    assert_eq!(sheet.rows[1], ["Paint", "Paint"]);
    // "Total" sits after a million blank rows, outside the preview.
    assert_eq!(sheet.rows.len(), 2);
    assert!(sheet.truncated);
}

#[test]
fn office_macros_and_embedded_programs_block_opening() {
    let active = [
        (
            "survey.docx",
            fixture::docx(&[("word/vbaProject.bin", "x")]),
        ),
        (
            "rooms.xlsx",
            fixture::xlsx(&[("xl/macrosheets/sheet1.xml", "<x/>")]),
        ),
        (
            "survey.docx",
            fixture::docx(&[("word/embeddings/oleObject1.bin", "x")]),
        ),
        (
            "survey.docx",
            fixture::docx(&[(
                "word/_rels/settings.xml.rels",
                r#"<Relationships><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/attachedTemplate" Target="https://example.test/t.dotm" TargetMode="External"/></Relationships>"#,
            )]),
        ),
        (
            "notes.odt",
            fixture::odt(&[("Basic/Standard/Module1.xml", "<x/>")]),
        ),
        // No extension: a desktop could still pick an Office app from content.
        ("invoice", fixture::docx(&[("word/vbaProject.bin", "x")])),
        ("report.docm", fixture::docx(&[])),
        (
            "legacy.doc",
            fixture::compound(&[("/WordDocument", b"x"), ("/Macros/VBA/dir", b"x")]),
        ),
        (
            "legacy.xls",
            fixture::compound(&[("/Workbook", &fixture::biff_globals(1))]),
        ),
        (
            "letter.rtf",
            b"{\\rtf1 Hello {\\object\\objemb {\\*\\objdata 0105}}}".to_vec(),
        ),
    ];
    for (name, bytes) in active {
        assert_eq!(
            safety::refusal(name, &bytes),
            Some(Refusal::ActiveContent),
            "{name}"
        );
    }
    let safe = [
        ("survey.docx", fixture::docx(&[])),
        (
            "survey.docx",
            fixture::docx(&[(
                "word/_rels/document.xml.rels",
                r#"<Relationships><Relationship Id="r1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.test/" TargetMode="External"/></Relationships>"#,
            )]),
        ),
        ("rooms.xlsx", fixture::xlsx(&[])),
        ("deck.pptx", fixture::pptx()),
        ("notes.odt", fixture::odt(&[])),
        ("budget.ods", fixture::ods()),
        ("legacy.doc", fixture::compound(&[("/WordDocument", b"x")])),
        (
            "legacy.xls",
            fixture::compound(&[("/Workbook", &fixture::biff_globals(0))]),
        ),
        ("letter.rtf", b"{\\rtf1 Hello}".to_vec()),
        // An ordinary archive is not an Office document.
        ("tools.zip", fixture::zip(&[("scripts/build.txt", "x")])),
    ];
    for (name, bytes) in safe {
        assert_eq!(safety::refusal(name, &bytes), None, "{name}");
    }
}

#[test]
fn unreadable_documents_cannot_be_opened() {
    for (name, bytes) in [
        ("survey.docx", b"PK\x03\x04broken".to_vec()),
        ("legacy.doc", b"not a compound file".to_vec()),
        (
            "legacy.ppt",
            fixture::compound(&[("/PowerPoint Document", b"x")]),
        ),
    ] {
        assert_eq!(
            safety::refusal(name, &bytes),
            Some(Refusal::Unchecked),
            "{name}"
        );
    }
}

#[test]
fn opening_a_document_with_macros_writes_nothing() {
    let parent = tempfile::tempdir().expect("parent");
    let files = Files::new(Some(parent.path().into()), Arc::new(MockLauncher::new()));
    let bytes = fixture::docx(&[("word/vbaProject.bin", "x")]);
    let error = files
        .open("survey.docx", &bytes, true)
        .expect_err("refused");
    assert!(error.to_string().contains("macros, scripts"));
    assert_eq!(std::fs::read_dir(parent.path()).expect("list").count(), 0);
}

#[test]
fn open_writes_a_private_copy_then_launches_it() {
    let parent = tempfile::tempdir().expect("parent");
    let mut launcher = MockLauncher::new();
    launcher
        .expect_open()
        .with(function(|path: &Path| {
            path.file_name().is_some_and(|name| name == "notes.txt")
                && std::fs::read(path).is_ok_and(|bytes| bytes == b"Monday at nine.")
        }))
        .times(1)
        .returning(|_| Ok(()));
    let files = Files::new(Some(parent.path().into()), Arc::new(launcher));
    let path = files
        .open("notes.txt", b"Monday at nine.", true)
        .expect("open");
    assert!(path.starts_with(parent.path()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn repeated_names_get_separate_copies() {
    let parent = tempfile::tempdir().expect("parent");
    let files = Files::new(Some(parent.path().into()), Arc::new(MockLauncher::new()));
    let first = files.open("same.txt", b"one", false).expect("first");
    let second = files.open("same.txt", b"two", false).expect("second");
    assert_ne!(first, second);
    assert_eq!(std::fs::read(first).expect("first bytes"), b"one");
    assert_eq!(std::fs::read(second).expect("second bytes"), b"two");
}

#[test]
fn sender_paths_cannot_escape_the_temporary_folder() {
    let parent = tempfile::tempdir().expect("parent");
    let files = Files::new(Some(parent.path().into()), Arc::new(MockLauncher::new()));
    let path = files
        .open("../../outside.txt", b"contained", false)
        .expect("open");
    assert!(path.starts_with(parent.path()));
    assert_eq!(path.file_name().expect("name"), "outside.txt");
}

#[test]
fn programs_are_refused_before_anything_is_written() {
    let parent = tempfile::tempdir().expect("parent");
    let files = Files::new(Some(parent.path().into()), Arc::new(MockLauncher::new()));
    let error = files.open("setup.exe", b"data", true).expect_err("refused");
    assert!(error.to_string().contains("Save a copy instead"));
    assert_eq!(std::fs::read_dir(parent.path()).expect("list").count(), 0);
}

#[test]
fn launcher_failures_are_reported() {
    let parent = tempfile::tempdir().expect("parent");
    let mut launcher = MockLauncher::new();
    launcher
        .expect_open()
        .returning(|_| Err(anyhow::anyhow!("no default app")));
    let files = Files::new(Some(parent.path().into()), Arc::new(launcher));
    assert!(files.open("notes.txt", b"text", true).is_err());
}

#[test]
fn zip_keeps_every_attachment_under_a_distinct_name() {
    let attachment = |name: &str, bytes: &[u8]| shep_mail_core::model::Attachment {
        name: name.into(),
        bytes: bytes.to_vec(),
    };
    let bytes = zip(&[
        attachment("report.pdf", b"first"),
        attachment("Report.pdf", b"second"),
        attachment("../notes", b"third"),
        attachment("report.pdf", b"fourth"),
    ])
    .expect("zip");
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("archive");
    let mut entries = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).expect("entry");
        let mut contents = String::new();
        std::io::Read::read_to_string(&mut file, &mut contents).expect("read");
        entries.push((file.name().expect("name").to_string(), contents));
    }
    let expected = [
        ("report.pdf", "first"),
        ("Report (2).pdf", "second"),
        ("notes", "third"),
        ("report (3).pdf", "fourth"),
    ];
    assert_eq!(
        entries,
        expected.map(|(name, contents)| (name.to_owned(), contents.to_owned()))
    );
}

#[test]
fn copies_are_removed_with_the_session() {
    let parent = tempfile::tempdir().expect("parent");
    let files = Files::new(Some(parent.path().into()), Arc::new(MockLauncher::new()));
    let path = files.open("notes.txt", b"text", false).expect("open");
    drop(files);
    assert!(!path.exists());
}
