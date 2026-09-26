//! A small focusable "?" beside a setting whose meaning is not obvious. Its
//! short help appears while the pointer is over it or while it has keyboard
//! focus, and the whole control is omitted when help icons are turned off.
use super::{App, Message, components};
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
    text::{self as core_text, Renderer as _},
    widget::{Id, Operation, Tree, operation::Focusable, tree},
};
use iced::widget::{container, row, space, text};
use iced::{
    Alignment, Border, Color, Element, Event, Length, Pixels, Point, Rectangle, Renderer, Size,
    Theme, Vector, keyboard,
};
#[cfg(feature = "test-support")]
use std::sync::Arc;

/// One setting's help: a stable id and its text.
#[derive(Debug)]
pub struct Topic {
    pub id: &'static str,
    pub text: &'static str,
}

pub const CHECK_INTERVAL: Topic = Topic {
    id: "help-check-interval",
    text: "How often Shep asks each server for new mail and changes. Accounts with IMAP push get new Inbox mail sooner, but this check still covers every folder.",
};
pub const CROSS_ACCOUNT_MOVES: Topic = Topic {
    id: "help-cross-account-moves",
    text: "Lets you move a message into another IMAP account. Shep copies it there first and removes the original only after the copy is confirmed.",
};
pub const FOREIGN_MOVE_FOLDERS: Topic = Topic {
    id: "help-foreign-move-folders",
    text: "Adds folders from your other IMAP accounts to Move search results. Choosing one asks before moving. Needs moving mail between accounts turned on.",
};
pub const CLOSE_TO_TRAY: Topic = Topic {
    id: "help-close-to-tray",
    text: "Closing the window hides Shep instead of quitting, so it keeps checking mail and showing notifications. Open or quit it from the tray icon. Without a system tray, closing quits.",
};
pub const BACKUP_COMPRESSION: Topic = Topic {
    id: "help-backup-compression",
    text: "Makes backup files smaller. Compression is not protection: anyone with an unencrypted copy can still read your mail.",
};
pub const BACKUP_ENCRYPTION: Topic = Topic {
    id: "help-backup-encryption",
    text: "Locks each copy with your passphrase. Restoring on another device needs the same passphrase, so keep a note of it somewhere safe.",
};
pub const SYNCED_PASSWORDS: Topic = Topic {
    id: "help-synced-passwords",
    text: "Stores mail passwords in Shep's private Drive app data so your other Shep installations can sign in. Only your Google account protects them; there is no separate passphrase.",
};

#[cfg(test)]
pub const ALL: [&Topic; 7] = [
    &CHECK_INTERVAL,
    &CROSS_ACCOUNT_MOVES,
    &FOREIGN_MOVE_FOLDERS,
    &CLOSE_TO_TRAY,
    &BACKUP_COMPRESSION,
    &BACKUP_ENCRYPTION,
    &SYNCED_PASSWORDS,
];

/// Logical size of the focusable target; the mark sits at its top.
const TARGET: f32 = 16.;
/// Diameter of the drawn mark, raised like a footnote beside its label.
const MARK: f32 = 11.;
/// Extra pointer margin so the effective target is 24px square.
const REACH: f32 = 4.;
/// Space between the mark and the focus ring drawn around it.
const RING_GAP: f32 = 2.;
const GAP: f32 = 6.;
const TIP_WIDTH: f32 = 300.;
/// The tip keeps this margin from every window edge.
const MARGIN: f32 = 8.;

impl App {
    /// The help icon for `topic`, or nothing when help icons are off.
    pub(super) fn help(&self, topic: &'static Topic) -> Element<'static, Message> {
        if !self.preferences.help_icons {
            return space().width(0).into();
        }
        let tip = HelpTip::new(topic);
        #[cfg(feature = "test-support")]
        let tip = tip.observed(self.draw_log.clone());
        tip.into()
    }

    /// A setting control followed by its help mark, raised at the top of the line.
    pub(super) fn with_help<'a>(
        &self,
        control: impl Into<Element<'a, Message>>,
        topic: &'static Topic,
    ) -> Element<'a, Message> {
        row![control.into(), self.help(topic)]
            .spacing(4)
            .align_y(Alignment::Start)
            .into()
    }
}

#[derive(Debug, Default)]
struct State {
    hovered: bool,
    focused: bool,
    /// The last open state that requested a relayout, so a focus change made
    /// by an operation still rebuilds the overlay on the next event.
    shown: bool,
}
impl State {
    fn open(&self) -> bool {
        self.hovered || self.focused
    }
}
impl Focusable for State {
    fn is_focused(&self) -> bool {
        self.focused
    }
    fn focus(&mut self) {
        self.focused = true;
    }
    fn unfocus(&mut self) {
        self.focused = false;
    }
}

pub(super) struct HelpTip {
    topic: &'static Topic,
    tip: Element<'static, Message>,
    #[cfg(feature = "test-support")]
    log: Option<Arc<super::draw_log::Log>>,
}
impl HelpTip {
    pub(super) fn new(topic: &'static Topic) -> Self {
        Self {
            topic,
            tip: container(text(topic.text).size(12).line_height(1.4))
                .padding([9, 12])
                .max_width(TIP_WIDTH)
                .style(components::card)
                .into(),
            #[cfg(feature = "test-support")]
            log: None,
        }
    }
    #[cfg(feature = "test-support")]
    pub(super) fn observed(mut self, log: Arc<super::draw_log::Log>) -> Self {
        self.log = Some(log);
        self
    }
}

/// The drawn mark: centred horizontally at the top of the target, leaving room
/// for the focus ring above it.
pub(super) fn mark_bounds(target: Rectangle) -> Rectangle {
    Rectangle::new(
        Point::new(target.center_x() - MARK / 2., target.y + RING_GAP),
        Size::new(MARK, MARK),
    )
}

/// Where the tip goes: centred below the icon, above it when there is no room
/// below, and always inside the window.
pub(super) fn place(anchor: Rectangle, tip: Size, window: Size) -> Rectangle {
    let x = (anchor.center_x() - tip.width / 2.)
        .min(window.width - MARGIN - tip.width)
        .max(MARGIN);
    let below = anchor.y + anchor.height + GAP;
    let y = if below + tip.height <= window.height - MARGIN {
        below
    } else {
        (anchor.y - GAP - tip.height).max(MARGIN)
    };
    Rectangle::new(Point::new(x, y), tip)
}

impl Widget<Message, Theme, Renderer> for HelpTip {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.tip)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.tip));
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fixed(TARGET), Length::Fixed(TARGET))
    }
    fn layout(&mut self, _: &mut Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.resolve(TARGET, TARGET, Size::new(TARGET, TARGET)))
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        _: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.focusable(
            Some(&Id::new(self.topic.id)),
            layout.bounds(),
            tree.state.downcast_mut::<State>(),
        );
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _: &Renderer,
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let reach = layout.bounds().expand(REACH);
        let over = cursor.is_over(reach) && cursor.is_over(*viewport);
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. }) => state.hovered = over,
            Event::Mouse(mouse::Event::CursorLeft) => state.hovered = false,
            Event::Mouse(mouse::Event::ButtonPressed(_)) => {
                // Clicking the icon pins its help; clicking elsewhere releases it.
                state.focused = over && !state.focused;
            }
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: keyboard::Key::Named(keyboard::key::Named::Escape),
                ..
            }) => state.focused = false,
            Event::Window(iced::window::Event::Unfocused) => state.hovered = false,
            _ => {}
        }
        if state.open() != state.shown {
            state.shown = state.open();
            shell.invalidate_layout();
            shell.request_redraw();
        }
    }
    fn mouse_interaction(
        &self,
        _: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _: &Rectangle,
        _: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds().expand(REACH)) {
            mouse::Interaction::Help
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
        _: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        // Recorded even when scrolled away, so observations keep a stable order.
        #[cfg(feature = "test-support")]
        if let Some(log) = &self.log {
            log.help_icon(self.topic.id, state.focused, state.hovered, bounds);
        }
        let Some(visible) = bounds.expand(3.).intersection(viewport) else {
            return;
        };
        let p = components::colors(theme);
        let (fill, colour, edge) = if state.open() {
            (p.tint, p.accent, p.accent)
        } else {
            (Color::TRANSPARENT, p.muted, p.muted)
        };
        let mark = mark_bounds(bounds);
        renderer::Renderer::fill_quad(
            renderer,
            renderer::Quad {
                bounds: mark,
                border: Border {
                    color: edge,
                    width: 1.,
                    radius: (MARK / 2.).into(),
                },
                ..Default::default()
            },
            fill,
        );
        if state.focused {
            // A visible focus ring around the mark, as for other controls.
            renderer::Renderer::fill_quad(
                renderer,
                renderer::Quad {
                    bounds: mark.expand(RING_GAP),
                    border: Border {
                        color: p.accent,
                        width: 1.5,
                        radius: (MARK / 2. + RING_GAP).into(),
                    },
                    ..Default::default()
                },
                Color::TRANSPARENT,
            );
        }
        renderer.fill_text(
            core_text::Text {
                content: "?".into(),
                bounds: mark.size(),
                size: Pixels(8.5),
                line_height: core_text::LineHeight::Relative(1.),
                font: components::BOLD,
                align_x: core_text::Alignment::Center,
                align_y: iced::alignment::Vertical::Center,
                shaping: core_text::Shaping::Basic,
                wrapping: core_text::Wrapping::None,
            },
            mark.center(),
            colour,
            visible,
        );
    }
    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        _: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let state = tree.state.downcast_ref::<State>();
        let anchor = layout.bounds() + translation;
        if !state.open() || !anchor.intersects(viewport) {
            return None;
        }
        Some(overlay::Element::new(Box::new(Tip {
            #[cfg(feature = "test-support")]
            topic: self.topic,
            anchor,
            tip: &mut self.tip,
            tree: &mut tree.children[0],
            #[cfg(feature = "test-support")]
            log: self.log.as_deref(),
        })))
    }
}

impl From<HelpTip> for Element<'static, Message> {
    fn from(tip: HelpTip) -> Self {
        Element::new(tip)
    }
}

struct Tip<'b> {
    #[cfg(feature = "test-support")]
    topic: &'static Topic,
    anchor: Rectangle,
    tip: &'b mut Element<'static, Message>,
    tree: &'b mut Tree,
    #[cfg(feature = "test-support")]
    log: Option<&'b super::draw_log::Log>,
}

impl overlay::Overlay<Message, Theme, Renderer> for Tip<'_> {
    fn layout(&mut self, renderer: &Renderer, window: Size) -> layout::Node {
        let available = Size::new(
            (window.width - MARGIN * 2.).max(0.),
            (window.height - MARGIN * 2.).max(0.),
        );
        let content = self.tip.as_widget_mut().layout(
            self.tree,
            renderer,
            &layout::Limits::new(Size::ZERO, available),
        );
        let bounds = place(self.anchor, content.size(), window);
        content.move_to(bounds.position())
    }
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        #[cfg(feature = "test-support")]
        if let Some(log) = self.log {
            log.help_tip(self.topic.id, layout.bounds());
        }
        self.tip.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            &layout.bounds(),
        );
    }
}

#[cfg(test)]
mod tests;
