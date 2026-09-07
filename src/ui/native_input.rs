//! Keep key presses in the same native message stream as mouse actions.
use super::*;
use iced::Rectangle;
use iced::advanced::widget::{Id, Operation, operation::Focusable};

/// Snapshot the actual widget focus while processing this key, not after an
/// asynchronous round trip that may observe a different field or dialog.
#[derive(Debug, Clone, Copy, Default)]
pub struct Focus {
    pub search: bool,
    pub find: bool,
}

impl Operation for Focus {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn focusable(&mut self, id: Option<&Id>, _: Rectangle, state: &mut dyn Focusable) {
        if !state.is_focused() {
            return;
        }
        static IDS: std::sync::OnceLock<[Id; 2]> = std::sync::OnceLock::new();
        let [search, find] = IDS.get_or_init(|| [Id::new("search"), Id::new("find-message")]);
        self.search |= id == Some(search);
        self.find |= id == Some(find);
    }
}

impl App {
    pub(super) fn native_key(
        &mut self,
        key: Key,
        modifiers: keyboard::Modifiers,
        captured: bool,
        focus: Focus,
    ) -> Task<Message> {
        let action = chord(&key, modifiers)
            .as_deref()
            .and_then(|key| self.preferences.shortcuts.resolve(key));
        if self.dialog.is_none()
            && self.remapping.is_none()
            && action == Some(Action::SelectAll)
            && (self.tab != Tab::Mail
                || !self.list_focus
                || self.sidebar_focus
                || self.full_reader
                || focus.search
                || focus.find)
        {
            return Task::none();
        }
        if self.find_message.open
            && self.tab == Tab::Mail
            && self.dialog.is_none()
            && self.remapping.is_none()
            && self.context_menu.is_none()
            && self.folder_controls.menu.is_none()
            && self.composer.context.is_none()
            && key == Key::Named(keyboard::key::Named::Enter)
            && focus.find
        {
            return self.handle_find(find_message::Message::Next(modifiers.shift()));
        }
        let input_guard = !captured
            && self.dialog.is_none()
            && self.remapping.is_none()
            && self.context_menu.is_none()
            && self.folder_controls.menu.is_none()
            && self.composer.context.is_none()
            && action.is_some_and(|action| {
                !matches!(
                    action,
                    Action::Search
                        | Action::Find
                        | Action::Mail
                        | Action::Calendar
                        | Action::Settings
                        | Action::Compose
                        | Action::Sync
                )
            });
        if input_guard && self.tab != Tab::Mail {
            return Task::none();
        }
        // Some modified chords (e.g. Ctrl+D) are left uncaptured by iced text
        // inputs. Native focus still protects editing from mail mutations.
        self.key(
            key,
            modifiers,
            captured || input_guard && (focus.search || focus.find),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::advanced::{Layout, Shell, Widget, layout, mouse, widget::Tree};
    use iced::widget::{button, column, container, text_input};
    use iced::{Point, Renderer};

    struct Native {
        area: context_menu::ContextArea<'static>,
        renderer: Renderer,
        tree: Tree,
        node: layout::Node,
        messages: Vec<Message>,
    }

    impl Native {
        fn new(input_id: &'static str) -> Self {
            let mut area = context_menu::ContextArea::root(
                column![
                    container(
                        text_input("Search", "")
                            .id(input_id)
                            .on_input(Message::Query)
                    )
                    .height(40),
                    button("Review")
                        .width(100)
                        .height(40)
                        .on_press(Message::Open(Dialog::Sender)),
                ]
                .spacing(10),
                100,
            );
            let renderer = Renderer::new(iced::Font::DEFAULT, iced::Pixels(16.));
            let mut tree = Tree::new(&area as &dyn Widget<Message, Theme, Renderer>);
            let node = area.layout(
                &mut tree,
                &renderer,
                &layout::Limits::new(Size::ZERO, Size::new(200., 200.)),
            );
            let mut focus =
                iced::advanced::widget::operation::focusable::focus::<()>(Id::new(input_id));
            area.operate(&mut tree, Layout::new(&node), &renderer, &mut focus);
            Self {
                area,
                renderer,
                tree,
                node,
                messages: vec![],
            }
        }

        fn event(&mut self, event: iced::Event) {
            self.area.update(
                &mut self.tree,
                &event,
                Layout::new(&self.node),
                mouse::Cursor::Available(Point::new(10., 60.)),
                &self.renderer,
                &mut iced::advanced::clipboard::Null,
                &mut Shell::new(&mut self.messages),
                &Rectangle::with_size(Size::new(200., 200.)),
            );
        }

        fn key(&mut self, key: Key, modifiers: keyboard::Modifiers) {
            self.event(iced::Event::Keyboard(keyboard::Event::KeyPressed {
                modified_key: key.clone(),
                key,
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                location: keyboard::Location::Standard,
                modifiers,
                text: None,
                repeat: false,
            }));
        }

        fn click(&mut self) {
            self.event(iced::Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(10., 60.),
            }));
            self.event(iced::Event::Mouse(mouse::Event::ButtonPressed(
                mouse::Button::Left,
            )));
            self.event(iced::Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left,
            )));
        }
    }

    #[test]
    fn native_escape_keeps_order_before_a_later_mouse_dialog_open() {
        let mut native = Native::new("search");
        native.key(
            Key::Named(keyboard::key::Named::Escape),
            keyboard::Modifiers::empty(),
        );
        native.click();
        assert!(matches!(
            native.messages.as_slice(),
            [
                Message::Key(Key::Named(keyboard::key::Named::Escape), _, _, _),
                Message::Open(Dialog::Sender),
            ]
        ));
        let (mut app, _) = App::new();
        app.focused_input = Some("search");
        for message in native.messages {
            let _ = app.handle(message);
        }
        assert_eq!(app.dialog, Some(Dialog::Sender));
        assert!(app.focused_input.is_none());
    }

    #[test]
    fn native_unhandled_chords_keep_the_focus_at_their_own_event() {
        for input in ["search", "find-message"] {
            let mut native = Native::new(input);
            native.key(Key::Character("d".into()), keyboard::Modifiers::CTRL);
            native.click();
            native.key(Key::Character("d".into()), keyboard::Modifiers::CTRL);
            let keys: Vec<_> = native
                .messages
                .iter()
                .filter_map(|m| match m {
                    Message::Key(_, _, captured, focus) => Some((*captured, *focus)),
                    _ => None,
                })
                .collect();
            assert_eq!(keys.len(), 2);
            assert!(
                !keys[0].0,
                "This regression must exercise an unhandled text-input chord"
            );
            assert_eq!(keys[0].1.search, input == "search");
            assert_eq!(keys[0].1.find, input == "find-message");
            assert!(!keys[1].1.search && !keys[1].1.find);
        }
    }

    #[test]
    fn native_find_enter_keeps_focus_and_shift_without_an_async_focus_check() {
        let mut native = Native::new("find-message");
        native.key(
            Key::Named(keyboard::key::Named::Enter),
            keyboard::Modifiers::SHIFT,
        );
        assert!(matches!(native.messages.as_slice(),[
            Message::Key(Key::Named(keyboard::key::Named::Enter),modifiers,false,Focus { find:true,.. })
        ] if modifiers.shift()));
    }
}
