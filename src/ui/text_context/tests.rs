use super::*;
use std::cell::Cell;

#[derive(Default)]
struct ClipboardProbe {
    reads: Cell<usize>,
    writes: Vec<String>,
}
impl Clipboard for ClipboardProbe {
    fn read(&self, _: iced::advanced::clipboard::Kind) -> Option<String> {
        self.reads.set(self.reads.get() + 1);
        Some("unexpected synchronous read".into())
    }
    fn write(&mut self, _: iced::advanced::clipboard::Kind, value: String) {
        self.writes.push(value);
    }
}
struct Native<'a> {
    element: Element<'a, Message>,
    renderer: Renderer,
    tree: Tree,
    node: layout::Node,
    messages: Vec<Message>,
    clipboard: ClipboardProbe,
}
type InputState =
    widget::text_input::State<<Renderer as iced::advanced::text::Renderer>::Paragraph>;
impl<'a> Native<'a> {
    fn input(secure: bool) -> Self {
        let element = Input::new("Fixture", "alpha bravo")
            .on_input(Message::Query)
            .secure(secure)
            .width(300)
            .into();
        Self::new(element)
    }
    fn new(mut element: Element<'a, Message>) -> Self {
        let renderer = Renderer::new(iced::Font::DEFAULT, iced::Pixels(16.));
        let mut tree = Tree::new(&element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, Size::new(500., 400.)),
        );
        Self {
            element,
            renderer,
            tree,
            node,
            messages: vec![],
            clipboard: ClipboardProbe::default(),
        }
    }
    fn input_state(&mut self) -> &mut InputState {
        self.tree.children[0].state.downcast_mut::<InputState>()
    }
    fn event(&mut self, event: Event) {
        self.element.as_widget_mut().update(
            &mut self.tree,
            &event,
            Layout::new(&self.node),
            mouse::Cursor::Available(Point::new(12., 12.)),
            &self.renderer,
            &mut self.clipboard,
            &mut Shell::new(&mut self.messages),
            &Rectangle::with_size(Size::new(500., 400.)),
        );
    }
    fn open(&mut self) {
        self.event(Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Right,
        )));
        assert!(self.tree.state.downcast_ref::<State>().position.is_some());
    }
    fn clipboard_ready(&mut self, value: Option<&str>) {
        let state = self.tree.state.downcast_ref::<State>();
        let epoch = state.clip.lock().expect("clipboard state").epoch;
        let _ = handle(Request::Ready(
            epoch,
            Arc::downgrade(&state.clip),
            value.map(str::to_owned),
        ));
    }
    fn choose(&mut self, action: Action) {
        let mut index = Action::ALL
            .iter()
            .position(|item| *item == action)
            .expect("action");
        let mut overlay = self
            .element
            .as_widget_mut()
            .overlay(
                &mut self.tree,
                Layout::new(&self.node),
                &self.renderer,
                &Rectangle::with_size(Size::new(500., 400.)),
                Vector::ZERO,
            )
            .expect("text menu");
        let node = overlay
            .as_overlay_mut()
            .layout(&self.renderer, Size::new(500., 400.));
        let menu = Layout::new(&node).children().next().expect("menu column");
        if menu.children().count() == 2 {
            index = match action {
                Action::Copy => 0,
                Action::SelectAll => 1,
                Action::Cut | Action::Paste => return,
            };
        }
        let pointer = menu
            .children()
            .nth(index)
            .expect("menu action")
            .bounds()
            .center();
        for event in [
            mouse::Event::ButtonPressed(mouse::Button::Left),
            mouse::Event::ButtonReleased(mouse::Button::Left),
        ] {
            overlay.as_overlay_mut().update(
                &Event::Mouse(event),
                Layout::new(&node),
                mouse::Cursor::Available(pointer),
                &self.renderer,
                &mut self.clipboard,
                &mut Shell::new(&mut self.messages),
            );
        }
    }
    fn enter(&mut self) {
        let mut overlay = self
            .element
            .as_widget_mut()
            .overlay(
                &mut self.tree,
                Layout::new(&self.node),
                &self.renderer,
                &Rectangle::with_size(Size::new(500., 400.)),
                Vector::ZERO,
            )
            .expect("text menu");
        let node = overlay
            .as_overlay_mut()
            .layout(&self.renderer, Size::new(500., 400.));
        overlay.as_overlay_mut().update(
            &key(keyboard::key::Named::Enter, keyboard::Modifiers::empty()),
            Layout::new(&node),
            mouse::Cursor::Unavailable,
            &self.renderer,
            &mut self.clipboard,
            &mut Shell::new(&mut self.messages),
        );
    }
}
fn key(named: keyboard::key::Named, modifiers: keyboard::Modifiers) -> Event {
    Event::Keyboard(keyboard::Event::KeyPressed {
        key: keyboard::Key::Named(named),
        modified_key: keyboard::Key::Named(named),
        physical_key: keyboard::key::Physical::Unidentified(
            keyboard::key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers,
        text: None,
        repeat: false,
    })
}

#[test]
fn right_click_preserves_unfocused_selection_and_copies_without_clipboard_read() {
    let mut native = Native::input(false);
    native.input_state().select_range(6, 11);
    native.open();
    assert!(native.input_state().is_focused());
    native.choose(Action::Copy);
    assert_eq!(native.clipboard.writes, ["bravo"]);
    assert_eq!(native.clipboard.reads.get(), 0);
    assert!(native.tree.state.downcast_ref::<State>().position.is_none());
}

#[test]
fn cut_requires_selection_and_password_never_copies_or_cuts() {
    for secure in [false, true] {
        let mut native = Native::input(secure);
        if secure {
            native.input_state().select_range(0, 5);
        }
        native.open();
        native.choose(Action::Cut);
        native.choose(Action::Copy);
        assert!(native.clipboard.writes.is_empty());
        assert!(
            !native
                .messages
                .iter()
                .any(|message| matches!(message, Message::Query(_)))
        );
    }
    let mut native = Native::input(false);
    native.input_state().select_range(0, 6);
    native.open();
    native.choose(Action::Cut);
    assert_eq!(native.clipboard.writes, ["alpha "]);
    assert!(
        native
            .messages
            .iter()
            .any(|message| matches!(message, Message::Query(value) if value == "bravo"))
    );
}

#[test]
fn paste_waits_for_async_clipboard_then_uses_selection_and_releases_native_paste_state() {
    let mut native = Native::input(false);
    native.input_state().select_range(0, 5);
    native.open();
    native.choose(Action::Paste);
    assert!(
        !native
            .messages
            .iter()
            .any(|message| matches!(message, Message::Query(_)))
    );
    native.clipboard_ready(Some("new"));
    native.choose(Action::Paste);
    assert!(
        native
            .messages
            .iter()
            .any(|message| matches!(message, Message::Query(value) if value == "new bravo"))
    );
    native.open();
    native.clipboard_ready(Some("different"));
    native.choose(Action::Paste);
    assert!(
        native
            .messages
            .iter()
            .any(|message| matches!(message, Message::Query(value) if value.contains("different")))
    );
    assert_eq!(native.clipboard.reads.get(), 0);
}

#[test]
fn keyboard_menu_requires_actual_focus_and_select_all_uses_native_selection() {
    let mut native = Native::input(false);
    native.event(key(
        keyboard::key::Named::ContextMenu,
        keyboard::Modifiers::empty(),
    ));
    assert!(native.tree.state.downcast_ref::<State>().position.is_none());
    native.input_state().focus();
    native.event(key(keyboard::key::Named::F10, keyboard::Modifiers::SHIFT));
    native.choose(Action::SelectAll);
    native.open();
    native.choose(Action::Copy);
    assert_eq!(native.clipboard.writes, ["alpha bravo"]);
}

#[test]
fn old_clipboard_completion_cannot_enable_a_reopened_menu() {
    let mut native = Native::input(false);
    native.open();
    let state = native.tree.state.downcast_mut::<State>();
    let old_epoch = state.clip.lock().expect("state").epoch;
    let target = Arc::downgrade(&state.clip);
    state.close();
    native.open();
    let _ = handle(Request::Ready(
        old_epoch,
        target,
        Some("old clipboard".into()),
    ));
    native.choose(Action::Paste);
    assert!(
        !native
            .messages
            .iter()
            .any(|message| matches!(message, Message::Query(_)))
    );
}

#[test]
fn focus_operation_unfocuses_other_inputs_without_moving_target_selection() {
    let mut target = Native::input(false);
    let mut previous = Native::input(false);
    previous.input_state().focus();
    target.input_state().select_range(0, 5);
    target.open();
    let state = target.tree.state.downcast_ref::<State>();
    let mut operation = UnfocusOthers {
        target: Arc::downgrade(&state.clip),
        epoch: state.clip.lock().expect("state").epoch,
        inside: false,
    };
    for native in [&mut target, &mut previous] {
        native.element.as_widget_mut().operate(
            &mut native.tree,
            Layout::new(&native.node),
            &native.renderer,
            &mut operation,
        );
    }
    assert!(!previous.input_state().is_focused());
    target.choose(Action::Copy);
    assert_eq!(target.clipboard.writes, ["alpha"]);
    previous.input_state().focus();
    previous.element.as_widget_mut().operate(
        &mut previous.tree,
        Layout::new(&previous.node),
        &previous.renderer,
        &mut operation,
    );
    assert!(
        previous.input_state().is_focused(),
        "A closed menu cannot steal newer focus"
    );
}

#[test]
fn read_only_editor_offers_copy_without_forwarding_any_edit() {
    let mut content = widget::text_editor::Content::with_text("Read-only fixture");
    content.perform(widget::text_editor::Action::SelectAll);
    let editor = widget::text_editor(&content)
        .height(100)
        .on_action(|action| Message::ReaderSelection("fixture".into(), 0, action));
    let mut native =
        Native::new(TextContext::editor(editor, &content, false, "fixture".into()).into());
    native.open();
    native.clipboard_ready(Some("must not replace text"));
    native.choose(Action::Cut);
    native.choose(Action::Paste);
    assert!(
        !native
            .messages
            .iter()
            .any(|message| matches!(message, Message::ReaderSelection(..)))
    );
    native.choose(Action::Copy);
    assert_eq!(native.clipboard.writes, ["Read-only fixture"]);
    assert_eq!(content.text(), "Read-only fixture");
}

#[test]
fn disabled_input_keeps_all_native_actions_disabled() {
    let mut native = Native::new(Input::new("Saving", "retained selection").into());
    native.input_state().select_range(0, 8);
    native.open();
    native.clipboard_ready(Some("clipboard"));
    for action in Action::ALL {
        native.choose(action);
    }
    assert!(native.clipboard.writes.is_empty());
    assert!(
        !native
            .messages
            .iter()
            .any(|message| matches!(message, Message::Query(_)))
    );
    assert!(native.tree.state.downcast_ref::<State>().position.is_some());
}

#[test]
fn selection_completed_after_open_updates_initial_keyboard_action() {
    let mut native = Native::input(false);
    native.open();
    native.input_state().select_range(0, 6);
    native.enter();
    assert_eq!(native.clipboard.writes, ["alpha "]);
    assert!(
        native
            .messages
            .iter()
            .any(|message| matches!(message, Message::Query(value) if value == "bravo"))
    );
}

#[test]
fn replacing_the_field_identity_closes_the_menu_and_rejects_old_clipboard_reply() {
    let mut native = Native::input(false);
    native.open();
    let state = native.tree.state.downcast_ref::<State>();
    let epoch = state.clip.lock().expect("state").epoch;
    let target = Arc::downgrade(&state.clip);
    native.element = Input::new("Other field", "different")
        .on_input(Message::Query)
        .into();
    native.element.as_widget_mut().diff(&mut native.tree);
    let _ = handle(Request::Ready(epoch, target, Some("old clipboard".into())));
    let state = native.tree.state.downcast_ref::<State>();
    assert!(state.position.is_none());
    assert!(!state.clip.lock().expect("state").ready);
}
