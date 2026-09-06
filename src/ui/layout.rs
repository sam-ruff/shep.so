//! Layout changes stay on the UI thread; persistence uses the ordered worker.
use super::*;
use iced::advanced::Renderer as _;
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, mouse, renderer,
    widget::{Tree, tree},
};
use iced::{Length, Point, Rectangle, Renderer};

impl App {
    pub(super) fn max_sidebar_width(&self) -> f32 {
        (self.size.width / (self.preferences.interface_scale as f32 / 100.) - 520.)
            .clamp(160., 480.)
    }
    pub(super) fn sidebar_width(&self) -> f32 {
        self.preferences
            .sidebar_width
            .unwrap_or(if self.size.width < 1100. { 200. } else { 222. })
            .clamp(160., self.max_sidebar_width())
    }
    pub(super) fn debounce_layout(&mut self) -> Task<Message> {
        self.preference_sync.changed();
        self.layout_generation += 1;
        let generation = self.layout_generation;
        Task::perform(
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                generation
            },
            Message::SaveLayout,
        )
    }
    pub(super) fn flush_pane_resize(&mut self) -> bool {
        let Some(event) = self.pending_resize.take() else {
            return false;
        };
        let ratio = event.ratio.clamp(0.2, 0.7);
        self.panes.resize(event.split, ratio);
        self.preferences.reader_split = ratio;
        self.preference_sync.changed();
        true
    }
    pub(super) fn persist_preferences(&mut self, request: u64, preferences: Preferences) {
        // One coalesced retry slot prevents lost settings on a full queue without
        // accumulating an unbounded backlog while the disk is busy.
        if self.try_command(Command::SavePreferences(request, preferences.clone())) {
            self.pending_preference_save = None;
        } else {
            self.pending_preference_save = Some((request, preferences));
        }
    }
}

#[derive(Default)]
struct Drag {
    origin: Option<(Point, f32)>,
}
pub(super) struct SidebarDivider(pub f32);
fn hit(bounds: Rectangle) -> Rectangle {
    Rectangle {
        x: bounds.x - 4.,
        width: 9.,
        ..bounds
    }
}
impl Widget<Message, Theme, Renderer> for SidebarDivider {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Drag>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(Drag::default())
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(1.), Length::Fill)
    }
    fn layout(&mut self, _: &mut Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(Size::new(1., limits.max().height))
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _: &Renderer,
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _: &Rectangle,
    ) {
        let drag = tree.state.downcast_mut::<Drag>();
        match event {
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if cursor.is_over(hit(layout.bounds())) =>
            {
                drag.origin = cursor.position().map(|p| (p, self.0));
                shell.capture_event();
                shell.request_redraw();
            }
            iced::Event::Mouse(mouse::Event::CursorMoved { position }) if drag.origin.is_some() => {
                let (origin, width) = drag.origin.unwrap();
                shell.publish(Message::SidebarResize(width + position.x - origin.x));
                shell.capture_event();
            }
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if drag.origin.take().is_some() =>
            {
                shell.capture_event();
                shell.request_redraw();
            }
            iced::Event::Window(iced::window::Event::Unfocused) => drag.origin = None,
            _ => {}
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _: &Rectangle,
        _: &Renderer,
    ) -> mouse::Interaction {
        if tree.state.downcast_ref::<Drag>().origin.is_some()
            || cursor.is_over(hit(layout.bounds()))
        {
            mouse::Interaction::ResizingHorizontally
        } else {
            mouse::Interaction::None
        }
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _: &Rectangle,
    ) {
        let active = tree.state.downcast_ref::<Drag>().origin.is_some()
            || cursor.is_over(hit(layout.bounds()));
        renderer.fill_quad(
            renderer::Quad {
                bounds: layout.bounds(),
                ..Default::default()
            },
            if active {
                colors(theme).accent
            } else {
                colors(theme).border
            },
        );
    }
}
impl From<SidebarDivider> for Element<'_, Message> {
    fn from(value: SidebarDivider) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_flushes_latest_drag_and_waits_for_acknowledgment() {
        let (mut app, _) = App::new();
        let _ = app.handle(Message::SidebarResize(310.));
        app.pending_resize = Some(widget::pane_grid::ResizeEvent {
            split: app.reader_split,
            ratio: 0.52,
        });
        let window = iced::window::Id::unique();
        let _ = app.handle(Message::WindowClose(window));
        assert_eq!(app.pending_close, Some(window));
        assert_eq!(app.preferences.reader_split, 0.52);
        let (request, saved) = app.pending_preference_save.clone().unwrap();
        assert_eq!(saved.sidebar_width, Some(310.));
        let _ = app.handle(Message::SidebarResize(330.));
        let _ = app.handle(Message::Backend(Event::PreferencesSaved(
            request,
            Arc::new(PreferenceSnapshot {
                revision: 1,
                value: saved,
            }),
        )));
        assert!(app.preference_sync.dirty());
        assert_eq!(app.pending_close, Some(window));
        assert_eq!(app.preferences.sidebar_width, Some(330.));
        let _ = app.handle(Message::Backend(Event::PreferencesSaveFailed(
            request,
            "Disk unavailable".into(),
        )));
        assert!(app.pending_close.is_none());
        assert!(app.preference_sync.dirty());
        assert!(app.notice.as_ref().unwrap().0.contains("Disk unavailable"));
    }
    #[test]
    fn save_toast_requires_current_ack_and_invalid_edits_do_not_confirm() {
        let (mut app, _) = App::new();
        app.settings_fields();
        let _ = app.handle(Message::SavePreferences);
        assert!(app.saved_toast.is_none());
        let request = app.confirm_save.unwrap();
        let saved = app.preferences.clone();
        let _ = app.handle(Message::SidebarResize(300.));
        let _ = app.handle(Message::Backend(Event::PreferencesSaved(
            request,
            Arc::new(PreferenceSnapshot {
                revision: 1,
                value: saved,
            }),
        )));
        assert!(app.saved_toast.is_none());
        let request = app.preference_sync.generation();
        let saved = app.preferences.clone();
        let _ = app.handle(Message::Backend(Event::PreferencesSaved(
            request,
            Arc::new(PreferenceSnapshot {
                revision: 2,
                value: saved,
            }),
        )));
        assert!(app.saved_toast.is_some());
        let _ = app.handle(Message::DismissToast);
        app.fields.insert("contacts", "invalid address".into());
        let _ = app.handle(Message::SavePreferences);
        assert!(app.saved_toast.is_none());
        assert!(app.confirm_save.is_none());
    }
    #[test]
    fn a_full_persistence_queue_coalesces_and_retries_the_latest_layout() {
        let (mut app, _) = App::new();
        let (sender, mut receiver) = engine::CommandSender::persistence_test_channel();
        while sender
            .try_send(Command::SavePreferences(0, Preferences::default()))
            .is_ok()
        {}
        app.tx = Some(sender);
        app.preferences.sidebar_width = Some(300.);
        app.save_preferences();
        app.preferences.sidebar_width = Some(340.);
        app.save_preferences();
        assert_eq!(
            app.pending_preference_save
                .as_ref()
                .unwrap()
                .1
                .sidebar_width,
            Some(340.)
        );
        receiver.try_recv().unwrap();
        let _ = app.handle(Message::Tick);
        assert!(app.pending_preference_save.is_none());
        let mut last = None;
        while let Ok(command) = receiver.try_recv() {
            last = Some(command);
        }
        let Some(Command::SavePreferences(request, prefs)) = last else {
            panic!("Missing retried settings");
        };
        assert_eq!(request, app.preference_sync.generation());
        assert_eq!(prefs.sidebar_width, Some(340.));
    }
    #[test]
    fn sidebar_clamps_display_width_without_overwriting_the_saved_preference() {
        let (mut app, _) = App::new();
        app.preferences.sidebar_width = Some(470.);
        app.size = Size::new(900., 640.);
        assert_eq!(app.sidebar_width(), 380.);
        app.preferences.interface_scale = 140;
        assert_eq!(app.sidebar_width(), 160.);
        assert_eq!(app.preferences.sidebar_width, Some(470.));
        for width in [f32::NAN, f32::INFINITY, -1.] {
            app.preferences.sidebar_width = Some(width);
            assert!(app.preferences.validate().is_err());
        }
    }
}
