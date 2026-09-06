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
                if menu.mail.unread {
                    "Mark as read"
                } else {
                    "Mark as unread"
                },
                "mail",
                None,
            ),
            (
                Flag,
                if menu.mail.starred {
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
        let mut mail = menu.mail;
        if self.busy.contains(&format!("message:{}", mail.id))
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
                "This message is being updated. Try again when it finishes.",
                true,
            );
            return Task::none();
        }
        match action {
            MailAction::Read | MailAction::Flag => {
                if action == MailAction::Read {
                    mail.unread = !mail.unread;
                } else {
                    mail.starred = !mail.starred;
                }
                self.send(Command::Flags(mail));
            }
            MailAction::Archive | MailAction::Trash => {
                self.send(Command::Move(
                    mail,
                    if action == MailAction::Archive {
                        "Archive"
                    } else {
                        "Trash"
                    }
                    .into(),
                ));
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
}
impl<'a> ContextArea<'a> {
    pub fn new(content: impl Into<Element<'a, Message>>, mail: String) -> Self {
        Self {
            content: content.into(),
            mail: Some(mail),
        }
    }
}
impl<'a> ContextArea<'a> {
    pub fn sidebar(content: impl Into<Element<'a, Message>>) -> Self {
        Self {
            content: content.into(),
            mail: None,
        }
    }
}
impl Widget<Message, Theme, Renderer> for ContextArea<'_> {
    fn tag(&self) -> iced::advanced::widget::tree::Tag {
        iced::advanced::widget::tree::Tag::of::<keyboard::Modifiers>()
    }
    fn state(&self) -> iced::advanced::widget::tree::State {
        iced::advanced::widget::tree::State::new(keyboard::Modifiers::default())
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
        event: &iced::Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let modifiers = tree.state.downcast_mut::<keyboard::Modifiers>();
        if let iced::Event::Keyboard(keyboard::Event::ModifiersChanged(value)) = event {
            *modifiers = *value;
        }
        if matches!(event, iced::Event::Window(iced::window::Event::Unfocused)) {
            *modifiers = keyboard::Modifiers::default();
        }
        let modifiers = *modifiers;
        if matches!(
            event,
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right))
        ) && cursor.is_over(layout.bounds())
            && cursor.is_over(*viewport)
            && let Some(mail) = &self.mail
            && let Some(position) = cursor.position()
        {
            shell.publish(Message::MailContext(mail.clone(), position));
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
        shell.merge(child, |message| match message {
            Message::SidebarAction(index) => Message::SidebarClick(index, modifiers),
            Message::Select(id) => Message::SelectClick(id, modifiers),
            other => other,
        });
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
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}
impl<'a> From<ContextArea<'a>> for Element<'a, Message> {
    fn from(area: ContextArea<'a>) -> Self {
        Self::new(area)
    }
}
