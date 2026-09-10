//! Gesture bookkeeping stays inside widgets: pointer motion only redraws. Start,
//! target transitions and release publish small messages to the app controller.
use super::*;
use iced::advanced::{Layout, Shell, mouse, renderer, text};
use iced::{Point, Rectangle, Renderer};
use std::{cell::RefCell, rc::Rc};
use text::Renderer as _;

type Paragraph = <Renderer as text::Renderer>::Paragraph;
#[derive(Default)]
struct Inner {
    session: Option<Session>,
    position: Point,
    blocked: bool,
    escape_consumed: bool,
    suppress_release: bool,
    hover: Option<(Target, Option<&'static str>)>,
    over_reveal: Option<Reveal>,
    revealing: Option<RevealHover>,
    label: String,
    paragraph: text::paragraph::Plain<Paragraph>,
}
struct RevealHover {
    item: Reveal,
    since: Instant,
    fired: bool,
}
struct Session {
    payload: Arc<Payload>,
    origin: Point,
    active: bool,
    cancelled: bool,
}
#[derive(Clone, Default)]
pub(in crate::ui) struct Handle(Rc<RefCell<Inner>>);
impl Handle {
    pub fn active(&self) -> bool {
        self.0
            .borrow()
            .session
            .as_ref()
            .is_some_and(|s| s.active && !s.cancelled)
    }
    pub fn holding(&self) -> bool {
        self.0.borrow().session.is_some()
    }
    pub fn consume_escape(&self) -> bool {
        std::mem::take(&mut self.0.borrow_mut().escape_consumed)
    }
    pub fn clear(&self) {
        let mut state = self.0.borrow_mut();
        state.suppress_release |= state.session.is_some();
        state.session = None;
        state.hover = None;
        state.revealing = None;
        state.over_reveal = None;
    }
    pub fn observation(&self) -> serde_json::Value {
        let state = self.0.borrow();
        serde_json::json!({
            "active":state.session.as_ref().is_some_and(|s|s.active && !s.cancelled),
            "count":state.session.as_ref().map(|s|s.payload.count()),
            "target":state.hover.as_ref().map(|(t,_)|&t.folder),
            "account":state.hover.as_ref().and_then(|(t,_)|t.account.as_deref()),
            "valid":state.hover.as_ref().is_some_and(|(_,error)|error.is_none()),
            "reason":state.hover.as_ref().and_then(|(_,error)|*error),
        })
    }
}
#[derive(Clone)]
pub(in crate::ui) enum Region {
    Root(Handle, bool, Rules),
    Reveal(Handle, Reveal),
    Source(Handle, Option<Arc<Payload>>),
    Block(Handle),
    Target(Handle, Target, Rules),
}
#[derive(Default)]
pub(in crate::ui) struct Cycle {
    active: bool,
    hover: Option<(Target, Option<&'static str>)>,
    suppress: bool,
}
impl Cycle {
    pub fn filter(&self, message: Message) -> Message {
        if self.suppress && !matches!(message, Message::InboxScroll(_)) {
            Message::Noop
        } else {
            message
        }
    }
}
impl Region {
    pub fn before(&self, event: &iced::Event, cursor: mouse::Cursor) -> Cycle {
        match self {
            Self::Root(handle, enabled, _) => {
                let mut state = handle.0.borrow_mut();
                let mut cycle = Cycle {
                    active: state.session.as_ref().is_some_and(|s| s.active),
                    hover: state.hover.clone(),
                    ..Default::default()
                };
                if !enabled {
                    state.session = None;
                    state.hover = None;
                    return cycle;
                }
                if let Some(position) = cursor.position() {
                    state.position = position;
                }
                match event {
                    iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                        state.session = None;
                        state.blocked = false;
                        state.suppress_release = false;
                    }
                    iced::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                        let position = state.position;
                        if let Some(session) = state.session.as_mut() {
                            let delta = position - session.origin;
                            if !session.cancelled && delta.x * delta.x + delta.y * delta.y >= 36. {
                                session.active = true;
                            }
                        }
                    }
                    iced::Event::Keyboard(keyboard::Event::KeyPressed {
                        key: Key::Named(keyboard::key::Named::Escape),
                        ..
                    }) if state.session.is_some() => {
                        state.escape_consumed = true;
                        if let Some(session) = state.session.as_mut() {
                            session.cancelled = true;
                            session.active = false;
                        }
                        cycle.suppress = true;
                    }
                    iced::Event::Window(iced::window::Event::Unfocused)
                    | iced::Event::Mouse(mouse::Event::CursorLeft)
                    | iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
                        if state.session.is_some() =>
                    {
                        state.session = None;
                        state.suppress_release = true;
                        cycle.suppress = true;
                    }
                    _ => {}
                }
                cycle.suppress |= state.suppress_release
                    && matches!(
                        event,
                        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                    );
                cycle.suppress |= cycle.active
                    || state
                        .session
                        .as_ref()
                        .is_some_and(|s| s.active || s.cancelled);
                state.hover = None;
                state.over_reveal = None;
                cycle
            }
            _ => Cycle::default(),
        }
    }
    pub fn after(
        &self,
        cycle: &Cycle,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        shell: &mut Shell<'_, Message>,
    ) {
        let over = cursor.is_over(layout.bounds()) && cursor.is_over(*viewport);
        match self {
            Self::Source(handle, Some(payload))
                if over
                    && matches!(
                        event,
                        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                    ) =>
            {
                let mut state = handle.0.borrow_mut();
                if !state.blocked && state.session.is_none() {
                    state.session = Some(Session {
                        payload: payload.clone(),
                        origin: state.position,
                        active: false,
                        cancelled: false,
                    });
                }
            }
            Self::Block(handle)
                if over
                    && matches!(
                        event,
                        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                    ) =>
            {
                handle.0.borrow_mut().blocked = true;
            }
            Self::Target(handle, target, rules) if over => {
                let mut state = handle.0.borrow_mut();
                if let Some(session) = &state.session
                    && session.active
                    && !session.cancelled
                {
                    let error = rules.check(&session.payload, target).err();
                    state.hover = Some((target.clone(), error));
                }
            }
            Self::Reveal(handle, item) if over => {
                let mut state = handle.0.borrow_mut();
                if state
                    .session
                    .as_ref()
                    .is_some_and(|s| s.active && !s.cancelled)
                {
                    state.over_reveal = Some(item.clone());
                }
            }
            Self::Root(handle, ..) => {
                let mut state = handle.0.borrow_mut();
                if matches!(
                    event,
                    iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                ) {
                    if let Some(session) = state.session.take()
                        && session.active
                        && !session.cancelled
                    {
                        shell.publish(Message::DropMail(
                            session.payload,
                            state.hover.as_ref().map(|(t, _)| t.clone()),
                        ));
                    }
                    state.hover = None;
                    state.suppress_release = false;
                }
                let active = state
                    .session
                    .as_ref()
                    .is_some_and(|s| s.active && !s.cancelled);
                if active {
                    if let Some(item) = state.over_reveal.clone() {
                        if state.revealing.as_ref().is_none_or(|r| r.item != item) {
                            state.revealing = Some(RevealHover {
                                item,
                                since: Instant::now(),
                                fired: false,
                            });
                        }
                        let reveal = state.revealing.as_mut().unwrap();
                        let deadline = reveal.since + std::time::Duration::from_millis(600);
                        if !reveal.fired {
                            if Instant::now() >= deadline {
                                shell.publish(Message::DragReveal(reveal.item.clone()));
                                reveal.fired = true;
                            } else {
                                shell.request_redraw_at(deadline);
                            }
                        }
                    } else {
                        state.revealing = None;
                    }
                } else {
                    state.revealing = None;
                    state.over_reveal = None;
                }
                if cycle.active != active || cycle.hover != state.hover {
                    shell.publish(Message::DragChanged);
                    shell.request_redraw();
                }
                if active && matches!(event, iced::Event::Mouse(mouse::Event::CursorMoved { .. })) {
                    shell.request_redraw();
                }
                if cycle.suppress {
                    shell.capture_event();
                }
            }
            _ => {}
        }
    }
    pub fn interaction(&self) -> Option<mouse::Interaction> {
        let Self::Root(handle, ..) = self else {
            return None;
        };
        let state = handle.0.borrow();
        state
            .session
            .as_ref()
            .filter(|s| s.active && !s.cancelled)
            .map(|_| {
                if state
                    .hover
                    .as_ref()
                    .is_some_and(|(_, error)| error.is_none())
                {
                    mouse::Interaction::Grabbing
                } else {
                    mouse::Interaction::NoDrop
                }
            })
    }
    pub fn draw(
        &self,
        layout: Layout<'_>,
        renderer: &mut Renderer,
        theme: &Theme,
        viewport: &Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        let (Self::Root(handle, ..) | Self::Target(handle, ..)) = self else {
            return;
        };
        let mut state = handle.0.borrow_mut();
        let Some(session) = &state.session else {
            return;
        };
        if !session.active || session.cancelled {
            return;
        }
        let p = colors(theme);
        if let Self::Target(_, target, rules) = self {
            if !layout.bounds().intersects(viewport) {
                return;
            }
            let valid = rules.check(&session.payload, target).is_ok();
            let hovered = state.hover.as_ref().is_some_and(|(t, _)| t == target);
            if (valid || hovered)
                && let Some(bounds) = layout.bounds().intersection(viewport)
            {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        border: iced::Border {
                            color: if valid { p.accent } else { p.flag },
                            width: if hovered { 2. } else { 1. },
                            radius: 7.into(),
                        },
                        ..Default::default()
                    },
                    iced::Color::TRANSPARENT,
                );
            }
            return;
        }
        let count = session.payload.count();
        let mut label = format!(
            "Move {count} {}",
            if count == 1 { "message" } else { "messages" }
        );
        if let Some((target, error)) = &state.hover {
            let folder = if let Self::Root(_, _, rules) = self {
                rules.workspace.folder_label(
                    target
                        .account
                        .as_deref()
                        .or_else(|| session.payload.groups().next().map(|(account, _)| account)),
                    &target.folder,
                )
            } else {
                std::borrow::Cow::Borrowed(target.folder.as_str())
            };
            label.push_str(&format!(
                " → {}",
                super::super::views::truncate(&folder, 32)
            ));
            if let Self::Root(_, _, rules) = self
                && let Some(id) = &target.account
                && let Some(account) = rules.workspace.accounts.iter().find(|a| &a.id == id)
            {
                label.push_str(&format!(
                    " · {}",
                    super::super::views::truncate(&account.name, 24)
                ));
            }
            if let Some(error) = error {
                label.push('\n');
                label.push_str(error);
            }
        }
        if state.over_reveal.is_some() {
            label.push_str("\nHold to open folders");
        }
        if state.label != label {
            state.paragraph.update(text::Text {
                content: &label,
                bounds: Size::new(290., f32::INFINITY),
                size: 12.into(),
                line_height: text::LineHeight::Relative(1.4),
                font: iced::Font::DEFAULT,
                align_x: text::Alignment::Left,
                align_y: iced::alignment::Vertical::Top,
                shaping: text::Shaping::Advanced,
                wrapping: text::Wrapping::Word,
            });
            state.label = label;
        }
        let size = state.paragraph.min_bounds() + Size::new(24., 18.);
        let bounds = Rectangle::new(
            Point::new(
                (state.position.x + 16.)
                    .min(viewport.x + viewport.width - size.width - 8.)
                    .max(viewport.x + 8.),
                (state.position.y + 18.)
                    .min(viewport.y + viewport.height - size.height - 8.)
                    .max(viewport.y + 8.),
            ),
            size,
        );
        renderer.with_layer(*viewport, |renderer| {
            renderer.fill_quad(
                renderer::Quad {
                    bounds,
                    border: iced::Border {
                        color: p.accent,
                        width: 1.,
                        radius: 8.into(),
                    },
                    shadow: iced::Shadow {
                        color: iced::Color::from_rgba(0., 0., 0., 0.18),
                        offset: iced::Vector::new(0., 3.),
                        blur_radius: 10.,
                    },
                    ..Default::default()
                },
                p.surface,
            );
            renderer.fill_paragraph(
                state.paragraph.raw(),
                Point::new(bounds.x + 12., bounds.y + 9.),
                p.text,
                *viewport,
            );
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hover_reveal_fires_once_after_dwell_and_escape_cancels_the_pending_drop() {
        let handle = Handle::default();
        let mail = parse_mail(
            "a",
            "1",
            "INBOX",
            b"Subject: Fixture\r\n\r\nBody".to_vec(),
            true,
            false,
        )
        .unwrap()
        .summary;
        handle.0.borrow_mut().session = Some(Session {
            payload: Arc::new(Payload::Single(Box::new(mail))),
            origin: Point::ORIGIN,
            active: true,
            cancelled: false,
        });
        let root = Region::Root(
            handle.clone(),
            true,
            Rules {
                workspace: Arc::new(Workspace::default()),
                cross_account: false,
            },
        );
        let reveal = Region::Reveal(handle.clone(), Reveal::Inbox);
        let node = iced::advanced::layout::Node::new(Size::new(100., 40.));
        let viewport = Rectangle::with_size(Size::new(200., 100.));
        let event = iced::Event::Window(iced::window::Event::RedrawRequested(Instant::now()));
        let cursor = mouse::Cursor::Available(Point::new(10., 10.));
        let mut messages = vec![];
        for expired in [false, true, true] {
            if expired {
                handle.0.borrow_mut().revealing.as_mut().unwrap().since =
                    Instant::now() - std::time::Duration::from_secs(1);
            }
            let cycle = root.before(&event, cursor);
            reveal.after(
                &Cycle::default(),
                &event,
                Layout::new(&node),
                cursor,
                &viewport,
                &mut Shell::new(&mut messages),
            );
            root.after(
                &cycle,
                &event,
                Layout::new(&node),
                cursor,
                &viewport,
                &mut Shell::new(&mut messages),
            );
            if !expired {
                assert!(
                    messages
                        .iter()
                        .all(|m| !matches!(m, Message::DragReveal(_)))
                );
            }
        }
        assert_eq!(
            messages
                .iter()
                .filter(|m| matches!(m, Message::DragReveal(Reveal::Inbox)))
                .count(),
            1
        );
        // Cancelling a drag cannot become Escape's ordinary clear-selection action.
        let event = iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: Key::Named(keyboard::key::Named::Escape),
            modified_key: Key::Named(keyboard::key::Named::Escape),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Escape),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            text: None,
            repeat: false,
        });
        let cycle = root.before(&event, cursor);
        root.after(
            &cycle,
            &event,
            Layout::new(&node),
            cursor,
            &viewport,
            &mut Shell::new(&mut messages),
        );
        assert!(handle.consume_escape());
        assert!(!handle.consume_escape());
        assert!(!handle.active());
        let release = iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        let cycle = root.before(&release, cursor);
        root.after(
            &cycle,
            &release,
            Layout::new(&node),
            cursor,
            &viewport,
            &mut Shell::new(&mut messages),
        );
        assert!(!handle.holding());
        assert!(messages.iter().all(|m| !matches!(m, Message::DropMail(..))));
    }
}
