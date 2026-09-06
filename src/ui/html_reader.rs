//! Small UI-side state only. DOM, fonts, image decoding and rasterization belong
//! to the renderer worker. Geometry/hover requests coalesce under backpressure.
mod canvas;
use super::*;
use crate::html_render::{self, Input, Viewport};

#[derive(Debug, Clone)]
pub enum Message {
    Backend(html_render::Event),
    Input(Input),
    Pump,
    Plain(bool),
    Quotes,
    LinkResult(Result<(), String>),
    Scale(f32),
}
#[derive(Default)]
pub(super) struct State {
    tx: Option<tokio::sync::mpsc::Sender<Input>>,
    queue: VecDeque<Input>,
    pumping: bool,
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
    fn enqueue(&mut self, command: Input) {
        // A new view supersedes unprocessed geometry; moves supersede adjacent
        // hover/drag positions, but never a Down/Up/Copy boundary.
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
            state.enqueue(Input::Clear);
            if let Some(detail) = detail {
                let viewport = Viewport {
                    width: 600,
                    height: 600,
                    scale: state.key.as_ref().unwrap().5 as f32 / 100.,
                };
                let hide_quotes = state.key.as_ref().unwrap().3;
                state.enqueue(Input::Load {
                    generation: state.generation,
                    body: detail.html.clone().unwrap(),
                    viewport,
                    font_size: self.preferences.reader_font_size,
                    hide_quotes,
                });
            }
        }
        self.load_html_images();
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
            Message::Scale(scale) => self.html_reader.system_scale = scale,
            Message::Backend(Event::Ready(tx)) => self.html_reader.tx = Some(tx),
            Message::Backend(Event::Frame(frame))
                if frame.generation == self.html_reader.generation =>
            {
                if self
                    .html_reader
                    .frame
                    .as_ref()
                    .is_some_and(|previous| previous.layout_revision != frame.layout_revision)
                {
                    self.html_reader.selection.clear();
                    self.html_reader.rectangles.clear();
                }
                self.html_reader
                    .resources
                    .extend(frame.images.iter().filter(|u| remote_url(u)).cloned());
                self.html_reader.handle = Some(widget::image::Handle::from_rgba(
                    frame.width,
                    frame.height,
                    bytes::Bytes::from_owner(frame.pixels.clone()),
                ));
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
                        self.html_reader.enqueue(command);
                    }
                } else {
                    self.html_reader.enqueue(command);
                }
            }
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
