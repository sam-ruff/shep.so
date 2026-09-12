//! One native text menu for inputs, editors and the formatted reader.
use super::{Element, Message, Theme, components};
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
    widget::{Id, Operation, Tree, operation::Focusable, tree},
};
use iced::{Event, Length, Point, Rectangle, Renderer, Size, Vector, keyboard, widget};
use std::sync::{Arc, Mutex, Weak};

mod input;
#[cfg(test)]
mod tests;
pub use input::Input;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Cut,
    Copy,
    Paste,
    SelectAll,
}
impl Action {
    const ALL: [Self; 4] = [Self::Cut, Self::Copy, Self::Paste, Self::SelectAll];
    fn label(self) -> &'static str {
        match self {
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::SelectAll => "Select all",
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Cut => "x",
            Self::Copy => "c",
            Self::Paste => "v",
            Self::SelectAll => "a",
        }
    }
}

#[derive(Default)]
pub struct Clip {
    epoch: u64,
    value: Option<String>,
    ready: bool,
    #[cfg(feature = "test-support")]
    enabled: [bool; 4],
}
impl std::fmt::Debug for Clip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardState")
            .field("ready", &self.ready)
            .finish()
    }
}
#[derive(Clone)]
pub enum Request {
    Read(u64, Weak<Mutex<Clip>>),
    Ready(u64, Weak<Mutex<Clip>>, Option<String>),
}
impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TextClipboardRequest")
    }
}
pub fn handle(request: Request) -> iced::Task<Message> {
    match request {
        Request::Read(epoch, target) => iced::Task::batch([
            iced::advanced::widget::operate(UnfocusOthers {
                target: target.clone(),
                epoch,
                inside: false,
            })
            .discard(),
            iced::clipboard::read().map(move |value| {
                Message::TextContext(Request::Ready(epoch, target.clone(), value))
            }),
        ]),
        Request::Ready(epoch, target, value) => {
            if let Some(target) = target.upgrade()
                && let Ok(mut target) = target.lock()
                && target.epoch == epoch
            {
                target.value = value;
                target.ready = true;
            }
            iced::Task::none()
        }
    }
}

#[cfg(feature = "test-support")]
#[derive(Default)]
pub(super) struct Observation(Option<(u64, Weak<Mutex<Clip>>)>);
#[cfg(feature = "test-support")]
impl Observation {
    pub fn observe(&mut self, request: &Request) {
        if let Request::Read(epoch, target) = request {
            self.0 = Some((*epoch, target.clone()));
        }
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let current = self.0.as_ref().and_then(|(epoch, target)| {
            let target = target.upgrade()?;
            let target = target.lock().ok()?;
            (target.epoch == *epoch).then_some((target.ready, target.enabled))
        });
        let (ready, enabled) = current.unwrap_or_default();
        serde_json::json!({"open": current.is_some(), "clipboard_ready": ready,
            "cut": enabled[0], "copy": enabled[1], "paste": enabled[2], "select_all": enabled[3]})
    }
}

enum Kind<'a> {
    Input {
        value: widget::text_input::Value,
        editable: bool,
        secure: bool,
    },
    Editor {
        content: &'a widget::text_editor::Content,
        editable: bool,
    },
    Html(&'a super::html_reader::State),
}
#[derive(Debug, Clone, PartialEq, Eq)]
enum Identity {
    Input(Option<Id>, String),
    Editor(String),
    Html(u64),
}
pub struct TextContext<'a> {
    content: Element<'a, Message>,
    kind: Kind<'a>,
    identity: Identity,
    theme: Option<Theme>,
}
impl<'a> TextContext<'a> {
    pub fn editor(
        content: impl Into<Element<'a, Message>>,
        buffer: &'a widget::text_editor::Content,
        editable: bool,
        identity: String,
    ) -> Self {
        Self {
            content: content.into(),
            kind: Kind::Editor {
                content: buffer,
                editable,
            },
            identity: Identity::Editor(identity),
            theme: None,
        }
    }
    pub(super) fn html(
        content: impl Into<Element<'a, Message>>,
        state: &'a super::html_reader::State,
    ) -> Self {
        Self {
            content: content.into(),
            kind: Kind::Html(state),
            identity: Identity::Html(state.generation),
            theme: None,
        }
    }
    pub(super) fn menu_theme(mut self, theme: Theme) -> Self {
        self.theme = Some(theme);
        self
    }
    fn available(&self, tree: &Tree, clip: &Clip) -> [bool; 4] {
        let (selected, populated, editable, secure) = match &self.kind {
            Kind::Input {
                value,
                editable,
                secure,
            } => {
                let state = tree.state.downcast_ref::<widget::text_input::State<
                    <Renderer as iced::advanced::text::Renderer>::Paragraph,
                >>();
                (
                    *editable && state.cursor().selection(value).is_some(),
                    *editable && !value.is_empty(),
                    *editable,
                    *secure,
                )
            }
            Kind::Editor { content, editable } => (
                content.cursor().selection.is_some(),
                !content.is_empty(),
                *editable,
                false,
            ),
            Kind::Html(state) => (
                !state.selection.is_empty(),
                state.frame.is_some(),
                false,
                false,
            ),
        };
        [
            editable && selected && !secure,
            selected && !secure,
            editable && clip.ready && clip.value.as_ref().is_some_and(|text| !text.is_empty()),
            populated,
        ]
    }
}
struct State {
    identity: Identity,
    position: Option<Point>,
    index: usize,
    navigated: bool,
    clip: Arc<Mutex<Clip>>,
    menu: Tree,
    modifiers: keyboard::Modifiers,
}
impl State {
    fn close(&mut self) {
        self.position = None;
        self.navigated = false;
        if let Ok(mut clip) = self.clip.lock() {
            clip.epoch = clip.epoch.wrapping_add(1);
            clip.value = None;
            clip.ready = false;
        }
    }
}
#[derive(Default)]
struct Focus {
    focused: bool,
    focus: bool,
}
struct FocusScope(Option<Weak<Mutex<Clip>>>);
struct UnfocusOthers {
    target: Weak<Mutex<Clip>>,
    epoch: u64,
    inside: bool,
}
impl Operation for UnfocusOthers {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn custom(&mut self, _: Option<&Id>, _: Rectangle, state: &mut dyn std::any::Any) {
        if let Some(scope) = state.downcast_ref::<FocusScope>() {
            self.inside = scope
                .0
                .as_ref()
                .is_some_and(|target| target.ptr_eq(&self.target));
        }
    }
    fn focusable(&mut self, _: Option<&Id>, _: Rectangle, state: &mut dyn Focusable) {
        if !self.inside
            && self
                .target
                .upgrade()
                .is_some_and(|target| target.lock().is_ok_and(|target| target.epoch == self.epoch))
        {
            state.unfocus();
        }
    }
}
impl Operation for Focus {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn focusable(&mut self, _: Option<&Id>, _: Rectangle, state: &mut dyn Focusable) {
        self.focused |= state.is_focused();
        if self.focus && !state.is_focused() {
            state.focus();
        }
    }
}
fn context_key(event: &Event) -> bool {
    matches!(event, Event::Keyboard(keyboard::Event::KeyPressed {key, modifiers, ..}) if
        (*key == keyboard::Key::Named(keyboard::key::Named::ContextMenu) && modifiers.is_empty())
        || (*key == keyboard::Key::Named(keyboard::key::Named::F10) && *modifiers == keyboard::Modifiers::SHIFT))
}
impl Widget<Message, Theme, Renderer> for TextContext<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State {
            identity: self.identity.clone(),
            position: None,
            index: 0,
            navigated: false,
            clip: Arc::new(Mutex::new(Clip::default())),
            menu: Tree::empty(),
            modifiers: keyboard::Modifiers::default(),
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        if state.identity != self.identity {
            state.close();
            state.identity = self.identity.clone();
        }
        tree.diff_children(std::slice::from_ref(&self.content));
    }
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_ref::<State>();
        operation.custom(
            None,
            layout.bounds(),
            &mut FocusScope(Some(Arc::downgrade(&state.clip))),
        );
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
        operation.custom(None, layout.bounds(), &mut FocusScope(None));
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            state.modifiers = *modifiers;
        }
        if matches!(event, Event::Window(iced::window::Event::Unfocused)) {
            state.close();
        }
        let right = matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
        ) && cursor.is_over(layout.bounds())
            && cursor.is_over(*viewport);
        let mut focus = Focus::default();
        if right || context_key(event) {
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                layout,
                renderer,
                &mut focus,
            );
        }
        if right || context_key(event) && focus.focused {
            if !focus.focused {
                let cursor = if let Kind::Input { value, .. } = &self.kind {
                    Some(
                        tree.children[0]
                            .state
                            .downcast_ref::<widget::text_input::State<
                                <Renderer as iced::advanced::text::Renderer>::Paragraph,
                            >>()
                            .cursor()
                            .state(value),
                    )
                } else {
                    None
                };
                self.content.as_widget_mut().operate(
                    &mut tree.children[0],
                    layout,
                    renderer,
                    &mut Focus {
                        focus: true,
                        ..Default::default()
                    },
                );
                if let Some(cursor) = cursor {
                    let input = tree.children[0]
                        .state
                        .downcast_mut::<widget::text_input::State<
                            <Renderer as iced::advanced::text::Renderer>::Paragraph,
                        >>();
                    match cursor {
                        widget::text_input::cursor::State::Index(index) => {
                            input.move_cursor_to(index)
                        }
                        widget::text_input::cursor::State::Selection { start, end } => {
                            input.select_range(start, end)
                        }
                    }
                }
            }
            state.close();
            state.position = Some(if right {
                cursor.position().unwrap_or(layout.position())
            } else {
                layout.position() + Vector::new(12., 24.)
            });
            if let Ok(clip) = state.clip.lock() {
                let enabled = self.available(&tree.children[0], &clip);
                state.index = enabled.iter().position(|enabled| *enabled).unwrap_or(3);
                shell.publish(Message::TextContext(Request::Read(
                    clip.epoch,
                    Arc::downgrade(&state.clip),
                )));
            }
            shell.capture_event();
            shell.request_redraw();
            return;
        }
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let state = tree.state.downcast_mut::<State>();
        let Some(position) = state.position else {
            return self.content.as_widget_mut().overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
            );
        };
        let enabled = state
            .clip
            .lock()
            .map(|clip| self.available(&tree.children[0], &clip))
            .unwrap_or([false; 4]);
        #[cfg(feature = "test-support")]
        if let Ok(mut clip) = state.clip.lock() {
            clip.enabled = enabled;
        }
        if (!state.navigated || !enabled[state.index])
            && let Some(index) = enabled.iter().position(|enabled| *enabled)
        {
            state.index = index;
        }
        let mut items = widget::column![].spacing(2);
        for (index, action) in Action::ALL.into_iter().enumerate() {
            if matches!(
                self.kind,
                Kind::Editor {
                    editable: false,
                    ..
                } | Kind::Html(_)
            ) && matches!(action, Action::Cut | Action::Paste)
            {
                continue;
            }
            items = items.push(
                widget::button(widget::text(action.label()).size(13))
                    .padding([9, 12])
                    .width(Length::Fill)
                    .style(if state.index == index && enabled[index] {
                        components::selected
                    } else {
                        components::ghost
                    })
                    .on_press_maybe(enabled[index].then_some(action)),
            );
        }
        let menu: iced::Element<'_, Action> = widget::container(items)
            .width(204)
            .padding(6)
            .style(components::card)
            .into();
        state.menu.diff(&menu);
        Some(overlay::Element::new(Box::new(MenuOverlay {
            content: &mut self.content,
            child: &mut tree.children[0],
            kind: &self.kind,
            state,
            menu,
            target_layout: layout,
            viewport: *viewport,
            position: position + translation,
            enabled,
            theme: self.theme.as_ref(),
        })))
    }
}
impl<'a> From<TextContext<'a>> for Element<'a, Message> {
    fn from(value: TextContext<'a>) -> Self {
        Self::new(value)
    }
}

struct MenuOverlay<'a, 'b> {
    content: &'a mut Element<'b, Message>,
    child: &'a mut Tree,
    kind: &'a Kind<'b>,
    state: &'a mut State,
    menu: iced::Element<'a, Action>,
    target_layout: Layout<'a>,
    viewport: Rectangle,
    position: Point,
    enabled: [bool; 4],
    theme: Option<&'a Theme>,
}
impl MenuOverlay<'_, '_> {
    fn choose(
        &mut self,
        action: Action,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        let Some(index) = Action::ALL
            .iter()
            .position(|candidate| *candidate == action)
        else {
            return;
        };
        if !self.enabled[index] {
            return;
        }
        if let Kind::Html(state) = self.kind {
            let input = match action {
                Action::Copy => crate::html_render::Input::Copy(state.generation),
                Action::SelectAll => crate::html_render::Input::SelectAll(state.generation),
                _ => return,
            };
            shell.publish(Message::Html(super::html_reader::Message::Input(input)));
        } else {
            let value = self
                .state
                .clip
                .lock()
                .ok()
                .and_then(|clip| clip.value.clone());
            let mut clipboard = CachedClipboard {
                native: clipboard,
                value,
            };
            let modifiers = keyboard::Modifiers::COMMAND;
            let key = keyboard::Key::Character(action.key().into());
            for event in [
                Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)),
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key: key.clone(),
                    modified_key: key.clone(),
                    physical_key: keyboard::key::Physical::Unidentified(
                        keyboard::key::NativeCode::Unidentified,
                    ),
                    location: keyboard::Location::Standard,
                    modifiers,
                    text: None,
                    repeat: false,
                }),
                Event::Keyboard(keyboard::Event::KeyReleased {
                    key: key.clone(),
                    modified_key: key,
                    physical_key: keyboard::key::Physical::Unidentified(
                        keyboard::key::NativeCode::Unidentified,
                    ),
                    location: keyboard::Location::Standard,
                    modifiers,
                }),
                Event::Keyboard(keyboard::Event::ModifiersChanged(self.state.modifiers)),
            ] {
                self.content.as_widget_mut().update(
                    self.child,
                    &event,
                    self.target_layout,
                    mouse::Cursor::Unavailable,
                    renderer,
                    &mut clipboard,
                    shell,
                    &self.viewport,
                );
            }
        }
        self.state.close();
        shell.capture_event();
        shell.request_redraw();
    }
}
struct CachedClipboard<'a> {
    native: &'a mut dyn Clipboard,
    value: Option<String>,
}
impl Clipboard for CachedClipboard<'_> {
    fn read(&self, _: iced::advanced::clipboard::Kind) -> Option<String> {
        self.value.clone()
    }
    fn write(&mut self, kind: iced::advanced::clipboard::Kind, value: String) {
        self.native.write(kind, value);
    }
}
impl overlay::Overlay<Message, Theme, Renderer> for MenuOverlay<'_, '_> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.menu.as_widget_mut().layout(
            &mut self.state.menu,
            renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );
        let position = Point::new(
            self.position
                .x
                .clamp(4., (bounds.width - node.size().width - 4.).max(4.)),
            self.position
                .y
                .clamp(4., (bounds.height - node.size().height - 4.).max(4.)),
        );
        node.move_to(position)
    }
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.menu.as_widget().draw(
            &self.state.menu,
            renderer,
            self.theme.unwrap_or(theme),
            style,
            layout,
            cursor,
            &layout.bounds(),
        );
    }
    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        if let Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) = event {
            self.state.modifiers = *modifiers;
        }
        if matches!(event, Event::Window(iced::window::Event::Unfocused))
            || matches!(event, Event::Mouse(mouse::Event::ButtonPressed(_)))
                && !cursor.is_over(layout.bounds())
        {
            self.state.close();
            shell.capture_event();
            shell.request_redraw();
            return;
        }
        if let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = event {
            use keyboard::key::Named;
            match key {
                keyboard::Key::Named(Named::Escape) => self.state.close(),
                keyboard::Key::Named(Named::ArrowDown | Named::ArrowUp) => {
                    self.state.navigated = true;
                    let step = if *key == keyboard::Key::Named(Named::ArrowUp) {
                        3
                    } else {
                        1
                    };
                    for _ in 0..4 {
                        self.state.index = (self.state.index + step) % 4;
                        if self.enabled[self.state.index] {
                            break;
                        }
                    }
                }
                keyboard::Key::Named(Named::Enter) => {
                    self.choose(Action::ALL[self.state.index], renderer, clipboard, shell)
                }
                _ => {}
            }
            shell.capture_event();
            shell.request_redraw();
            return;
        }
        let mut actions = Vec::new();
        let mut local = Shell::new(&mut actions);
        self.menu.as_widget_mut().update(
            &mut self.state.menu,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            &mut local,
            &layout.bounds(),
        );
        if local.event_status() == iced::event::Status::Captured {
            shell.capture_event();
        }
        shell.request_redraw_at(local.redraw_request());
        if local.is_layout_invalid() {
            shell.invalidate_layout();
        }
        if local.are_widgets_invalid() {
            shell.invalidate_widgets();
        }
        for action in actions {
            self.choose(action, renderer, clipboard, shell);
        }
    }
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.menu.as_widget().mouse_interaction(
            &self.state.menu,
            layout,
            cursor,
            &layout.bounds(),
            renderer,
        )
    }
}
