use super::*;
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
    widget::{Operation, Tree},
};
use iced::widget::{button, column, container, opaque, row, space, text};
use iced::{Alignment, Length, Point, Rectangle, Renderer, Vector};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MailAction {
    Open,
    Reply,
    ReplyAll,
    Read,
    Flag,
    Move,
    Archive,
    Trash,
    CopySender,
    Export,
}
#[derive(Debug, Clone)]
pub(super) struct Menu {
    pub mail: Mail,
    pub position: Point,
    pub index: usize,
}
impl App {
    pub(super) fn shortcut_hint(&self, label: &str, action: Action) -> String {
        let keys = if self.preferences.shortcut_tooltips {
            self.preferences.shortcuts.key(action).replace(
                "Mod",
                if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl"
                },
            )
        } else {
            String::new()
        };
        if keys.is_empty() {
            label.into()
        } else {
            format!("{label} · {keys}")
        }
    }
    pub(super) fn mail_menu_items(
        &self,
    ) -> Vec<(MailAction, &'static str, &'static str, Option<Action>)> {
        let Some(menu) = &self.context_menu else {
            return vec![];
        };
        use MailAction::*;
        vec![
            (Open, "Open message", "mail", Some(Action::OpenMessage)),
            (Reply, "Reply", "reply", Some(Action::Reply)),
            (ReplyAll, "Reply all", "reply", Some(Action::ReplyAll)),
            (
                Read,
                if self.mail_actions.effective(&menu.mail).unread {
                    "Mark as read"
                } else {
                    "Mark as unread"
                },
                "mail",
                None,
            ),
            (
                Flag,
                if self.mail_actions.effective(&menu.mail).starred {
                    "Remove flag"
                } else {
                    "Flag message"
                },
                "flag",
                Some(Action::Star),
            ),
            (Move, "Move to folder…", "move", Some(Action::Move)),
            (Archive, "Archive", "archive", Some(Action::Archive)),
            (Trash, "Move to Trash", "trash", Some(Action::Delete)),
            (CopySender, "Copy sender address", "copy", None),
            (Export, "Export message…", "download", None),
        ]
    }
    pub(super) fn mail_context_view(&self) -> Element<'_, Message> {
        let Some(menu) = &self.context_menu else {
            return space().into();
        };
        let mut items = column![].spacing(2);
        for (index, (action, label, glyph, shortcut)) in
            self.mail_menu_items().into_iter().enumerate()
        {
            let focused = menu.index == index;
            items = items.push(
                button(
                    row![
                        icon(glyph, 18.),
                        text(label).size(12),
                        space().width(Length::Fill),
                        muted(
                            shortcut
                                .map(|a| self.preferences.shortcuts.label(a))
                                .unwrap_or_default()
                        )
                        .size(11),
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                )
                .padding([10, 12])
                .width(Length::Fill)
                .style(if focused { selected } else { ghost })
                .on_press(Message::MailContextAction(action)),
            );
        }
        let scale = self.preferences.interface_scale as f32 / 100.;
        let x = menu
            .position
            .x
            .clamp(8., (self.size.width / scale - 280.).max(8.));
        let y = menu
            .position
            .y
            .clamp(8., (self.size.height / scale - 430.).max(8.));
        container(opaque(container(items).width(268).padding(6).style(card)))
            .padding(iced::Padding {
                left: x,
                top: y,
                ..Default::default()
            })
            .into()
    }
    pub(super) fn choose_mail_context(&mut self, action: MailAction) -> Task<Message> {
        let Some(menu) = self.context_menu.take() else {
            return Task::none();
        };
        let mail = self.mail_actions.effective(&menu.mail).clone();
        if self.bulk_owns_mail(&mail.id)
            && matches!(
                action,
                MailAction::Read
                    | MailAction::Flag
                    | MailAction::Move
                    | MailAction::Archive
                    | MailAction::Trash
            )
        {
            self.notice(
                "This message is part of a group change. Finish it or open History to review it.",
                true,
            );
            return Task::none();
        }
        if self.busy.contains(&format!("message:{}", mail.id))
            && matches!(
                action,
                MailAction::Move | MailAction::Archive | MailAction::Trash
            )
        {
            self.notice(
                "This message is being updated. Try again when it finishes.",
                true,
            );
            return Task::none();
        }
        match action {
            MailAction::Read | MailAction::Flag => {
                self.toggle_mail_flag(mail, action == MailAction::Read);
            }
            MailAction::Archive | MailAction::Trash => {
                self.move_mail(
                    mail,
                    if action == MailAction::Archive {
                        "Archive"
                    } else {
                        "Trash"
                    }
                    .into(),
                );
            }
            MailAction::CopySender => {
                return iced::clipboard::write(
                    crate::remote_images::sender_address(&mail.sender).unwrap_or(mail.sender),
                );
            }
            MailAction::Open => return self.handle(Message::OpenMessage(mail.id)),
            _ => {
                self.select(mail.id.clone());
                self.pending_mail_action = Some((mail.id, action));
                return self.finish_mail_context();
            }
        }
        Task::none()
    }
    pub(super) fn finish_mail_context(&mut self) -> Task<Message> {
        let Some((id, action)) = &self.pending_mail_action else {
            return Task::none();
        };
        if self.selected.as_ref() != Some(id) {
            self.pending_mail_action = None;
            return Task::none();
        }
        if self.detail.as_ref().is_none_or(|d| d.summary.id != *id) {
            return Task::none();
        }
        let message = match action {
            MailAction::Reply => Message::Reply,
            MailAction::ReplyAll => Message::ReplyAll,
            MailAction::Move => Message::Open(Dialog::Move),
            MailAction::Export => Message::Open(Dialog::Export),
            _ => {
                self.pending_mail_action = None;
                return Task::none();
            }
        };
        self.pending_mail_action = None;
        self.handle(message)
    }
}

/// Captures the click's real position without subscribing the app to every mouse move.
pub(super) struct ContextArea<'a> {
    content: Element<'a, Message>,
    mail: Option<String>,
    draft: Option<String>,
    preserve_pointer: bool,
    interface_scale: u16,
    drag: Option<drag_mail::Region>,
    #[cfg(feature = "test-support")]
    draw_witness: Option<(u64, Arc<std::sync::atomic::AtomicU64>)>,
}
impl<'a> ContextArea<'a> {
    pub fn new(content: impl Into<Element<'a, Message>>, mail: String) -> Self {
        Self {
            content: content.into(),
            mail: Some(mail),
            draft: None,
            preserve_pointer: false,
            interface_scale: 100,
            drag: None,
            #[cfg(feature = "test-support")]
            draw_witness: None,
        }
    }
    pub fn with_drag(mut self, region: drag_mail::Region) -> Self {
        self.drag = Some(region);
        self
    }
    #[cfg(feature = "test-support")]
    pub fn with_draw_witness(
        mut self,
        epoch: u64,
        drawn: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        self.draw_witness = Some((epoch, drawn));
        self
    }
}
impl<'a> ContextArea<'a> {
    /// The runtime supplies the final cursor for a whole input batch. Preserve
    /// each motion before dispatch, outside scrollable coordinate transforms.
    pub fn root(content: impl Into<Element<'a, Message>>, interface_scale: u16) -> Self {
        let mut area = Self::sidebar(content);
        area.preserve_pointer = true;
        area.interface_scale = interface_scale;
        area
    }
    pub fn sidebar(content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            content: content.into(),
            mail: None,
            draft: None,
            preserve_pointer: false,
            interface_scale: 100,
            drag: None,
            #[cfg(feature = "test-support")]
            draw_witness: None,
        }
    }
    pub fn draft(content: impl Into<Element<'a, Message>>, id: String) -> Self {
        Self {
            content: content.into(),
            mail: None,
            draft: Some(id),
            preserve_pointer: false,
            interface_scale: 100,
            drag: None,
            #[cfg(feature = "test-support")]
            draw_witness: None,
        }
    }
}
#[derive(Default)]
struct InputState {
    modifiers: keyboard::Modifiers,
    pointer: pointer::Tracker,
    interface_scale: u16,
}
impl Widget<Message, Theme, Renderer> for ContextArea<'_> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<InputState>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(InputState {
            interface_scale: self.interface_scale,
            ..InputState::default()
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<InputState>();
        if state.interface_scale != self.interface_scale {
            state.pointer.clear();
            state.interface_scale = self.interface_scale;
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
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<InputState>();
        if let iced::Event::Keyboard(keyboard::Event::ModifiersChanged(value)) = event {
            state.modifiers = *value;
        }
        if matches!(event, iced::Event::Window(iced::window::Event::Unfocused)) {
            state.modifiers = keyboard::Modifiers::default();
        }
        let modifiers = state.modifiers;
        let cursor = if self.preserve_pointer {
            state.pointer.update(event, cursor)
        } else {
            cursor
        };
        let drag_cycle = self.drag.as_ref().map(|drag| drag.before(event, cursor));
        if matches!(
            event,
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
        ) && cursor.is_over(layout.bounds())
            && cursor.is_over(*viewport)
            && (self.mail.is_some() || self.draft.is_some())
            && let Some(position) = cursor.position()
        {
            if let Some(mail) = &self.mail {
                shell.publish(Message::MailContext(mail.clone(), position));
            } else if let Some(draft) = &self.draft {
                shell.publish(Message::DraftContext(draft.clone(), position));
            }
            shell.capture_event();
            return;
        }
        let mut messages = Vec::new();
        let mut child = Shell::new(&mut messages);
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            &mut child,
            viewport,
        );
        shell.merge(child, |message| {
            let message = if let Some(cycle) = &drag_cycle {
                cycle.filter(message)
            } else {
                message
            };
            match message {
                Message::SidebarAction(index) => Message::SidebarClick(index, modifiers),
                Message::Select(id) => Message::SelectClick(id, modifiers),
                Message::OpenMessage(id) => Message::OpenMessageClick(id, modifiers),
                other => other,
            }
        });
        if let Some(drag) = &self.drag {
            drag.after(
                drag_cycle.as_ref().unwrap(),
                event,
                layout,
                cursor,
                viewport,
                shell,
            );
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
        if let Some(interaction) = self.drag.as_ref().and_then(|drag| drag.interaction()) {
            return interaction;
        }
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
        if let Some(drag) = &self.drag {
            drag.draw(layout, renderer, theme, viewport);
        }
        // Observe the real widget draw, never a controller acknowledgment.
        // Native tests can wait for the checkbox layout before injecting input.
        #[cfg(feature = "test-support")]
        if let Some((epoch, drawn)) = &self.draw_witness
            && layout.bounds().intersects(viewport)
        {
            drawn.store(*epoch, std::sync::atomic::Ordering::Release);
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
        let tracker = tree.state.downcast_ref::<InputState>().pointer.clone();
        self.content
            .as_widget_mut()
            .overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
            )
            .map(|content| {
                if self.preserve_pointer {
                    tracker.wrap(content)
                } else {
                    content
                }
            })
    }
}
impl<'a> From<ContextArea<'a>> for Element<'a, Message> {
    fn from(area: ContextArea<'a>) -> Self {
        Self::new(area)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_clicks_keep_their_positions_before_scroll_transforms() {
        let mut area = ContextArea::root(
            iced::widget::column![
                button("First")
                    .width(100)
                    .height(40)
                    .on_press(Message::CheckMail("first".into())),
                button("Second")
                    .width(100)
                    .height(40)
                    .on_press(Message::CheckMail("second".into())),
            ]
            .spacing(10),
            100,
        );
        let mut renderer = Renderer::new(iced::Font::DEFAULT, iced::Pixels(16.));
        let mut tree = Tree::new(&area as &dyn Widget<Message, Theme, Renderer>);
        let bounds = Size::new(200., 200.);
        let node = area.layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );
        let mut messages = Vec::new();
        let last_cursor = mouse::Cursor::Available(Point::new(10., 60.));
        let first = Point::new(10., 10.);
        let second = Point::new(10., 60.);
        for position in [first, second] {
            for event in [
                iced::Event::Mouse(mouse::Event::CursorMoved { position }),
                iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
            ] {
                area.update(
                    &mut tree,
                    &event,
                    Layout::new(&node),
                    last_cursor,
                    &renderer,
                    &mut iced::advanced::clipboard::Null,
                    &mut Shell::new(&mut messages),
                    &Rectangle::with_size(bounds),
                );
                if matches!(event, iced::Event::Mouse(mouse::Event::CursorMoved { .. })) {
                    area.update(
                        &mut tree,
                        &iced::Event::Window(iced::window::Event::RedrawRequested(Instant::now())),
                        Layout::new(&node),
                        last_cursor,
                        &renderer,
                        &mut iced::advanced::clipboard::Null,
                        &mut Shell::new(&mut messages),
                        &Rectangle::with_size(bounds),
                    );
                    area.draw(
                        &tree,
                        &mut renderer,
                        &Theme::Light,
                        &renderer::Style::default(),
                        Layout::new(&node),
                        mouse::Cursor::Available(position),
                        &Rectangle::with_size(bounds),
                    );
                }
            }
        }
        let ids: Vec<_> = messages
            .iter()
            .filter_map(|m| match m {
                Message::CheckMail(id) => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, ["first", "second"]);
        // Drawing must not erase motion before its corresponding click.
        area.draw(
            &tree,
            &mut renderer,
            &Theme::Light,
            &renderer::Style::default(),
            Layout::new(&node),
            last_cursor,
            &Rectangle::with_size(bounds),
        );
    }
}
