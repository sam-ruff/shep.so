//! The bordered To/Cc/Bcc box: chips flow across lines with the entry after
//! them, under one focus ring, and a press on empty space focuses the entry.
//!
//! The entry is always the first child in the widget tree, so adding or
//! removing chips never replaces its state and focus.
use crate::ui::{Message, components::colors};
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
    widget::{Id, Operation, Tree, operation::Focusable, tree},
};
use iced::{
    Border, Element, Event, Length, Padding, Point, Rectangle, Renderer, Size, Theme, Vector,
};

const PADDING: Padding = Padding {
    top: 4.,
    right: 4.,
    bottom: 4.,
    left: 4.,
};
const SPACING: f32 = 4.;
/// Narrower space left after the chips moves typed text to its own line.
const MIN_ENTRY: f32 = 140.;
/// An empty entry only needs room for its caret, so one chip never adds a line.
const EMPTY_ENTRY: f32 = 16.;

pub(in crate::ui) struct RecipientBox<'a> {
    /// The entry, then each chip in display order.
    items: Vec<Element<'a, Message>>,
    empty: bool,
    #[cfg(feature = "test-support")]
    log: Option<(&'static str, std::sync::Arc<crate::ui::draw_log::Log>)>,
}

impl<'a> RecipientBox<'a> {
    pub fn new(
        entry: impl Into<Element<'a, Message>>,
        empty: bool,
        chips: impl IntoIterator<Item = Element<'a, Message>>,
    ) -> Self {
        Self {
            items: std::iter::once(entry.into()).chain(chips).collect(),
            empty,
            #[cfg(feature = "test-support")]
            log: None,
        }
    }

    /// Records where each chip's remove button is drawn, for native tests.
    #[cfg(feature = "test-support")]
    pub fn observed(
        mut self,
        field: &'static str,
        log: std::sync::Arc<crate::ui::draw_log::Log>,
    ) -> Self {
        self.log = Some((field, log));
        self
    }
}

#[derive(Default)]
struct State {
    focused: bool,
}

/// Reports whether the entry has focus, optionally focusing it first.
#[derive(Default)]
struct Entry {
    focus: bool,
    focused: bool,
}

impl Operation for Entry {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn focusable(&mut self, _: Option<&Id>, _: Rectangle, state: &mut dyn Focusable) {
        if self.focus && !state.is_focused() {
            state.focus();
        }
        self.focused |= state.is_focused();
    }
}

/// Places items left to right, starting a new line when one does not fit.
#[derive(Default)]
struct Flow {
    width: f32,
    x: f32,
    y: f32,
    line: f32,
    current: Vec<usize>,
    placed: Vec<(usize, layout::Node)>,
}

impl Flow {
    fn place(&mut self, index: usize, node: layout::Node) {
        let size = node.size();
        if self.x > 0. && self.x + size.width > self.width {
            self.break_line();
        }
        self.placed
            .push((index, node.move_to(Point::new(self.x, self.y))));
        self.current.push(self.placed.len() - 1);
        self.line = self.line.max(size.height);
        self.x += size.width + SPACING;
    }

    /// Centres the finished line's items vertically, then starts another.
    fn break_line(&mut self) {
        for &placed in &self.current {
            let node = &mut self.placed[placed].1;
            let offset = (self.line - node.size().height) / 2.;
            node.translate_mut(Vector::new(0., offset));
        }
        self.current.clear();
        self.y += self.line + SPACING;
        self.x = 0.;
        self.line = 0.;
    }

    fn remaining(&self) -> f32 {
        self.width - self.x
    }
}

impl Widget<Message, Theme, Renderer> for RecipientBox<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        self.items.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.items);
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let width = limits.width(Length::Fill).max().width;
        let mut flow = Flow {
            width: (width - PADDING.x()).max(0.),
            ..Default::default()
        };
        let chip_width = (flow.width - EMPTY_ENTRY - SPACING).max(0.);
        let chip_limits = layout::Limits::new(Size::ZERO, Size::new(chip_width, f32::INFINITY));
        for (index, (chip, state)) in self
            .items
            .iter_mut()
            .zip(&mut tree.children)
            .enumerate()
            .skip(1)
        {
            let node = chip.as_widget_mut().layout(state, renderer, &chip_limits);
            flow.place(index, node);
        }
        let minimum = if self.empty { EMPTY_ENTRY } else { MIN_ENTRY };
        if flow.x > 0. && flow.remaining() < minimum {
            flow.break_line();
        }
        let entry_width = flow.remaining().max(0.);
        let entry_limits = layout::Limits::new(Size::ZERO, Size::new(entry_width, f32::INFINITY))
            .width(entry_width);
        let node =
            self.items[0]
                .as_widget_mut()
                .layout(&mut tree.children[0], renderer, &entry_limits);
        flow.place(0, node);
        flow.break_line();
        let height = (flow.y - SPACING).max(0.) + PADDING.y();
        let mut placed = flow.placed;
        placed.sort_by_key(|(index, _)| *index);
        let children = placed
            .into_iter()
            .map(|(_, node)| node.translate(Vector::new(PADDING.left, PADDING.top)))
            .collect();
        layout::Node::with_children(Size::new(width, height), children)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        operation.traverse(&mut |operation| {
            for ((item, state), layout) in self
                .items
                .iter_mut()
                .zip(&mut tree.children)
                .zip(layout.children())
            {
                item.as_widget_mut()
                    .operate(state, layout, renderer, operation);
            }
        });
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
        for ((item, state), child) in self
            .items
            .iter_mut()
            .zip(&mut tree.children)
            .zip(layout.children())
        {
            item.as_widget_mut().update(
                state, event, child, cursor, renderer, clipboard, shell, viewport,
            );
        }
        let press = matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
        ) && !shell.is_event_captured()
            && cursor.is_over(layout.bounds());
        let mut entry = Entry {
            focus: press,
            ..Default::default()
        };
        if let Some(child) = layout.children().next() {
            self.items[0].as_widget_mut().operate(
                &mut tree.children[0],
                child,
                renderer,
                &mut entry,
            );
        }
        if press {
            shell.capture_event();
        }
        let state = tree.state.downcast_mut::<State>();
        if state.focused != entry.focused {
            state.focused = entry.focused;
            shell.request_redraw();
        }
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
        #[cfg(feature = "test-support")]
        if let Some((field, log)) = &self.log {
            // Each chip is a container around its label and remove button.
            log.recipients(
                field,
                layout.bounds(),
                layout.children().skip(1).filter_map(|chip| {
                    chip.children()
                        .next()
                        .and_then(|row| row.children().last())
                        .map(|remove| remove.bounds())
                }),
            );
        }
        let p = colors(theme);
        let focused = tree.state.downcast_ref::<State>().focused;
        renderer::Renderer::fill_quad(
            renderer,
            renderer::Quad {
                bounds: layout.bounds(),
                border: Border {
                    color: if focused { p.accent } else { p.border },
                    width: 1.,
                    radius: 8.into(),
                },
                ..Default::default()
            },
            p.surface,
        );
        for ((item, state), child) in self.items.iter().zip(&tree.children).zip(layout.children()) {
            item.as_widget()
                .draw(state, renderer, theme, style, child, cursor, viewport);
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        let interaction = self
            .items
            .iter()
            .zip(&tree.children)
            .zip(layout.children())
            .map(|((item, state), child)| {
                item.as_widget()
                    .mouse_interaction(state, child, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or_default();
        if interaction == mouse::Interaction::None && cursor.is_over(layout.bounds()) {
            mouse::Interaction::Text
        } else {
            interaction
        }
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        overlay::from_children(
            &mut self.items,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a> From<RecipientBox<'a>> for Element<'a, Message> {
    fn from(value: RecipientBox<'a>) -> Self {
        Self::new(value)
    }
}
