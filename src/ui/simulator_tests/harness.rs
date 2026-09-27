//! Drives `App` like the iced runtime does, without a window or display.
//!
//! `iced_test::Simulator` rebuilds widget state for every instance, which loses
//! text focus and scroll positions between messages. The harness therefore keeps
//! one `UserInterface` cache across rebuilds, as the real runtime does, and uses
//! `iced_test` selectors, input events and `Simulator` snapshots on top of it.
//!
//! Background work is the real demo engine on an in-memory fixture store. Its
//! events and every `Task` the app returns are forwarded into bounded queues and
//! handled on the test thread. Assertions wait for an observed condition with a
//! deadline, never for a fixed time.
use super::*;
use futures::{FutureExt, StreamExt};
use iced::Point;
use iced::advanced::widget::Operation;
use iced_test::core::SmolStr;
use iced_test::core::renderer::Headless;
use iced_test::runtime::{self as iced_runtime, UserInterface, user_interface};
use iced_test::selector::{Bounded, Selector};
use keyboard::key::Named;
use tokio::sync::mpsc;

/// Bounds a hung scenario; passing checks return as soon as they hold.
const DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);
const NOTO_REGULAR: &[u8] = include_bytes!("../../../assets/NotoSans-Regular.ttf");
const NOTO_SEMIBOLD: &[u8] = include_bytes!("../../../assets/NotoSans-SemiBold.ttf");

type Ui<'a> = UserInterface<'a, Message, Theme, iced::Renderer>;

#[derive(Default)]
struct Clipboard(Option<String>);

impl iced::advanced::Clipboard for Clipboard {
    fn read(&self, _kind: iced::advanced::clipboard::Kind) -> Option<String> {
        self.0.clone()
    }

    fn write(&mut self, _kind: iced::advanced::clipboard::Kind, contents: String) {
        self.0 = Some(contents);
    }
}

pub struct Harness {
    app: App,
    renderer: iced::Renderer,
    cache: Option<user_interface::Cache>,
    clipboard: Clipboard,
    size: Size,
    cursor: iced::mouse::Cursor,
    window: iced::window::Id,
    actions: mpsc::UnboundedReceiver<iced_runtime::Action<Message>>,
    action_sender: mpsc::UnboundedSender<iced_runtime::Action<Message>>,
    backend: mpsc::Receiver<engine::Event>,
    tasks: tokio::task::JoinSet<()>,
}

fn load_fonts() {
    static FONTS: std::sync::Once = std::sync::Once::new();
    FONTS.call_once(|| {
        let mut fonts = iced_test::renderer::graphics::text::font_system()
            .write()
            .expect("font system");
        fonts.load_font(NOTO_REGULAR.into());
        fonts.load_font(NOTO_SEMIBOLD.into());
    });
}

impl Harness {
    /// The standard native fixture window.
    pub async fn start() -> Self {
        Self::with_size(1440., 920.).await
    }

    /// Starts the app on the demo engine and waits for its first mail page,
    /// matching the native harness's `desktop.start`.
    pub async fn with_size(width: f32, height: f32) -> Self {
        load_fonts();
        let renderer = <iced::Renderer as Headless>::new(
            iced::Font::with_name("Noto Sans"),
            iced::Pixels(16.),
            None,
        )
        .await
        .expect("headless renderer");
        let (app, _system_theme) = App::new();
        let (backend_sender, backend) = mpsc::channel(crate::model::CHANNEL_CAPACITY);
        let (action_sender, actions) = mpsc::unbounded_channel();
        let mut tasks = tokio::task::JoinSet::new();
        tasks.spawn(async move {
            let mut events = std::pin::pin!(engine::subscription(&true));
            while let Some(event) = events.next().await {
                if backend_sender.send(event).await.is_err() {
                    break;
                }
            }
        });
        let size = Size::new(width, height);
        let mut harness = Self {
            app,
            renderer,
            cache: None,
            clipboard: Clipboard::default(),
            size,
            cursor: iced::mouse::Cursor::Unavailable,
            window: iced::window::Id::unique(),
            actions,
            action_sender,
            backend,
            tasks,
        };
        harness.dispatch(Message::Resize(size));
        harness.redraw();
        harness.expect("ready", true).await;
        harness.expect("page_loaded", true).await;
        harness
    }

    /// The observation the native harness writes to its state file.
    pub fn state(&self) -> serde_json::Value {
        self.app.test_observation()
    }

    /// Waits until the observation at `path` equals `value`, like MCP `check`.
    pub async fn expect(&mut self, path: &str, value: impl Into<serde_json::Value>) {
        let value = value.into();
        self.expect_that(path, |observed| observed == &value).await;
    }

    /// Waits until the string or array at `path` contains `needle`.
    pub async fn expect_contains(&mut self, path: &str, needle: &str) {
        self.expect_that(path, |observed| match observed {
            serde_json::Value::String(text) => text.contains(needle),
            serde_json::Value::Array(items) => items.iter().any(|item| item == needle),
            _ => false,
        })
        .await;
    }

    /// Waits until the observation at `path` differs from `value`.
    pub async fn expect_ne(&mut self, path: &str, value: impl Into<serde_json::Value>) {
        let value = value.into();
        self.expect_that(path, |observed| observed != &value).await;
    }

    /// Waits until the number at `path` is at least `minimum`.
    pub async fn expect_at_least(&mut self, path: &str, minimum: f64) {
        self.expect_that(path, |observed| {
            observed.as_f64().is_some_and(|value| value >= minimum)
        })
        .await;
    }

    /// Waits until `accept` holds for the observation at `path`.
    pub async fn expect_that(&mut self, path: &str, accept: impl Fn(&serde_json::Value) -> bool) {
        let deadline = tokio::time::Instant::now() + DEADLINE;
        // The app's one-second `Tick` subscription, for timeouts and retries.
        let mut tick = tokio::time::interval_at(
            tokio::time::Instant::now() + std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(1),
        );
        loop {
            self.pump();
            let observed = lookup(&self.state(), path);
            if accept(&observed) {
                return;
            }
            tokio::select! {
                Some(action) = self.actions.recv() => self.perform(action),
                Some(event) = self.backend.recv() => self.handle(Message::Backend(event)),
                _ = tick.tick() => self.handle(Message::Tick),
                () = tokio::time::sleep_until(deadline) => {
                    panic!("{path} never matched; last observed {observed}")
                }
            }
        }
    }

    /// Clicks the centre of the first visible text equal to `text`.
    pub async fn click_text(&mut self, text: &str) {
        let bounds = self.text_bounds(text);
        let Some(bounds) = bounds.first() else {
            panic!("no visible text {text:?}");
        };
        self.click(bounds.center()).await;
    }

    /// Visible bounds of every text equal to `text`, in widget-tree order.
    fn text_bounds(&mut self, text: &str) -> Vec<iced::Rectangle> {
        let mut operation = text.find_all();
        let cache = self.cache.take().unwrap_or_default();
        let mut ui = Ui::build(self.app.view(), self.size, cache, &mut self.renderer);
        ui.operate(
            &self.renderer,
            &mut iced::advanced::widget::operation::black_box(&mut operation),
        );
        self.cache = Some(ui.into_cache());
        match operation.finish() {
            iced::advanced::widget::operation::Outcome::Some(found) => {
                found.iter().filter_map(Bounded::visible_bounds).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Moves, presses and releases the left button, like an XTest click.
    pub async fn click(&mut self, position: Point) {
        self.press_button(position, iced::mouse::Button::Left).await;
    }

    pub async fn click_at(&mut self, x: f32, y: f32) {
        self.click(Point::new(x, y)).await;
    }

    pub async fn right_click_at(&mut self, x: f32, y: f32) {
        self.press_button(Point::new(x, y), iced::mouse::Button::Right)
            .await;
    }

    /// Moves the pointer without pressing, like the native `hover` action.
    pub async fn hover(&mut self, x: f32, y: f32) {
        self.move_to(Point::new(x, y));
        self.settle().await;
    }

    /// Scrolls `amount` wheel notches downwards at the pointer, like xdotool.
    pub async fn scroll(&mut self, amount: u32) {
        for _ in 0..amount {
            self.deliver(iced::Event::Mouse(iced::mouse::Event::WheelScrolled {
                delta: iced::mouse::ScrollDelta::Lines { x: 0., y: -1. },
            }));
        }
        self.settle().await;
    }

    /// Drags with the left button in ten steps, as the native harness does.
    pub async fn drag(&mut self, from: (f32, f32), to: (f32, f32)) {
        use iced::mouse::{Button, Event};
        self.move_to(Point::new(from.0, from.1));
        self.deliver(iced::Event::Mouse(Event::ButtonPressed(Button::Left)));
        for step in 1..=10u8 {
            let fraction = f32::from(step) / 10.;
            self.move_to(Point::new(
                from.0 + (to.0 - from.0) * fraction,
                from.1 + (to.1 - from.1) * fraction,
            ));
        }
        self.deliver(iced::Event::Mouse(Event::ButtonReleased(Button::Left)));
        self.settle().await;
    }

    async fn press_button(&mut self, position: Point, button: iced::mouse::Button) {
        self.move_to(position);
        self.deliver(iced::Event::Mouse(iced::mouse::Event::ButtonPressed(
            button,
        )));
        self.deliver(iced::Event::Mouse(iced::mouse::Event::ButtonReleased(
            button,
        )));
        self.settle().await;
    }

    fn move_to(&mut self, position: Point) {
        self.deliver(iced::Event::Mouse(iced::mouse::Event::CursorMoved {
            position,
        }));
    }

    /// Types each character as its own key press, as xdotool does.
    pub async fn type_text(&mut self, text: &str) {
        for character in text.chars() {
            let text = SmolStr::new(character.to_string());
            let key = Key::Character(text.clone());
            self.press(key, keyboard::Modifiers::empty(), Some(text));
        }
        self.settle().await;
    }

    /// Sends one xdotool-style chord such as `ctrl+comma`, `alt+m` or `Escape`.
    pub async fn key(&mut self, chord: &str) {
        let (modifiers, key, text) = parse_chord(chord);
        if !modifiers.is_empty() {
            self.deliver(iced::Event::Keyboard(keyboard::Event::ModifiersChanged(
                modifiers,
            )));
        }
        self.press(key, modifiers, text);
        if !modifiers.is_empty() {
            self.deliver(iced::Event::Keyboard(keyboard::Event::ModifiersChanged(
                keyboard::Modifiers::empty(),
            )));
        }
        self.settle().await;
    }

    /// Renders the current view with `iced_test::Simulator`.
    ///
    /// Pixel hashes depend on the host's fallback fonts, so scenarios compare
    /// snapshots from the same run rather than committing reference images.
    pub fn snapshot(&self) -> iced_test::simulator::Snapshot {
        let settings = iced::Settings {
            default_font: iced::Font::with_name("Noto Sans"),
            ..Default::default()
        };
        let mut simulator = iced_test::Simulator::with_size(settings, self.size, self.app.view());
        simulator
            .snapshot(&self.app.theme())
            .expect("simulator snapshot")
    }

    fn press(&mut self, key: Key, modifiers: keyboard::Modifiers, text: Option<SmolStr>) {
        let physical =
            keyboard::key::Physical::Unidentified(keyboard::key::NativeCode::Unidentified);
        self.deliver(iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key.clone(),
            physical_key: physical,
            location: keyboard::Location::Standard,
            modifiers,
            repeat: false,
            text,
        }));
        self.deliver(iced::Event::Keyboard(keyboard::Event::KeyReleased {
            key: key.clone(),
            modified_key: key,
            physical_key: physical,
            location: keyboard::Location::Standard,
            modifiers,
        }));
    }

    /// Handles work that is already queued, without waiting for more.
    async fn settle(&mut self) {
        tokio::task::yield_now().await;
        self.pump();
    }

    fn pump(&mut self) {
        while self.tasks.try_join_next().is_some() {}
        loop {
            if let Ok(action) = self.actions.try_recv() {
                self.perform(action);
            } else if let Ok(event) = self.backend.try_recv() {
                self.handle(Message::Backend(event));
            } else {
                return;
            }
        }
    }

    /// Delivers one native event to the widget tree, then its messages.
    fn deliver(&mut self, event: iced::Event) {
        if let iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) = event {
            self.cursor = iced::mouse::Cursor::Available(position);
        }
        let mut messages = Vec::new();
        // The app's `event::listen_with` subscription sees modifier changes.
        if let iced::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            messages.push(Message::Modifiers(modifiers));
        }
        let cursor = self.cursor;
        let cache = self.cache.take().unwrap_or_default();
        let mut ui = Ui::build(self.app.view(), self.size, cache, &mut self.renderer);
        let _ = ui.update(
            std::slice::from_ref(&event),
            cursor,
            &mut self.renderer,
            &mut self.clipboard,
            &mut messages,
        );
        self.cache = Some(ui.into_cache());
        for message in messages {
            self.dispatch(message);
        }
        self.redraw();
    }

    /// Updates the app with one message and redraws, like one runtime cycle.
    fn handle(&mut self, message: Message) {
        self.dispatch(message);
        self.redraw();
    }

    fn dispatch(&mut self, message: Message) {
        let task = self.app.update(message);
        let Some(mut stream) = iced_runtime::task::into_stream(task) else {
            return;
        };
        // Actions that are ready now, such as focus operations, run before the
        // next input; only pending work continues in the background.
        let mut ready = Vec::new();
        let finished = loop {
            match stream.next().now_or_never() {
                Some(Some(action)) => ready.push(action),
                Some(None) => break true,
                None => break false,
            }
        };
        if !finished {
            let sender = self.action_sender.clone();
            self.tasks.spawn(async move {
                while let Some(action) = stream.next().await {
                    if sender.send(action).is_err() {
                        break;
                    }
                }
            });
        }
        for action in ready {
            self.perform(action);
        }
    }

    /// Requests and draws a frame; widgets record layout while drawing.
    fn redraw(&mut self) {
        let theme = self.app.theme();
        let style = iced::theme::Base::base(&theme);
        let cursor = self.cursor;
        let mut messages = Vec::new();
        let cache = self.cache.take().unwrap_or_default();
        let mut ui = Ui::build(self.app.view(), self.size, cache, &mut self.renderer);
        let _ = ui.update(
            &[iced::Event::Window(iced::window::Event::RedrawRequested(
                iced::time::Instant::now(),
            ))],
            cursor,
            &mut self.renderer,
            &mut self.clipboard,
            &mut messages,
        );
        ui.draw(
            &mut self.renderer,
            &theme,
            &iced::advanced::renderer::Style {
                text_color: style.text_color,
            },
            cursor,
        );
        self.cache = Some(ui.into_cache());
        // Redraw messages are handled on the next cycle, as the runtime does.
        for message in messages {
            self.dispatch(message);
        }
    }

    fn perform(&mut self, action: iced_runtime::Action<Message>) {
        match action {
            iced_runtime::Action::Output(message) => self.handle(message),
            iced_runtime::Action::Widget(operation) => self.operate(operation),
            iced_runtime::Action::Window(action) => self.window(action),
            // Clipboard, system, image and exit requests have no headless effect.
            _ => {}
        }
    }

    fn operate(&mut self, operation: Box<dyn Operation>) {
        let cache = self.cache.take().unwrap_or_default();
        let mut ui = Ui::build(self.app.view(), self.size, cache, &mut self.renderer);
        let mut next = Some(operation);
        while let Some(mut current) = next.take() {
            ui.operate(&self.renderer, current.as_mut());
            if let iced::advanced::widget::operation::Outcome::Chain(chained) = current.finish() {
                next = Some(chained);
            }
        }
        self.cache = Some(ui.into_cache());
        self.redraw();
    }

    fn window(&self, action: iced_runtime::window::Action) {
        use iced_runtime::window::Action;
        match action {
            Action::GetOldest(sender) | Action::GetLatest(sender) => {
                let _ = sender.send(Some(self.window));
            }
            Action::GetSize(_, sender) => {
                let _ = sender.send(self.size);
            }
            Action::GetScaleFactor(_, sender) => {
                let _ = sender.send(1.);
            }
            _ => {}
        }
    }
}

fn lookup(state: &serde_json::Value, path: &str) -> serde_json::Value {
    path.split('.')
        .try_fold(state, |value, part| match part.parse::<usize>() {
            Ok(index) => value.get(index),
            Err(_) => value.get(part),
        })
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

/// Maps xdotool key names onto iced keys.
fn parse_chord(chord: &str) -> (keyboard::Modifiers, Key, Option<SmolStr>) {
    let mut modifiers = keyboard::Modifiers::empty();
    let mut parts: Vec<&str> = chord.split('+').collect();
    let name = parts.pop().unwrap_or_default();
    for part in parts {
        modifiers |= match part {
            "ctrl" => keyboard::Modifiers::CTRL,
            "alt" => keyboard::Modifiers::ALT,
            "shift" => keyboard::Modifiers::SHIFT,
            "super" => keyboard::Modifiers::LOGO,
            other => panic!("unknown modifier {other}"),
        };
    }
    let named = match name {
        "Escape" => Some(Named::Escape),
        "Return" => Some(Named::Enter),
        "BackSpace" => Some(Named::Backspace),
        "Delete" => Some(Named::Delete),
        "Tab" => Some(Named::Tab),
        "Up" => Some(Named::ArrowUp),
        "Down" => Some(Named::ArrowDown),
        "Left" => Some(Named::ArrowLeft),
        "Right" => Some(Named::ArrowRight),
        "Home" => Some(Named::Home),
        "End" => Some(Named::End),
        "Page_Up" => Some(Named::PageUp),
        "Page_Down" => Some(Named::PageDown),
        "F5" => Some(Named::F5),
        "F10" => Some(Named::F10),
        _ => None,
    };
    if let Some(named) = named {
        return (modifiers, Key::Named(named), None);
    }
    let character = match name {
        "comma" => ",",
        "period" => ".",
        "slash" => "/",
        "space" => " ",
        other => other,
    };
    let character = if modifiers.shift() {
        character.to_uppercase()
    } else {
        character.to_owned()
    };
    let text = (!modifiers.control() && !modifiers.alt() && !modifiers.logo())
        .then(|| SmolStr::new(&character));
    (modifiers, Key::Character(character.into()), text)
}
