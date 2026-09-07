//! HTML layout and rasterization live on an owned worker thread. iced receives
//! immutable viewport frames and small input results through bounded channels.
mod commands;
mod container;
pub mod preparation;
mod selection;
use crate::email_content::HtmlBody;
use futures::{SinkExt, channel::mpsc};
use litehtml::{Document, DrawContext, Position};
use selection::Selection;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
type Fonts = shep_html_pixbuf::FontSystem;
fn fonts() -> Fonts {
    shep_html_pixbuf::new_font_system()
}

#[derive(Debug, Clone)]
pub struct Source {
    pub body: Arc<HtmlBody>,
    pub font_size: u16,
    pub hide_quotes: bool,
    pub images: Vec<(String, Arc<[u8]>)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}
impl Viewport {
    fn validate(self) -> anyhow::Result<Self> {
        // This limits the physical viewport allocation, never the document length.
        anyhow::ensure!(
            self.width > 0
                && self.height > 0
                && self.scale.is_finite()
                && (0.5..=4.).contains(&self.scale),
            "Invalid message viewport."
        );
        anyhow::ensure!(
            (self.width as f64 * self.height as f64 * f64::from(self.scale).powi(2)) <= 16_777_216.,
            "The message viewport is too large to render. Reduce window scaling."
        );
        Ok(self)
    }
}
#[derive(Debug, Clone)]
pub enum Input {
    Load {
        generation: u64,
        body: Arc<HtmlBody>,
        viewport: Viewport,
        font_size: u16,
        hide_quotes: bool,
        images: Vec<(String, Arc<[u8]>)>,
    },
    Resize(u64, Viewport),
    View(u64, Viewport, f32),
    SelectAll(u64),
    Scroll(u64, f32),
    Pan(u64, f32),
    Pointer(u64, Pointer, f32, f32),
    Copy(u64),
    Image(u64, String, Arc<[u8]>),
    ReflowApplied(u64, u64, f32),
    Find(u64, u64, String, bool),
    PlainFind(crate::message_find::plain::Request),
    Clear,
}
#[derive(Debug, Clone, Copy)]
pub enum Pointer {
    Down,
    Move,
    Up,
    Leave,
}
#[derive(Debug, Clone, Copy)]
pub struct Reflow {
    pub from: f32,
    pub to: f32,
}
#[derive(Debug, Clone)]
pub struct Frame {
    pub generation: u64,
    pub layout_revision: u64,
    pub viewport: Viewport,
    pub pixels: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub content_height: f32,
    pub content_width: f32,
    pub pan: f32,
    pub scroll: f32,
    pub images: Vec<String>,
    /// Validated remote resources actually decoded into these pixels.
    pub loaded_images: Vec<(String, Arc<[u8]>)>,
    pub background: Option<[u8; 4]>,
    pub reflow: Option<Reflow>,
}
impl Frame {
    pub fn matches_view(&self, viewport: Viewport, top: f32) -> bool {
        self.viewport == viewport
            && (self.scroll - top.clamp(0., (self.content_height - viewport.height as f32).max(0.)))
                .abs()
                < 1.
    }
}
#[derive(Debug, Clone)]
pub enum Event {
    Ready(commands::Sender<Input>, Arc<AtomicU64>),
    Frame(Arc<Frame>),
    Selection(u64, String, Vec<[f32; 4]>, bool),
    Copy(u64, String),
    Link(u64, String),
    Error(u64, String),
    Found(
        u64,
        u64,
        u64,
        Result<Arc<crate::message_find::Results>, String>,
    ),
    PlainFound(u64, Result<Arc<crate::message_find::Results>, String>),
}
pub fn subscription() -> impl futures::Stream<Item = Event> {
    iced::stream::channel(4, |mut output: mpsc::Sender<Event>| async move {
        let (tx, rx) = commands::channel(16);
        let _cancel = rx.cancel_on_drop();
        let current = Arc::new(AtomicU64::new(0));
        if output
            .send(Event::Ready(tx, current.clone()))
            .await
            .is_err()
        {
            return;
        }
        let _ = tokio::task::spawn_blocking(move || worker(rx, output, current)).await;
    })
}
fn emit(output: &mut mpsc::Sender<Event>, event: Event) -> bool {
    futures::executor::block_on(output.send(event)).is_ok()
}
fn worker(
    mut input: commands::Receiver<Input>,
    mut output: mpsc::Sender<Event>,
    current: Arc<AtomicU64>,
) {
    #[cfg(feature = "test-support")]
    let mut failure_once = std::env::args().any(|arg| arg == "--demo")
        && std::env::var("SHEP_TEST_HTML_FAILURE_ONCE").is_ok_and(|v| v == "1");
    #[cfg(feature = "test-support")]
    let delay = if std::env::args().any(|arg| arg == "--demo") {
        std::env::var("SHEP_TEST_HTML_DELAY_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
            .min(2000)
    } else {
        0
    };
    let mut next = None;
    let mut plain_fonts = None;
    let mut html_fonts = None;
    loop {
        let Some(command) = next.take().or_else(|| input.blocking_recv()) else {
            return;
        };
        if let Input::PlainFind(request) = command {
            if request.current.load(std::sync::atomic::Ordering::Relaxed) != request.revision {
                continue;
            }
            let fonts = plain_fonts.get_or_insert_with(crate::message_find::plain::fonts);
            let result = crate::message_find::plain::find(fonts, &request).map(Arc::new);
            if !emit(&mut output, Event::PlainFound(request.revision, result)) {
                return;
            }
            continue;
        }
        let Input::Load {
            generation,
            body,
            viewport,
            font_size,
            hide_quotes,
            images,
        } = command
        else {
            continue;
        };
        if current.load(Ordering::Relaxed) > generation {
            continue;
        }
        #[cfg(feature = "test-support")]
        if std::mem::take(&mut failure_once) {
            if !emit(
                &mut output,
                Event::Error(
                    generation,
                    "The formatted preview could not be prepared. Try again.".into(),
                ),
            ) {
                return;
            }
            continue;
        }
        #[cfg(feature = "test-support")]
        if delay > 0 {
            std::thread::sleep(std::time::Duration::from_millis(delay));
        }
        if current.load(Ordering::Relaxed) > generation {
            continue;
        }
        match document(
            generation,
            Source {
                body,
                font_size,
                hide_quotes,
                images,
            },
            viewport,
            html_fonts.get_or_insert_with(fonts).clone(),
            Some(&current),
            &mut input,
            &mut output,
        ) {
            Ok(load) => next = load,
            Err(error) => {
                if !emit(
                    &mut output,
                    Event::Error(
                        generation,
                        format!(
                            "The HTML message could not be displayed. Use Plain text or retry. {error}"
                        ),
                    ),
                ) {
                    return;
                }
            }
        }
    }
}
fn document(
    generation: u64,
    source: Source,
    mut viewport: Viewport,
    fonts: Fonts,
    current: Option<&AtomicU64>,
    input: &mut commands::Receiver<Input>,
    output: &mut mpsc::Sender<Event>,
) -> anyhow::Result<Option<Input>> {
    viewport.validate()?;
    #[cfg(test)]
    let mut profiling = std::time::Instant::now();
    let Source {
        body,
        font_size,
        hide_quotes,
        images,
    } = source;
    let surface = container::Surface::new(viewport.width, viewport.height, viewport.scale, fonts);
    let mut container = surface.clone();
    let font_size = font_size.clamp(11, 26);
    let reading = if body.reading_column {
        "body{box-sizing:border-box;max-width:48em;margin:0 auto;padding:16px 20px}"
    } else {
        ""
    };
    let source = format!(
        "<style>body{{font-family:sans-serif;font-size:{font_size}px;line-height:1.5;color:#18181b;background:#fff;margin:0}}img{{max-width:100%}}{reading}</style>{}",
        body.source.replace('\0', "\u{fffd}")
    );
    let quotes =
        hide_quotes.then_some("blockquote,.gmail_quote,.yahoo_quoted{display:none!important}");
    for (cid, bytes) in &body.inline {
        if let Ok(webp) = crate::remote_images::convert_to_webp(bytes) {
            surface
                .0
                .borrow_mut()
                .load_image_data(&format!("cid:{cid}"), &webp);
        }
    }
    // These are already validated, cached WebP bytes; never fetch resources here.
    surface.seed_images(images);
    let measure = surface.0.borrow().text_measure_fn();
    #[cfg(test)]
    profile_stage("prepare", &mut profiling);
    let mut document = Document::from_html(&source, &mut container, None, quotes)
        .map_err(|_| anyhow::anyhow!("HTML parsing failed."))?;
    #[cfg(test)]
    profile_stage("parse", &mut profiling);
    let _ = document.render(viewport.width as f32);
    #[cfg(test)]
    profile_stage("layout", &mut profiling);
    let mut selection = Selection::default();
    selection.layout(&document);
    #[cfg(test)]
    profile_stage("selection", &mut profiling);
    let mut dragging = false;
    let mut drag_origin = (0., 0.);
    let mut scroll = 0.;
    let mut view_top = 0.;
    let mut reflow = None;
    let mut images_changed = false;
    let mut pan = 0.;
    let mut repaint = true;
    let mut buffered = None;
    let mut layout_revision = 1;
    let mut find: Option<(u64, String, bool)> = None;
    let mut find_pending = false;
    loop {
        if current.is_some_and(|current| current.load(Ordering::Relaxed) > generation) {
            return Ok(None);
        }
        if images_changed && reflow.is_none() {
            let anchor =
                selection.anchor(view_top, pan, viewport.width as f32, viewport.height as f32);
            let _ = document.render(viewport.width as f32);
            selection.layout(&document);
            layout_revision += 1;
            find_pending = true;
            if let Some(delta) = anchor.and_then(|a| selection.displacement(a))
                && delta.abs() >= 1.
            {
                let to = (view_top + delta).max(0.).floor();
                reflow = Some(Reflow { from: view_top, to });
                scroll = to.clamp(0., (document.height() - viewport.height as f32).max(0.));
            }
            images_changed = false;
            repaint = true;
        }
        if repaint {
            let pixels = {
                surface.0.borrow_mut().resize_with_scale(
                    viewport.width,
                    viewport.height,
                    viewport.scale,
                );
                surface.begin_draw();
                document.draw(
                    DrawContext::default(),
                    -pan,
                    -scroll,
                    Some(Position {
                        x: 0.,
                        y: 0.,
                        width: viewport.width as f32,
                        height: viewport.height as f32,
                    }),
                );
                let mut pixels = surface.0.borrow().pixels().to_vec();
                // tiny-skia's buffer is premultiplied; iced images use straight RGBA.
                for p in pixels.chunks_exact_mut(4) {
                    if p[3] > 0 && p[3] < 255 {
                        for i in 0..3 {
                            p[i] = (u16::from(p[i]) * 255 / u16::from(p[3])).min(255) as u8;
                        }
                    }
                }
                Arc::from(pixels)
            };
            let frame = {
                let used_seeded_images = surface.requested_images();
                let loaded_images = surface.loaded_images();
                let background = surface.background();
                let mut surface = surface.0.borrow_mut();
                surface.take_pending_images();
                Frame {
                    generation,
                    layout_revision,
                    viewport,
                    pixels,
                    width: surface.width(),
                    height: surface.height(),
                    content_height: document.height(),
                    content_width: document.width(),
                    pan,
                    scroll,
                    images: used_seeded_images,
                    loaded_images,
                    background,
                    reflow,
                }
            };
            #[cfg(test)]
            profile_stage("paint", &mut profiling);
            if !emit(output, Event::Frame(Arc::new(frame))) {
                return Ok(None);
            }
            repaint = false;
        }
        if find_pending {
            if let Some((revision, query, match_case)) = &find {
                let result = selection
                    .find(query, *match_case, &measure)
                    .map(Arc::new)
                    .map_err(|error| error.to_string());
                if !emit(
                    output,
                    Event::Found(generation, *revision, layout_revision, result),
                ) {
                    return Ok(None);
                }
            }
            find_pending = false;
        }
        let Some(mut command) = buffered.take().or_else(|| input.blocking_recv()) else {
            return Ok(None);
        };
        while matches!(
            command,
            Input::View(..) | Input::Find(..) | Input::Pointer(_, Pointer::Move, ..)
        ) {
            let Ok(next) = input.try_recv() else {
                break;
            };
            let compatible = matches!((&command, &next), (Input::View(a, ..), Input::View(b, ..)) | (Input::Find(a, ..), Input::Find(b, ..)) | (Input::Pointer(a, Pointer::Move, ..), Input::Pointer(b, Pointer::Move, ..)) if a == b);
            if compatible {
                command = next;
            } else {
                buffered = Some(next);
                break;
            }
        }
        match command {
            Input::Find(id, revision, query, match_case) if id == generation => {
                find = Some((revision, query, match_case));
                find_pending = true;
            }
            Input::Load { .. } | Input::PlainFind(..) | Input::Clear => return Ok(Some(command)),
            Input::View(id, size, top) if id == generation && top.is_finite() => {
                let size = size.validate()?;
                if reflow.is_some_and(|r| size == viewport && (top - r.from).abs() < 1.) {
                    continue;
                }
                reflow = None;
                view_top = top;
                if size == viewport
                    && scroll == top.clamp(0., (document.height() - size.height as f32).max(0.))
                {
                    continue;
                }
                if size != viewport {
                    viewport = size;
                    surface.0.borrow_mut().resize_with_scale(
                        viewport.width,
                        viewport.height,
                        viewport.scale,
                    );

                    document.media_changed();
                    let _ = document.render(viewport.width as f32);
                    selection.layout(&document);
                    layout_revision += 1;
                    find_pending = true;
                }
                pan = pan.min((document.width() - viewport.width as f32).max(0.));
                scroll = top.clamp(0., (document.height() - viewport.height as f32).max(0.));
                repaint = true;
            }
            Input::Resize(id, size) if id == generation => {
                reflow = None;
                viewport = size.validate()?;
                surface.0.borrow_mut().resize_with_scale(
                    viewport.width,
                    viewport.height,
                    viewport.scale,
                );

                document.media_changed();
                let _ = document.render(viewport.width as f32);
                selection.layout(&document);
                layout_revision += 1;
                find_pending = true;
                pan = pan.min((document.width() - viewport.width as f32).max(0.));
                scroll = scroll.min((document.height() - viewport.height as f32).max(0.));
                repaint = true;
            }
            Input::Pan(id, x) if id == generation && x.is_finite() => {
                pan = x.clamp(0., (document.width() - viewport.width as f32).max(0.));
                repaint = true;
            }
            Input::Scroll(id, delta) if id == generation && delta.is_finite() => {
                reflow = None;
                view_top = delta;
                scroll = delta.clamp(0., (document.height() - viewport.height as f32).max(0.));
                repaint = true;
            }
            Input::Pointer(id, kind, x, y)
                if id == generation && x.is_finite() && y.is_finite() =>
            {
                match kind {
                    Pointer::Down => {
                        dragging = true;
                        drag_origin = (x, y);
                        selection.start(&measure, x, y);
                        document.on_lbutton_down(x, y, x - pan, y - scroll);
                    }
                    Pointer::Move => {
                        document.on_mouse_over(x, y, x - pan, y - scroll);
                        if dragging {
                            selection.extend(&measure, x, y);
                        }
                    }
                    Pointer::Up => {
                        if dragging {
                            selection.extend(&measure, x, y);
                        }
                        dragging = false;
                        if (x - drag_origin.0).abs() + (y - drag_origin.1).abs() < 5. {
                            document.on_lbutton_up(x, y, x - pan, y - scroll);
                            if let Some(url) = surface.0.borrow_mut().take_anchor_click()
                                && !emit(output, Event::Link(generation, url))
                            {
                                return Ok(None);
                            }
                        }
                    }
                    Pointer::Leave => {
                        document.on_mouse_leave();
                    }
                }
                let (text, rects) = selection.result(&measure);
                let link = surface.0.borrow().cursor() == "pointer";
                if !emit(output, Event::Selection(generation, text, rects, link)) {
                    return Ok(None);
                }
            }
            Input::SelectAll(id) if id == generation => {
                selection.all();
                let (text, rects) = selection.result(&measure);
                if !emit(output, Event::Selection(generation, text, rects, false)) {
                    return Ok(None);
                }
            }
            Input::Copy(id) if id == generation => {
                if !emit(
                    output,
                    Event::Copy(generation, selection.result(&measure).0),
                ) {
                    return Ok(None);
                }
            }
            Input::Image(id, url, bytes) if id == generation => {
                // LoadImages has already validated and converted these bytes.
                // Avoid a second full decode/encode before the renderer decode.
                if surface.load_remote_image(&url, bytes) {
                    // Decode arrivals as usual, but coalesce subsequent layouts
                    // until the native scroller acknowledges the first anchor.
                    images_changed = true;
                }
            }
            Input::ReflowApplied(id, layout, top)
                if id == generation
                    && layout == layout_revision
                    && top.is_finite()
                    && reflow.is_some() =>
            {
                reflow = None;
                view_top = top;
                scroll = top.clamp(0., (document.height() - viewport.height as f32).max(0.));
                repaint = true;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
fn profile_stage(stage: &str, started: &mut std::time::Instant) {
    if std::env::var_os("SHEP_PROFILE_CACHE").is_some() {
        println!(
            "stage={stage} ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.
        );
        *started = std::time::Instant::now();
    }
}
