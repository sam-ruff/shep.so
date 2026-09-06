use super::*;
use futures::StreamExt;
use std::collections::HashMap;

async fn next(rx: &mut mpsc::Receiver<Event>) -> Event {
    tokio::time::timeout(std::time::Duration::from_secs(20), rx.next())
        .await
        .unwrap()
        .unwrap()
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
    let thread = std::thread::spawn(move || worker(input, output));
    (tx, rx, thread)
}
#[tokio::test]
async fn html_table_styles_render_and_long_documents_keep_viewport_sized_frames() {
    let (tx, mut rx, thread) = start();
    tx.send(Input::Load { generation: 1, body: body("<html><body><table style=\"width:100%;border-collapse:collapse\"><tr><td style=\"background:#ff0000;height:70px;width:50%\">First cell</td><td style=\"background:#0000ff\">Second cell</td></tr></table><div style=\"height:20000px\">Long message</div><p>Last line</p></body></html>"), viewport: viewport(), font_size: 14, hide_quotes: false }).await.unwrap();
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
    tx.send(Input::Load { generation: 7, body: body("<html><head><style>@import url('https://invalid.example/private.css');</style><script>fetch('https://invalid.example/secret')</script></head><body><img src=\"file:///etc/passwd\" width=20 height=20><img src=\"https://invalid.example/tracker.png\" width=20 height=20>Safe text</body></html>"), viewport: viewport(), font_size: 14, hide_quotes: false }).await.unwrap();
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
    tx.send(Input::Load {generation: 1, body: body("<table style=\"width:1000px;border-collapse:collapse\"><tr><td style=\"width:500px;height:60px;background:red\">Left column</td><td style=\"width:500px;background:blue\">Right column</td></tr></table>"), viewport: viewport(), font_size:14, hide_quotes:false}).await.unwrap();
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
