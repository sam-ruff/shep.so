use super::*;
use futures::StreamExt;
use std::collections::HashMap;

async fn next(rx: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(std::time::Duration::from_secs(20), rx.next())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn cancelling_a_loaded_document_stops_the_renderer_with_its_sender_retained() {
    let (tx, input) = commands::channel(16);
    let cancel = input.cancel_on_drop();
    let (output, mut events) = mpsc::channel(4);
    let worker =
        tokio::task::spawn_blocking(move || worker(input, output, Arc::new(AtomicU64::new(0))));
    tx.send(Input::Load {
        generation: 1,
        body: body("<p>Close this formatted message</p>"),
        viewport: viewport(),
        font_size: 14,
        hide_quotes: true,
        images: vec![],
    })
    .await
    .unwrap();
    assert!(matches!(next(&mut events).await, Event::Frame(_)));
    drop(cancel);
    drop(events);
    tokio::time::timeout(std::time::Duration::from_secs(2), worker)
        .await
        .unwrap()
        .unwrap();
    assert!(tx.is_closed());
}

#[tokio::test]
async fn find_uses_visible_text_across_styles_and_rebuilds_after_wrapping() {
    let (tx, mut rx, thread) = start();
    tx.send(Input::Load { generation: 90, body: body("<p>CAFÉ <b>project</b> plan and café project plan.</p><blockquote>Invisible project plan</blockquote><p style='display:none'>Hidden project plan</p><p>Literal [a.*] Σ σ ς</p>"), viewport: viewport(), font_size: 14, hide_quotes: true, images: Vec::new() }).await.unwrap();
    assert!(matches!(next(&mut rx).await, Event::Frame(_)));
    tx.send(Input::Find(90, 1, "café project plan".into(), false))
        .await
        .unwrap();
    let Event::Found(90, 1, layout, Ok(found)) = next(&mut rx).await else {
        panic!()
    };
    assert_eq!(found.matches.len(), 2);
    assert_eq!(
        found.matches[0].rectangles.len(),
        1,
        "Adjacent styled words share one highlight"
    );
    tx.send(Input::Find(90, 2, "project plan".into(), false))
        .await
        .unwrap();
    assert!(
        matches!(next(&mut rx).await, Event::Found(90, 2, _, Ok(found)) if found.matches.len() == 2)
    );
    tx.send(Input::Resize(
        90,
        Viewport {
            width: 100,
            ..viewport()
        },
    ))
    .await
    .unwrap();
    assert!(matches!(next(&mut rx).await, Event::Frame(_)));
    assert!(
        matches!(next(&mut rx).await, Event::Found(90, 2, revision, Ok(found)) if revision > layout && found.matches.len() == 2)
    );
    tx.send(Input::Find(90, 3, "[a.*]".into(), false))
        .await
        .unwrap();
    assert!(
        matches!(next(&mut rx).await, Event::Found(90, 3, _, Ok(found)) if found.matches.len() == 1)
    );
    tx.send(Input::Find(90, 4, "CAFÉ".into(), true))
        .await
        .unwrap();
    assert!(
        matches!(next(&mut rx).await, Event::Found(90, 4, _, Ok(found)) if found.matches.len() == 1)
    );
    drop(tx);
    thread.join().unwrap();
}
fn body(source: &str) -> Arc<HtmlBody> {
    Arc::new(HtmlBody::new(source.into(), HashMap::new()))
}
fn viewport() -> Viewport {
    Viewport {
        width: 400,
        height: 180,
        scale: 1.,
    }
}
fn start() -> (
    commands::Sender<Input>,
    mpsc::Receiver<Event>,
    std::thread::JoinHandle<()>,
) {
    let (tx, input) = commands::channel(16);
    let (output, rx) = mpsc::channel(4);
    let thread = std::thread::spawn(move || worker(input, output, Arc::new(AtomicU64::new(0))));
    (tx, rx, thread)
}

#[tokio::test]
async fn simple_letters_have_padded_centered_columns_with_selectable_wrapped_text() {
    let (tx, mut rx, thread) = start();
    for (generation, width, font, expected_x) in
        [(1, 1000, 14, 184.), (2, 340, 14, 20.), (3, 1600, 22, 292.)]
    {
        tx.send(Input::Load {
            generation,
            body: body("<p style='margin:0'>Column marker</p><p>A readable letter with enough text to wrap naturally as the available space changes.</p>"),
            viewport: Viewport { width, height: 400, scale: 1. },
            font_size: font,
            hide_quotes: false,
            images: vec![],
        }).await.unwrap();
        let Event::Frame(frame) = next(&mut rx).await else {
            panic!("Expected pixels")
        };
        assert!(frame.content_width <= width as f32 + 1.);
        tx.send(Input::Find(
            generation,
            generation,
            "Column marker".into(),
            false,
        ))
        .await
        .unwrap();
        let Event::Found(_, _, _, Ok(found)) = next(&mut rx).await else {
            panic!("Expected rendered text geometry")
        };
        let first = &found.matches[0].rectangles[0];
        assert!(
            (first[0] - expected_x).abs() < 2.,
            "{width}/{font}: {first:?}"
        );
        assert!(first[1] >= 16., "Text must clear the top edge: {first:?}");
        tx.send(Input::SelectAll(generation)).await.unwrap();
        let Event::Selection(_, selected, _, _) = next(&mut rx).await else {
            panic!("Expected native selection")
        };
        assert!(selected.contains("Column marker"));
        assert!(selected.contains("available space changes"));
    }
    drop(tx);
    thread.join().unwrap();
}
#[tokio::test]
async fn image_reflows_keep_visible_pixels_and_coalesce_until_native_acknowledgement() {
    let (tx, mut rx, thread) = start();
    let source = format!(
        "<img src='https://example.test/one.webp' style='display:block;width:200px;height:auto'><img src='https://example.test/two.webp' style='display:block;width:200px;height:auto'>{}",
        (0..80)
            .map(|i| format!("<p>Reading paragraph {i}: keep this text steady.</p>"))
            .collect::<String>()
    );
    tx.send(Input::Load {
        generation: 44,
        body: body(&source),
        viewport: viewport(),
        font_size: 14,
        hide_quotes: false,
        images: vec![],
    })
    .await
    .unwrap();
    assert!(matches!(next(&mut rx).await, Event::Frame(_)));
    tx.send(Input::View(44, viewport(), 600.)).await.unwrap();
    let Event::Frame(before) = next(&mut rx).await else {
        panic!()
    };
    let bytes: Arc<[u8]> = Arc::from(include_bytes!("../../assets/logo-light.webp").as_slice());
    tx.send(Input::Image(
        44,
        "https://example.test/one.webp".into(),
        bytes.clone(),
    ))
    .await
    .unwrap();
    let Event::Frame(first) = next(&mut rx).await else {
        panic!()
    };
    assert_eq!(first.reflow.unwrap().from, 600.);
    assert_eq!(first.reflow.unwrap().to, 800.);
    assert_eq!(
        first.pixels, before.pixels,
        "The paragraph must stay at exactly the same pixel position"
    );
    tx.send(Input::Image(
        44,
        "https://example.test/two.webp".into(),
        bytes,
    ))
    .await
    .unwrap();
    tx.send(Input::Copy(44)).await.unwrap();
    assert!(
        matches!(next(&mut rx).await, Event::Copy(44, _)),
        "An image arrival must not block input while waiting for the scroller"
    );
    tx.send(Input::ReflowApplied(44, first.layout_revision, 800.))
        .await
        .unwrap();
    let Event::Frame(second) = next(&mut rx).await else {
        panic!()
    };
    assert_eq!(second.content_height, before.content_height + 400.);
    assert_eq!(second.reflow.unwrap().from, 800.);
    assert_eq!(second.reflow.unwrap().to, 1000.);
    assert_eq!(second.pixels, before.pixels);
    tx.send(Input::ReflowApplied(44, second.layout_revision, 1000.))
        .await
        .unwrap();
    assert!(
        matches!(next(&mut rx).await, Event::Frame(f) if f.reflow.is_none() && f.scroll == 1000.)
    );
    drop(tx);
    thread.join().unwrap();
}
#[tokio::test]
async fn images_below_the_reader_and_acknowledgements_for_old_documents_cannot_move_it() {
    let (tx, mut rx, thread) = start();
    let source = format!(
        "{}<img src='https://example.test/below.webp' style='display:block;width:200px;height:auto'>",
        (0..80)
            .map(|i| format!("<p>Reading paragraph {i}: visible content.</p>"))
            .collect::<String>()
    );
    tx.send(Input::Load {
        generation: 50,
        body: body(&source),
        viewport: viewport(),
        font_size: 14,
        hide_quotes: false,
        images: vec![],
    })
    .await
    .unwrap();
    assert!(matches!(next(&mut rx).await, Event::Frame(_)));
    tx.send(Input::View(50, viewport(), 600.)).await.unwrap();
    let Event::Frame(before) = next(&mut rx).await else {
        panic!()
    };
    tx.send(Input::Image(
        50,
        "https://example.test/below.webp".into(),
        Arc::from(include_bytes!("../../assets/logo-light.webp").as_slice()),
    ))
    .await
    .unwrap();
    let Event::Frame(after) = next(&mut rx).await else {
        panic!()
    };
    assert_eq!(after.content_height, before.content_height + 200.);
    assert!(after.reflow.is_none());
    assert_eq!(after.scroll, before.scroll);
    assert_eq!(after.pixels, before.pixels);
    tx.send(Input::Load {
        generation: 51,
        body: body("<p>A different message.</p>"),
        viewport: viewport(),
        font_size: 14,
        hide_quotes: false,
        images: vec![],
    })
    .await
    .unwrap();
    assert!(matches!(next(&mut rx).await, Event::Frame(f) if f.generation == 51 && f.scroll == 0.));
    tx.send(Input::ReflowApplied(50, after.layout_revision, 1200.))
        .await
        .unwrap();
    tx.send(Input::SelectAll(51)).await.unwrap();
    assert!(
        matches!(next(&mut rx).await, Event::Selection(51, text, ..) if text == "A different message.")
    );
    drop(tx);
    thread.join().unwrap();
}
#[tokio::test]
async fn superseded_loads_are_discarded_before_parsing_or_reporting_an_error() {
    let (tx, input) = commands::channel(16);
    let (output, mut rx) = mpsc::channel(4);
    tx.send(Input::Load {
        generation: 1,
        body: body("Obsolete"),
        viewport: Viewport {
            scale: f32::NAN,
            ..viewport()
        },
        font_size: 14,
        hide_quotes: false,
        images: vec![],
    })
    .await
    .unwrap();
    tx.send(Input::Load {
        generation: 2,
        body: body("<p>Newest message</p>"),
        viewport: viewport(),
        font_size: 14,
        hide_quotes: false,
        images: vec![],
    })
    .await
    .unwrap();
    let thread = std::thread::spawn(move || worker(input, output, Arc::new(AtomicU64::new(2))));
    assert!(matches!(next(&mut rx).await, Event::Frame(frame) if frame.generation == 2));
    tx.send(Input::SelectAll(2)).await.unwrap();
    assert!(
        matches!(next(&mut rx).await, Event::Selection(2, text, ..) if text == "Newest message")
    );
    drop(tx);
    thread.join().unwrap();
}
#[tokio::test]
async fn html_table_styles_render_and_long_documents_keep_viewport_sized_frames() {
    let (tx, mut rx, thread) = start();
    tx.send(Input::Load { generation: 1, body: body("<html><body><table style=\"width:100%;border-collapse:collapse\"><tr><td style=\"background:#ff0000;height:70px;width:50%\">First cell</td><td style=\"background:#0000ff\">Second cell</td></tr></table><div style=\"height:20000px\">Long message</div><p>Last line</p></body></html>"), viewport: viewport(), font_size: 14, hide_quotes: false, images: Vec::new() }).await.unwrap();
    let Event::Frame(first) = next(&mut rx).await else {
        panic!("Expected rendered frame");
    };
    assert_eq!(first.pixels.len(), 400 * 180 * 4);
    assert!(first.content_height > 20000.);
    let red = first
        .pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 240 && p[1] < 10 && p[2] < 10)
        .count();
    let blue = first
        .pixels
        .chunks_exact(4)
        .filter(|p| p[2] > 240 && p[0] < 10 && p[1] < 10)
        .count();
    assert!(
        red > 1000 && blue > 1000,
        "Table backgrounds must be painted: {red}/{blue}"
    );
    tx.send(Input::Scroll(1, 20000.)).await.unwrap();
    let Event::Frame(last) = next(&mut rx).await else {
        panic!();
    };
    assert_eq!(last.pixels.len(), first.pixels.len());
    assert!(last.scroll > 19000.);
    assert_ne!(last.pixels, first.pixels);
    // A delayed command from another document must not scroll this one.
    tx.send(Input::Scroll(0, -500.)).await.unwrap();
    tx.send(Input::Copy(1)).await.unwrap();
    assert!(matches!(next(&mut rx).await, Event::Copy(1, _)));
    drop(tx);
    thread.join().unwrap();
}
#[tokio::test]
async fn resources_are_reported_without_fetching_and_rejected_documents_can_be_replaced() {
    let (tx, mut rx, thread) = start();
    tx.send(Input::Load { generation: 7, body: body("<html><head><style>@import url('https://invalid.example/private.css');</style><script>fetch('https://invalid.example/secret')</script></head><body><img src=\"file:///etc/passwd\" width=20 height=20><img src=\"https://invalid.example/tracker.png\" width=20 height=20>Safe text</body></html>"), viewport: viewport(), font_size: 14, hide_quotes: false, images: Vec::new() }).await.unwrap();
    let Event::Frame(frame) = next(&mut rx).await else {
        panic!();
    };
    assert_eq!(frame.generation, 7);
    assert!(
        frame
            .images
            .iter()
            .any(|url| url == "https://invalid.example/tracker.png")
    );
    // The container has no filesystem/network implementation; unresolved URLs
    // are emitted as data, not opened. CSS imports and scripts have no loader.
    tx.send(Input::Load {
        generation: 8,
        body: body("<p>Bad viewport</p>"),
        viewport: Viewport {
            scale: f32::NAN,
            ..viewport()
        },
        font_size: 14,
        hide_quotes: false,
        images: Vec::new(),
    })
    .await
    .unwrap();
    assert!(matches!(next(&mut rx).await, Event::Error(8, _)));
    tx.send(Input::Load {
        generation: 9,
        body: body("<p>A different message</p>"),
        viewport: viewport(),
        font_size: 14,
        hide_quotes: false,
        images: Vec::new(),
    })
    .await
    .unwrap();
    assert!(matches!(next(&mut rx).await, Event::Frame(frame) if frame.generation == 9));
    drop(tx);
    thread.join().unwrap();
}

#[tokio::test]
async fn selection_copy_inline_images_quotes_and_links_share_the_actual_html_layout() {
    let (tx, mut rx, thread) = start();
    let mut source = HtmlBody::new("<html><body><div style=\"height:40px\"><a href=\"https://example.test/help\">Helpful link</a> and <b>bold text</b>.</div><img src=\"cid:logo\" width=30 height=30><blockquote>Hidden quotation</blockquote><p>Last visible line.</p></body></html>".into(), HashMap::from([("logo".into(), Arc::from(include_bytes!("../../assets/logo-light.webp").as_slice()))]));
    assert!(source.has_quotes);
    tx.send(Input::Load {
        generation: 1,
        body: Arc::new(source.clone()),
        viewport: viewport(),
        font_size: 14,
        hide_quotes: true,
        images: Vec::new(),
    })
    .await
    .unwrap();
    let Event::Frame(frame) = next(&mut rx).await else {
        panic!();
    };
    assert!(!frame.images.iter().any(|u| u == "cid:logo"));
    tx.send(Input::SelectAll(1)).await.unwrap();
    let Event::Selection(1, selected, rects, _) = next(&mut rx).await else {
        panic!();
    };
    assert!(selected.contains("Helpful link"), "{selected:?}");
    assert!(selected.contains("bold text"), "{selected:?}");
    assert!(selected.contains("Last visible line."), "{selected:?}");
    assert!(
        !selected.contains("Hidden quotation"),
        "Hidden content must not enter the clipboard: {selected:?}"
    );
    assert!(!rects.is_empty());
    tx.send(Input::Copy(1)).await.unwrap();
    assert!(matches!(next(&mut rx).await, Event::Copy(1, text) if text == selected));
    tx.send(Input::Pointer(1, Pointer::Move, 10., 10.))
        .await
        .unwrap();
    assert!(matches!(
        next(&mut rx).await,
        Event::Selection(1, _, _, true)
    ));
    tx.send(Input::Pointer(1, Pointer::Down, 10., 10.))
        .await
        .unwrap();
    let _ = next(&mut rx).await;
    tx.send(Input::Pointer(1, Pointer::Up, 10., 10.))
        .await
        .unwrap();
    assert!(
        matches!(next(&mut rx).await, Event::Link(1, url) if url == "https://example.test/help")
    );
    let _ = next(&mut rx).await;
    source
        .source
        .push_str("<div style=\"height:1000px\">More content</div>");
    tx.send(Input::Load {
        generation: 2,
        body: Arc::new(source),
        viewport: Viewport {
            scale: 2.,
            ..viewport()
        },
        font_size: 18,
        hide_quotes: false,
        images: Vec::new(),
    })
    .await
    .unwrap();
    let Event::Frame(frame) = next(&mut rx).await else {
        panic!();
    };
    assert_eq!((frame.width, frame.height), (800, 360));
    tx.send(Input::View(
        2,
        Viewport {
            width: 250,
            scale: 2.,
            ..viewport()
        },
        500.,
    ))
    .await
    .unwrap();
    let Event::Frame(frame) = next(&mut rx).await else {
        panic!();
    };
    assert_eq!((frame.width, frame.height), (500, 360));
    assert_eq!(frame.scroll, 500.);
    drop(tx);
    thread.join().unwrap();
}

#[tokio::test]
async fn image_css_size_position_repeat_and_device_scale_are_applied_before_clipping() {
    let mut pixels = image::RgbaImage::new(256, 128);
    for (x, _, pixel) in pixels.enumerate_pixels_mut() {
        *pixel = if x < 128 {
            image::Rgba([255, 0, 0, 255])
        } else {
            image::Rgba([0, 0, 255, 255])
        };
    }
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(pixels)
        .write_to(&mut encoded, image::ImageFormat::WebP)
        .unwrap();
    let body = Arc::new(HtmlBody::new("<html><body><div style=\"padding:20px\"><img src=\"cid:pattern\" width=32 height=16><div style=\"margin-top:10px;width:100px;height:32px;background-image:url(cid:pattern);background-size:32px 16px;background-repeat:repeat-x\"></div></div></body></html>".into(), HashMap::from([("pattern".into(), Arc::from(encoded.into_inner()))])));
    let (tx, mut rx, thread) = start();
    for scale in [1., 2.] {
        tx.send(Input::Load {
            generation: 1,
            body: body.clone(),
            viewport: Viewport {
                scale,
                ..viewport()
            },
            font_size: 14,
            hide_quotes: false,
            images: Vec::new(),
        })
        .await
        .unwrap();
        let Event::Frame(frame) = next(&mut rx).await else {
            panic!();
        };
        let pixel = |x: f32, y: f32| -> &[u8] {
            let i = (((y * scale) as u32 * frame.width + (x * scale) as u32) * 4) as usize;
            &frame.pixels[i..i + 4]
        };
        assert_eq!(pixel(25., 25.), [255, 0, 0, 255]);
        assert_eq!(
            pixel(45., 25.),
            [0, 0, 255, 255],
            "A resized image must include its right side, not crop the original."
        );
        assert_eq!(
            pixel(60., 25.),
            [255, 255, 255, 255],
            "No-repeat images cannot bleed outside their CSS size."
        );
        // The following background repeats horizontally at its CSS tile size.
        assert_eq!(pixel(61., 55.), [255, 0, 0, 255]);
        assert_eq!(pixel(76., 55.), [0, 0, 255, 255]);
        assert_eq!(pixel(61., 75.), [255, 255, 255, 255]);
    }
    drop(tx);
    thread.join().unwrap();
}

#[tokio::test]
async fn fixed_width_tables_pan_without_allocating_full_document_rasters() {
    let (tx, mut rx, thread) = start();
    tx.send(Input::Load {generation: 1, body: body("<table style=\"width:1000px;border-collapse:collapse\"><tr><td style=\"width:500px;height:60px;background:red\">Left column</td><td style=\"width:500px;background:blue\">Right column</td></tr></table>"), viewport: viewport(), font_size:14, hide_quotes:false, images: Vec::new()}).await.unwrap();
    let Event::Frame(first) = next(&mut rx).await else {
        panic!()
    };
    assert!(first.content_width >= 1000.);
    tx.send(Input::Pan(1, 600.)).await.unwrap();
    let Event::Frame(last) = next(&mut rx).await else {
        panic!()
    };
    assert_eq!(last.pan, 600.);
    assert_eq!(last.pixels.len(), 400 * 180 * 4);
    let pixel = |frame: &Frame| frame.pixels[100 * 4..100 * 4 + 4].to_vec();
    assert_eq!(pixel(&first), [255, 0, 0, 255]);
    assert_eq!(pixel(&last), [0, 0, 255, 255]);
    drop(tx);
    thread.join().unwrap();
}

#[tokio::test]
async fn frames_acknowledge_applied_images_and_document_background() {
    let (tx, mut rx, thread) = start();
    let bytes: Arc<[u8]> = Arc::from(include_bytes!("../../assets/logo-light.webp").as_slice());
    let url = "https://example.test/parcel.webp";
    let source = "<body style='background:#f5eddc'><table width='200' align='center' bgcolor='white'><tr><td>Parcel update<img src='https://example.test/parcel.webp' width='64' height='64'></td></tr></table></body>";
    tx.send(Input::Load {
        generation: 61,
        body: body(source),
        viewport: viewport(),
        font_size: 14,
        hide_quotes: false,
        images: vec![],
    })
    .await
    .unwrap();
    let Event::Frame(initial) = next(&mut rx).await else {
        panic!()
    };
    assert_eq!(initial.background, Some([245, 237, 220, 255]));
    assert!(initial.loaded_images.is_empty());
    assert_eq!(initial.images, [url]);
    tx.send(Input::Image(61, url.into(), bytes.clone()))
        .await
        .unwrap();
    let Event::Frame(loaded) = next(&mut rx).await else {
        panic!()
    };
    assert_eq!(loaded.images, [url]);
    assert_eq!(loaded.loaded_images.len(), 1);
    assert!(Arc::ptr_eq(&loaded.loaded_images[0].1, &bytes));
    assert_ne!(initial.pixels, loaded.pixels);
    tx.send(Input::Load {
        generation: 62,
        body: body(source),
        viewport: viewport(),
        font_size: 14,
        hide_quotes: false,
        images: loaded.loaded_images.clone(),
    })
    .await
    .unwrap();
    let Event::Frame(reopened) = next(&mut rx).await else {
        panic!()
    };
    assert_eq!(loaded.pixels, reopened.pixels);
    assert_eq!(loaded.background, reopened.background);
    // An unrelated image never becomes an input of this document.
    tx.send(Input::Load {
        generation: 63,
        body: body("<p>Ordinary mail</p>"),
        viewport: viewport(),
        font_size: 14,
        hide_quotes: false,
        images: loaded.loaded_images.clone(),
    })
    .await
    .unwrap();
    let Event::Frame(ordinary) = next(&mut rx).await else {
        panic!()
    };
    assert_eq!(ordinary.background, Some([255; 4]));
    assert!(ordinary.loaded_images.is_empty());
    drop(tx);
    thread.join().unwrap();
}

#[test]
fn failed_image_replacement_cannot_acknowledge_bytes_that_were_not_displayed() {
    let surface = container::Surface::new(300, 200, 1., fonts());
    let url = "https://example.test/image.webp";
    let original: Arc<[u8]> = Arc::from(include_bytes!("../../assets/logo-light.webp").as_slice());
    assert!(surface.load_remote_image(url, original.clone()));
    assert!(!surface.load_remote_image(url, Arc::from(b"invalid image".as_slice())));
    let loaded = surface.loaded_images();
    assert_eq!(loaded.len(), 1);
    assert!(Arc::ptr_eq(&loaded[0].1, &original));
}

/// Local-only diagnostic: no provider/keychain calls, writes, subjects, addresses,
/// URLs or message text in output. Never use this path in the MCP harness.
#[test]
#[ignore = "Requires SHEP_PROFILE_CACHE pointing to an explicitly authorized local cache; reads only"]
fn profile_cached_html_read_only() {
    // Compare the same real document with/without table reuse. Both modes keep
    // the independently tested inline/caption corrections; no production switch.
    let reuse = std::env::var_os("SHEP_PROFILE_UNCACHED").is_none();
    let _mode = litehtml_sys::table_layout_test::Mode::new(reuse);
    let path = std::env::var("SHEP_PROFILE_CACHE").expect("Set the authorized cache path");
    let cache =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let raw: Vec<Vec<u8>> = cache
        .prepare("SELECT raw FROM messages WHERE folder='INBOX' ORDER BY timestamp DESC LIMIT 12")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let font_started = std::time::Instant::now();
    let fonts = fonts();
    println!(
        "font_discovery_ms={:.3}",
        font_started.elapsed().as_secs_f64() * 1000.
    );
    for (index, bytes) in raw.into_iter().enumerate() {
        let started = std::time::Instant::now();
        let parsed = shep_mail_core::mime::parse(&bytes).unwrap();
        let content = crate::email_content::extract(&parsed).unwrap();
        let mime_ms = started.elapsed().as_secs_f64() * 1000.;
        let Some(body) = content.html.map(Arc::new) else {
            continue;
        };
        for round in 0..2 {
            let (tx, mut input) = commands::channel(1);
            let (mut output, mut frames) = mpsc::channel(1);
            tx.try_send(Input::Clear).unwrap();
            let started = std::time::Instant::now();
            document(
                0,
                Source {
                    body: body.clone(),
                    font_size: 14,
                    hide_quotes: true,
                    images: vec![],
                },
                Viewport {
                    width: 740,
                    height: 700,
                    scale: 1.,
                },
                fonts.clone(),
                None,
                &mut input,
                &mut output,
            )
            .unwrap();
            let render_ms = started.elapsed().as_secs_f64() * 1000.;
            drop(output);
            let Some(Event::Frame(frame)) = futures::executor::block_on(frames.next()) else {
                panic!()
            };
            println!(
                "{}",
                serde_json::json!({"index":index,"round":round,"reuse":reuse,"mime_bytes":bytes.len(),"html_bytes":body.source.len(),"inline_bytes":body.inline.values().map(|b| b.len()).sum::<usize>(),"mime_ms":mime_ms,"render_ms":render_ms,"height":frame.content_height,"input_sha256":format!("{:x}",<sha2::Sha256 as sha2::Digest>::digest(&bytes)),"pixels_sha256":format!("{:x}",<sha2::Sha256 as sha2::Digest>::digest(&frame.pixels))})
            );
        }
    }
}

use crate::complex_html_fixture as complex_fixture;

#[path = "table_tests.rs"]
mod table_tests;
