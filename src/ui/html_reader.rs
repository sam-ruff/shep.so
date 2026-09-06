//! Small UI-side state only. DOM, fonts, image decoding and rasterization belong
//! to the renderer worker. Geometry/hover requests coalesce under backpressure.
mod cache;
mod canvas;
use super::*;
use crate::html_render::{self, Input, Viewport};

#[derive(Debug, Clone)]
pub enum Message {
    Backend(html_render::Event),
    Prepared(html_render::preparation::Event),
    Input(Input),
    Pump,
    Plain(bool),
    Quotes,
    Scroll(u64, f32),
    ScrollEnd(u64, bool),
    LinkResult(Result<(), String>),
    Scale(f32),
}
#[derive(Default)]
pub(super) struct State {
    tx: Option<tokio::sync::mpsc::Sender<Input>>,
    queue: VecDeque<Input>,
    pumping: bool,
    pending: bool,
    pub cache: cache::Cache,
    pub generation: u64,
    key: Option<(String, [u8; 32], u16, bool, bool, u16)>,
    plain: Option<String>,
    quote_override: Option<(String, bool)>,
    pub frame: Option<Arc<html_render::Frame>>,
    pub handle: Option<widget::image::Handle>,
    pub selection: String,
    pub rectangles: Vec<[f32; 4]>,
    pub link: bool,
    pub error: Option<String>,
    pub resources: HashSet<String>,
    pub supplied: HashSet<String>,
    viewport: Option<(Viewport, f32)>,
    pub last_link: Option<String>,
    pub system_scale: f32,
}
impl State {
    pub fn view_current(&self) -> bool {
        self.frame.as_ref().is_some_and(|frame| {
            self.viewport
                .is_some_and(|(size, top)| frame.matches_view(size, top))
        })
    }

    fn enqueue(&mut self, command: Input) {
        // A new view supersedes unprocessed geometry; moves supersede adjacent
        // hover/drag positions, but never a Down/Up/Copy boundary.
        if matches!(command, Input::Find(..) | Input::PlainFind(..)) {
            self.queue
                .retain(|c| !matches!(c, Input::Find(..) | Input::PlainFind(..)));
        }
        if matches!(command, Input::Pan(..)) {
            self.queue.retain(|c| !matches!(c, Input::Pan(..)));
        }
        if matches!(command, Input::View(..)) {
            self.queue.retain(|c| !matches!(c, Input::View(..)));
        }
        if matches!(command, Input::Pointer(_, html_render::Pointer::Move, ..))
            && matches!(
                self.queue.back(),
                Some(Input::Pointer(_, html_render::Pointer::Move, ..))
            )
        {
            self.queue.pop_back();
        }
        self.queue.push_back(command);
    }
    fn pump(&mut self) -> Task<super::Message> {
        // The native canvas provides the first real viewport. Keep Find/input
        // behind Load while geometry is still unknown.
        if self.pending {
            return Task::none();
        }
        if let Some(tx) = &self.tx {
            while let Some(command) = self.queue.pop_front() {
                if let Err(error) = tx.try_send(command) {
                    if matches!(error, tokio::sync::mpsc::error::TrySendError::Closed(_)) {
                        self.error =
                            Some("The message renderer stopped. Reopen Shep to retry.".into());
                        self.queue.clear();
                        break;
                    }
                    self.queue.push_front(error.into_inner());
                    if !self.pumping {
                        self.pumping = true;
                        return Task::perform(
                            tokio::time::sleep(std::time::Duration::from_millis(16)),
                            |_| super::Message::Html(Message::Pump),
                        );
                    }
                    break;
                }
            }
        }
        Task::none()
    }
}
impl App {
    pub(super) fn formatted(&self, detail: &MailDetail) -> bool {
        detail.html.is_some() && self.html_reader.plain.as_deref() != Some(&detail.summary.id)
    }
    pub(super) fn html_quotes_hidden(&self, detail: &MailDetail) -> bool {
        self.html_reader
            .quote_override
            .as_ref()
            .filter(|(id, _)| id == &detail.summary.id)
            .map(|(_, hidden)| *hidden)
            .unwrap_or(self.preferences.reply_display != ReplyDisplay::Expanded)
    }
    pub(super) fn prepare_html(&mut self) -> Task<super::Message> {
        let detail = self.detail.as_ref().filter(|d| {
            self.tab == Tab::Mail
                && self.formatted(d)
                && self.reader_id() == Some(d.summary.id.as_str())
        });
        let key = detail.map(|d| {
            (
                d.summary.id.clone(),
                d.html.as_ref().unwrap().signature,
                self.preferences.reader_font_size,
                self.html_quotes_hidden(d),
                crate::remote_images::allowed(&self.preferences, &d.summary),
                (self.html_scale() * 100.).round() as u16,
            )
        });
        let reset_scroll = !self.conversation_visible()
            && self.html_reader.key.as_ref().map(|key| &key.0) != key.as_ref().map(|key| &key.0);
        let reset_horizontal = self.html_reader.key != key;
        if reset_horizontal {
            let state = &mut self.html_reader;
            state.generation += 1;
            state.queue.clear();
            state.frame = None;
            state.handle = None;
            state.selection.clear();
            state.rectangles.clear();
            state.resources.clear();
            state.supplied.clear();
            state.error = None;
            state.last_link = None;
            state.link = false;
            state.key = key;
            state.viewport = None;
            state.pending = detail.is_some();
            if !state.pending {
                state.enqueue(Input::Clear);
            }
        }
        self.load_html_images();
        self.preload_html();
        let scroll = if reset_scroll {
            widget::operation::snap_to("message-reader", widget::scrollable::RelativeOffset::START)
        } else {
            Task::none()
        };
        let horizontal = if reset_horizontal {
            widget::operation::snap_to("html-horizontal", widget::scrollable::RelativeOffset::START)
        } else {
            Task::none()
        };
        Task::batch([self.html_reader.pump(), scroll, horizontal])
    }
    pub(super) fn handle_html(&mut self, message: Message) -> Task<super::Message> {
        use html_render::Event;
        match message {
            Message::Prepared(html_render::preparation::Event::Ready(tx)) => {
                self.html_reader.cache.tx = Some(tx);
            }
            Message::Prepared(html_render::preparation::Event::Prepared(key, frame)) => {
                if let Some(frame) = frame {
                    self.html_reader.cache.insert(key, frame);
                }
            }
            Message::Backend(Event::Found(generation, revision, layout, result))
                if generation == self.html_reader.generation
                    && self
                        .html_reader
                        .frame
                        .as_ref()
                        .is_some_and(|f| f.layout_revision == layout) =>
            {
                self.find_message.accept(revision, result);
            }
            Message::Backend(Event::PlainFound(revision, result)) => {
                self.find_message.accept(revision, result)
            }
            Message::Scale(scale) => self.html_reader.system_scale = scale,
            Message::Backend(Event::Ready(tx)) => self.html_reader.tx = Some(tx),
            Message::Backend(Event::Frame(frame))
                if frame.generation == self.html_reader.generation =>
            {
                // Resource discovery must survive a superseded geometry frame:
                // the worker reports each requested URL only once per document.
                self.html_reader
                    .resources
                    .extend(frame.images.iter().filter(|u| remote_url(u)).cloned());
                if self
                    .html_reader
                    .viewport
                    .is_none_or(|(viewport, top)| !frame.matches_view(viewport, top))
                {
                    return Task::none();
                }
                if self
                    .html_reader
                    .frame
                    .as_ref()
                    .is_some_and(|previous| previous.layout_revision != frame.layout_revision)
                {
                    self.find_message.results = None;
                    self.html_reader.selection.clear();
                    self.html_reader.rectangles.clear();
                }
                self.html_reader.handle = Some(cache::handle(&frame));
                self.html_reader.frame = Some(frame);
            }
            Message::Backend(Event::Selection(id, text, rects, link))
                if id == self.html_reader.generation =>
            {
                self.html_reader.selection = text;
                self.html_reader.rectangles = rects;
                self.html_reader.link = link;
            }
            Message::Backend(Event::Copy(id, text)) if id == self.html_reader.generation => {
                if !text.is_empty() {
                    return iced::clipboard::write(text);
                }
            }
            Message::Backend(Event::Link(id, url))
                if id == self.html_reader.generation && self.dialog.is_none() =>
            {
                if remote_url(&url) {
                    self.html_reader.last_link = Some(url.clone());
                    if !self.demo {
                        return Task::perform(
                            async move {
                                tokio::task::spawn_blocking(move || webbrowser::open(&url))
                                    .await
                                    .map_err(|e| e.to_string())?
                                    .map_err(|e| e.to_string())
                            },
                            |r| super::Message::Html(Message::LinkResult(r)),
                        );
                    }
                } else if let Ok(url) = url::Url::parse(&url)
                    && url.scheme() == "mailto"
                {
                    self.open(Dialog::Compose);
                    // Treat the link as addresses only; do not accept hidden
                    // recipients/headers or attachments supplied by a message.
                    self.fields.insert(
                        "to",
                        percent_encoding::percent_decode_str(url.path())
                            .decode_utf8_lossy()
                            .into_owned(),
                    );
                }
            }
            Message::Backend(Event::Error(id, error)) if id == self.html_reader.generation => {
                self.html_reader.error = Some(error)
            }
            Message::Backend(_) => {}
            Message::Plain(plain) => {
                self.html_reader.plain = self
                    .detail
                    .as_ref()
                    .filter(|_| plain)
                    .map(|d| d.summary.id.clone());
            }
            Message::Quotes => {
                if let Some(detail) = &self.detail {
                    self.html_reader.quote_override =
                        Some((detail.summary.id.clone(), !self.html_quotes_hidden(detail)));
                }
            }
            Message::Input(command) => {
                if let Input::View(id, size, top) = command {
                    if id == self.html_reader.generation
                        && self.html_reader.viewport != Some((size, top))
                    {
                        self.html_reader.viewport = Some((size, top));
                        if self.html_reader.pending
                            && let Some(request) = self
                                .detail
                                .as_ref()
                                .and_then(|d| self.html_preparation(d, size))
                        {
                            let state = &mut self.html_reader;
                            state.pending = false;
                            if top == 0.
                                && let Some((frame, handle)) = state.cache.get(&request.key, id)
                            {
                                state
                                    .resources
                                    .extend(frame.images.iter().filter(|u| remote_url(u)).cloned());
                                state.frame = Some(frame);
                                state.handle = Some(handle);
                            }
                            state
                                .supplied
                                .extend(request.source.images.iter().map(|(url, _)| url.clone()));
                            state.queue.push_front(Input::Load {
                                generation: id,
                                body: request.source.body,
                                viewport: size,
                                font_size: request.source.font_size,
                                hide_quotes: request.source.hide_quotes,
                                images: request.source.images,
                            });
                        }
                        self.html_reader.enqueue(command);
                    }
                } else {
                    self.html_reader.enqueue(command);
                }
            }
            Message::Scroll(id, amount) if id == self.html_reader.generation => {
                let target = if self.conversation_visible() {
                    "conversation-reader"
                } else {
                    "message-reader"
                };
                return widget::operation::scroll_by(
                    target,
                    widget::scrollable::AbsoluteOffset { x: 0., y: amount },
                );
            }
            Message::ScrollEnd(id, end) if id == self.html_reader.generation => {
                let target = if self.conversation_visible() {
                    "conversation-reader"
                } else {
                    "message-reader"
                };
                return widget::operation::snap_to(
                    target,
                    widget::scrollable::RelativeOffset {
                        x: 0.,
                        y: if end { 1. } else { 0. },
                    },
                );
            }
            Message::Scroll(..) | Message::ScrollEnd(..) => {}
            Message::Pump => self.html_reader.pumping = false,
            Message::LinkResult(Err(error)) => {
                self.notice(format!("The link could not be opened: {error}"), true)
            }
            Message::LinkResult(Ok(())) => {}
        }
        Task::none()
    }
    fn load_html_images(&mut self) {
        if self.html_reader.key.as_ref().is_none_or(|key| !key.4) {
            return;
        }
        let urls: Vec<_> = self.html_reader.resources.iter().cloned().collect();
        let mut requests = Vec::new();
        for url in urls {
            if self.html_reader.supplied.contains(&url) {
                continue;
            }
            if let Some(bytes) = self
                .remote_bytes
                .iter()
                .find(|(u, _)| u == &url)
                .map(|(_, bytes)| bytes.clone())
            {
                if self.html_reader.supplied.insert(url.clone()) {
                    self.html_reader
                        .enqueue(Input::Image(self.html_reader.generation, url, bytes));
                }
            } else if !self.image_errors.contains_key(&url)
                && !self.requested_images.contains(&url)
                && requests.len() + self.requested_images.len() < 8
            {
                requests.push(url);
            }
        }
        if !requests.is_empty() && self.try_command(Command::LoadImages(requests.clone())) {
            self.requested_images.extend(requests);
        }
    }
    fn html_scale(&self) -> f32 {
        ((self.html_reader.system_scale.max(1.) * self.preferences.interface_scale as f32).round()
            / 100.)
            .clamp(0.5, 4.)
    }
    pub(super) fn html_canvas(&self) -> Element<'_, super::Message> {
        canvas::Canvas::new(
            &self.html_reader,
            self.dialog.is_none() && self.context_menu.is_none(),
            self.html_scale(),
        )
        .into()
    }
}
fn remote_url(input: &str) -> bool {
    url::Url::parse(input).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https") && u.username().is_empty() && u.password().is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn app() -> App {
        let store = crate::store::Store::memory().unwrap();
        let mail = parse_mail("fixture", "1", "INBOX", b"From: Example <test@example.test>\r\nContent-Type: text/html\r\n\r\n<p>Visible mail</p><img src=\"https://example.test/image.webp\">".to_vec(), true, false).unwrap();
        let id = mail.summary.id.clone();
        store.upsert(vec![mail]).await.unwrap();
        let (mut app, _) = App::new();
        app.selected = Some(id.clone());
        app.detail = Some(Arc::new(store.detail(id).await.unwrap()));
        app
    }
    fn frame(generation: u64, viewport: Viewport, scroll: f32) -> Arc<html_render::Frame> {
        Arc::new(html_render::Frame {
            generation,
            layout_revision: 1,
            viewport,
            pixels: Arc::from([0; 4]),
            width: 1,
            height: 1,
            content_height: 2000.,
            content_width: viewport.width as f32,
            scroll,
            pan: 0.,
            images: vec!["https://example.test/image.webp".into()],
        })
    }
    #[tokio::test]
    async fn html_waits_for_native_geometry_and_keeps_find_behind_load() {
        let mut app = app().await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        app.html_reader.tx = Some(tx);
        let _ = app.prepare_html();
        let id = app.html_reader.generation;
        app.html_reader
            .enqueue(Input::Find(id, 1, "Visible".into(), false));
        let _ = app.html_reader.pump();
        assert!(
            rx.try_recv().is_err(),
            "No provisional render or lost Find before geometry"
        );
        let viewport = Viewport {
            width: 723,
            height: 387,
            scale: 1.5,
        };
        let _ = app.handle_html(Message::Input(Input::View(id, viewport, 0.)));
        let _ = app.html_reader.pump();
        assert!(
            matches!(rx.try_recv().unwrap(), Input::Load { viewport: size, images, .. } if size == viewport && images.is_empty())
        );
        assert!(matches!(rx.try_recv().unwrap(), Input::Find(_, 1, ..)));
        assert!(matches!(rx.try_recv().unwrap(), Input::View(_, size, 0.) if size == viewport));
    }
    #[tokio::test]
    async fn html_rejects_old_geometry_but_retains_resource_discovery() {
        let mut app = app().await;
        let _ = app.prepare_html();
        let id = app.html_reader.generation;
        let old = Viewport {
            width: 600,
            height: 600,
            scale: 1.,
        };
        let size = Viewport {
            width: 723,
            height: 387,
            scale: 1.5,
        };
        let _ = app.handle_html(Message::Input(Input::View(id, size, 0.)));
        let _ = app.handle_html(Message::Backend(html_render::Event::Frame(frame(
            id, old, 0.,
        ))));
        assert!(app.html_reader.frame.is_none());
        assert!(
            app.html_reader
                .resources
                .contains("https://example.test/image.webp")
        );
        let _ = app.handle_html(Message::Backend(html_render::Event::Frame(frame(
            id, size, 0.,
        ))));
        assert!(app.html_reader.view_current());
        let _ = app.handle_html(Message::Input(Input::View(id, size, 300.)));
        let _ = app.handle_html(Message::Backend(html_render::Event::Frame(frame(
            id, size, 0.,
        ))));
        assert!(!app.html_reader.view_current());
        let _ = app.handle_html(Message::Backend(html_render::Event::Frame(frame(
            id, size, 300.,
        ))));
        assert!(app.html_reader.view_current());
        let _ = app.handle_html(Message::Backend(html_render::Event::Frame(frame(
            id - 1,
            size,
            600.,
        ))));
        assert!(app.html_reader.view_current());
    }
    #[tokio::test]
    async fn html_prepared_frames_respect_images_fonts_quotes_and_viewport_changes() {
        let mut app = app().await;
        let size = Viewport {
            width: 600,
            height: 400,
            scale: 1.,
        };
        app.preferences.image_policy = ImagePolicy::AllowAll;
        app.remote_bytes
            .push_back(("https://example.test/image.webp".into(), Arc::from([1; 4])));
        let allowed = app
            .html_preparation(app.detail.as_ref().unwrap(), size)
            .unwrap();
        assert_eq!(allowed.source.images.len(), 1);
        app.html_reader
            .cache
            .insert(allowed.key.clone(), frame(0, size, 0.));
        assert!(
            app.html_reader
                .cache
                .get(&allowed.key, 3)
                .is_some_and(|(f, _)| f.generation == 3)
        );
        app.preferences.image_policy = ImagePolicy::BlockAll;
        let blocked = app
            .html_preparation(app.detail.as_ref().unwrap(), size)
            .unwrap();
        assert!(blocked.source.images.is_empty());
        assert!(app.html_reader.cache.get(&blocked.key, 3).is_none());
        app.preferences.image_policy = ImagePolicy::AllowAll;
        app.html_reader.cache.image_revision += 1;
        let changed = app
            .html_preparation(app.detail.as_ref().unwrap(), size)
            .unwrap();
        assert!(app.html_reader.cache.get(&changed.key, 3).is_none());
        for key in [
            html_render::preparation::Key {
                font_size: 20,
                ..allowed.key.clone()
            },
            html_render::preparation::Key {
                hide_quotes: !allowed.key.hide_quotes,
                ..allowed.key.clone()
            },
            html_render::preparation::Key {
                viewport: Viewport { width: 300, ..size },
                ..allowed.key.clone()
            },
            html_render::preparation::Key {
                signature: [0; 32],
                ..allowed.key.clone()
            },
        ] {
            assert!(app.html_reader.cache.get(&key, 3).is_none());
        }
    }
    #[tokio::test]
    async fn metadata_refresh_preserves_html_but_policy_revocation_and_plain_mode_clear_it() {
        let mut app = app().await;
        let _ = app.prepare_html();
        let generation = app.html_reader.generation;
        app.html_reader.selection = "Visible mail".into();
        Arc::make_mut(app.detail.as_mut().unwrap()).summary.starred = true;
        let _ = app.prepare_html();
        assert_eq!(app.html_reader.generation, generation);
        assert_eq!(app.html_reader.selection, "Visible mail");
        app.preferences.image_policy = ImagePolicy::AllowAll;
        let _ = app.prepare_html();
        assert!(app.html_reader.generation > generation);
        app.preferences.image_policy = ImagePolicy::BlockAll;
        let _ = app.prepare_html();
        let current = app.html_reader.generation;
        assert!(app.html_reader.resources.is_empty());
        assert!(app.html_reader.handle.is_none());
        let _ = app.handle_html(Message::Backend(html_render::Event::Selection(
            generation,
            "Obsolete message".into(),
            vec![],
            true,
        )));
        assert!(app.html_reader.selection.is_empty());
        assert!(!app.html_reader.link);
        let _ = app.handle_html(Message::Plain(true));
        let _ = app.prepare_html();
        assert!(app.html_reader.key.is_none());
        assert!(app.html_reader.generation > current);
        assert!(!app.formatted(app.detail.as_ref().unwrap()));
        assert!(app.requested_images.is_empty());
    }
    #[test]
    fn queued_geometry_coalesces_without_losing_selection_boundaries() {
        let mut state = State::default();
        state.enqueue(Input::Pointer(1, html_render::Pointer::Down, 1., 1.));
        for y in 0..1000 {
            state.enqueue(Input::Pointer(1, html_render::Pointer::Move, 10., y as f32));
        }
        state.enqueue(Input::Pointer(1, html_render::Pointer::Up, 10., 999.));
        state.enqueue(Input::Copy(1));
        for top in 0..1000 {
            state.enqueue(Input::View(
                1,
                Viewport {
                    width: 400,
                    height: 200,
                    scale: 1.,
                },
                top as f32,
            ));
        }
        assert_eq!(state.queue.len(), 5);
        assert!(matches!(
            state.queue[0],
            Input::Pointer(1, html_render::Pointer::Down, ..)
        ));
        assert!(matches!(
            state.queue[1],
            Input::Pointer(1, html_render::Pointer::Move, _, 999.)
        ));
        assert!(matches!(
            state.queue[2],
            Input::Pointer(1, html_render::Pointer::Up, ..)
        ));
        assert!(matches!(state.queue[3], Input::Copy(1)));
        assert!(matches!(state.queue[4], Input::View(1, _, 999.)));
    }
}
