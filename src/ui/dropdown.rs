//! Native dropdowns that the keyboard can dismiss.
//!
//! An iced menu ignores the keyboard, so keys would reach the view behind it
//! while it stays open over other controls. [`Dismissible`] gives the open
//! menu every key press: Escape and Tab close it, and no key reaches a dialog,
//! the reader or a mail shortcut while it is open.
use super::Theme;
use iced::advanced::{
    Clipboard, Layout, Overlay, Shell, Widget, layout, mouse, overlay, renderer,
    widget::{Operation, Tree, tree},
};
use iced::widget::{overlay::menu, pick_list as native};
use iced::{Element, Event, Length, Padding, Pixels, Rectangle, Renderer, Size, Vector, keyboard};
use std::borrow::Borrow;

/// A pick list whose open menu closes with Escape or Tab.
pub fn pick_list<'a, T, L, V, M>(
    options: L,
    selected: Option<V>,
    on_select: impl Fn(T) -> M + 'a,
) -> PickList<'a, T, L, V, M>
where
    T: ToString + PartialEq + Clone + 'a,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a,
    M: Clone + 'a,
{
    PickList(native::PickList::new(options, selected, on_select))
}

pub struct PickList<'a, T, L, V, M>(native::PickList<'a, T, L, V, M, Theme, Renderer>)
where
    T: ToString + PartialEq + Clone,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a;

impl<'a, T, L, V, M> PickList<'a, T, L, V, M>
where
    T: ToString + PartialEq + Clone,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a,
    M: Clone,
{
    pub fn placeholder(self, placeholder: impl Into<String>) -> Self {
        Self(self.0.placeholder(placeholder))
    }
    pub fn width(self, width: impl Into<Length>) -> Self {
        Self(self.0.width(width))
    }
    pub fn padding(self, padding: impl Into<Padding>) -> Self {
        Self(self.0.padding(padding))
    }
    pub fn text_size(self, size: impl Into<Pixels>) -> Self {
        Self(self.0.text_size(size))
    }
    pub fn font(self, font: impl Into<iced::Font>) -> Self {
        Self(self.0.font(font))
    }
    pub fn style(self, style: impl Fn(&Theme, native::Status) -> native::Style + 'a) -> Self {
        Self(self.0.style(style))
    }
    pub fn menu_style(self, style: impl Fn(&Theme) -> menu::Style + 'a) -> Self {
        Self(self.0.menu_style(style))
    }
}

impl<'a, T, L, V, M> From<PickList<'a, T, L, V, M>> for Element<'a, M>
where
    T: ToString + PartialEq + Clone + 'a,
    L: Borrow<[T]> + 'a,
    V: Borrow<T> + 'a,
    M: Clone + 'a,
{
    fn from(list: PickList<'a, T, L, V, M>) -> Self {
        Element::new(Dismissible::new(list.0))
    }
}

/// Routes key presses to the open menu of any widget whose popup closes on a
/// click outside it, and closes that popup with Escape or Tab.
pub struct Dismissible<'a, M> {
    content: Element<'a, M>,
}

impl<'a, M> Dismissible<'a, M> {
    pub fn new(content: impl Into<Element<'a, M>>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

#[derive(Default)]
struct State {
    /// The keyboard closed the menu; the widget closes on its next event.
    dismissed: bool,
}

fn dismisses(key: &keyboard::Key) -> bool {
    use keyboard::key::Named;
    matches!(
        key,
        keyboard::Key::Named(Named::Escape) | keyboard::Key::Named(Named::Tab)
    )
}

impl<M> Widget<M, Theme, Renderer> for Dismissible<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
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
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        if std::mem::take(&mut state.dismissed) {
            // A press away from the open menu is the widget's own way to close
            // it. Its capture belongs to this synthetic press, not `event`.
            let mut messages = Vec::new();
            let mut closing = Shell::new(&mut messages);
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                &Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                layout,
                mouse::Cursor::Unavailable,
                renderer,
                clipboard,
                &mut closing,
                viewport,
            );
            for message in messages {
                shell.publish(message);
            }
            shell.request_redraw();
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
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, M, Theme, Renderer>> {
        let Tree {
            state, children, ..
        } = tree;
        let state = state.downcast_mut::<State>();
        if state.dismissed {
            return None;
        }
        let content = self.content.as_widget_mut().overlay(
            &mut children[0],
            layout,
            renderer,
            viewport,
            translation,
        )?;
        Some(overlay::Element::new(Box::new(Menu { content, state })))
    }
}

/// The open menu. After a dismissal it is inert, so later input in the same
/// batch reaches the controls it covered.
struct Menu<'a, M> {
    content: overlay::Element<'a, M, Theme, Renderer>,
    state: &'a mut State,
}

impl<M> Overlay<M, Theme, Renderer> for Menu<'_, M> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        self.content.as_overlay_mut().layout(renderer, bounds)
    }
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        if self.state.dismissed {
            return;
        }
        self.content
            .as_overlay()
            .draw(renderer, theme, style, layout, cursor);
    }
    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content
            .as_overlay_mut()
            .operate(layout, renderer, operation);
    }
    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
    ) {
        if self.state.dismissed {
            return;
        }
        if let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = event {
            shell.capture_event();
            if dismisses(key) {
                self.state.dismissed = true;
                shell.request_redraw();
            }
            return;
        }
        self.content
            .as_overlay_mut()
            .update(event, layout, cursor, renderer, clipboard, shell);
    }
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.state.dismissed {
            return mouse::Interaction::None;
        }
        self.content
            .as_overlay()
            .mouse_interaction(layout, cursor, renderer)
    }
    fn overlay<'b>(
        &'b mut self,
        layout: Layout<'b>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'b, M, Theme, Renderer>> {
        if self.state.dismissed {
            return None;
        }
        self.content.as_overlay_mut().overlay(layout, renderer)
    }
    fn index(&self) -> f32 {
        self.content.as_overlay().index()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Message, context_menu::ContextArea};
    use super::*;
    use iced::widget::{button, column};
    use iced::{Point, keyboard::key::Named};
    use iced_runtime::user_interface::{Cache, UserInterface};

    const OPTIONS: [&str; 3] = ["First", "Second", "Third"];
    const BOUNDS: Size = Size::new(300., 300.);
    /// Below the closed list, where its open menu lies over the button.
    const COVERED: Point = Point::new(60., 100.);

    fn view() -> Element<'static, Message> {
        ContextArea::root(
            column![
                pick_list(OPTIONS, Some("First"), |choice: &str| {
                    Message::CheckMail(choice.into())
                })
                .width(200)
                .padding(10),
                button("Covered")
                    .width(200)
                    .height(80)
                    .on_press(Message::Query("covered".into())),
            ]
            .spacing(4),
            100,
        )
        .into()
    }

    struct Window {
        renderer: Renderer,
        cache: Cache,
        cursor: Point,
    }

    impl Window {
        fn new() -> Self {
            Self {
                renderer: Renderer::new(iced::Font::DEFAULT, Pixels(16.)),
                cache: Cache::new(),
                cursor: Point::ORIGIN,
            }
        }

        /// One runtime batch: overlay first, then the base widgets, as iced does.
        fn batch(&mut self, events: &[Event]) -> Vec<Message> {
            for event in events {
                if let Event::Mouse(mouse::Event::CursorMoved { position }) = event {
                    self.cursor = *position;
                }
            }
            let mut ui = UserInterface::build(
                view(),
                BOUNDS,
                std::mem::take(&mut self.cache),
                &mut self.renderer,
            );
            let mut messages = Vec::new();
            let _ = ui.update(
                events,
                mouse::Cursor::Available(self.cursor),
                &mut self.renderer,
                &mut iced::advanced::clipboard::Null,
                &mut messages,
            );
            self.cache = ui.into_cache();
            messages
        }
    }

    fn redraw() -> Event {
        Event::Window(iced::window::Event::RedrawRequested(
            std::time::Instant::now(),
        ))
    }

    fn click(at: Point) -> [Event; 3] {
        [
            Event::Mouse(mouse::Event::CursorMoved { position: at }),
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ]
    }

    fn key(named: Named) -> Event {
        let key = keyboard::Key::Named(named);
        Event::Keyboard(keyboard::Event::KeyPressed {
            modified_key: key.clone(),
            key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        })
    }

    fn keys(messages: &[Message]) -> usize {
        messages
            .iter()
            .filter(|m| matches!(m, Message::Key(..)))
            .count()
    }

    fn chosen(messages: &[Message]) -> Vec<&str> {
        messages
            .iter()
            .filter_map(|m| match m {
                Message::CheckMail(choice) => Some(choice.as_str()),
                Message::Query(button) => Some(button.as_str()),
                _ => None,
            })
            .collect()
    }

    fn opened() -> Window {
        let mut window = Window::new();
        let _ = window.batch(&[redraw()]);
        let _ = window.batch(&click(Point::new(60., 20.)));
        window
    }

    #[test]
    fn escape_closes_only_the_open_menu_and_the_covered_button_takes_the_next_click() {
        let mut window = opened();
        let escape = window.batch(&[key(Named::Escape)]);
        assert_eq!(keys(&escape), 0, "Escape must not reach the view behind");
        let _ = window.batch(&[redraw()]);
        let next = window.batch(&click(COVERED));
        assert_eq!(chosen(&next), ["covered"]);
    }

    #[test]
    fn escape_then_click_in_one_batch_reaches_the_covered_button() {
        let mut window = opened();
        let mut events = vec![key(Named::Escape)];
        events.extend(click(COVERED));
        let messages = window.batch(&events);
        assert_eq!(keys(&messages), 0);
        assert_eq!(chosen(&messages), ["covered"]);
    }

    #[test]
    fn a_second_escape_in_the_same_batch_reaches_the_application() {
        let mut window = opened();
        let messages = window.batch(&[key(Named::Escape), key(Named::Escape)]);
        assert_eq!(keys(&messages), 1);
        assert_eq!(chosen(&window.batch(&click(COVERED))), ["covered"]);
    }

    #[test]
    fn keys_while_open_never_reach_shortcuts_and_tab_closes() {
        let mut window = opened();
        let typed = window.batch(&[
            Event::Keyboard(keyboard::Event::KeyPressed {
                modified_key: keyboard::Key::Character("d".into()),
                key: keyboard::Key::Character("d".into()),
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::CTRL,
                text: None,
                repeat: false,
            }),
            key(Named::Tab),
        ]);
        assert_eq!(keys(&typed), 0);
        let _ = window.batch(&[redraw()]);
        assert_eq!(chosen(&window.batch(&click(COVERED))), ["covered"]);
        // Closed lists leave keys to the application, and reopen normally.
        assert_eq!(keys(&window.batch(&[key(Named::Escape)])), 1);
        let _ = window.batch(&click(Point::new(60., 20.)));
        assert_eq!(keys(&window.batch(&[key(Named::Escape)])), 0);
    }

    #[test]
    fn a_mouse_choice_still_selects_from_the_open_menu() {
        let mut window = opened();
        assert_eq!(chosen(&window.batch(&click(COVERED))), ["Second"]);
    }
}
