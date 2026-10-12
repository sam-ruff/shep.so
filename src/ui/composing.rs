use super::views::{Choice, truncate};
use super::*;
use crate::store::DraftState;
use crate::ui::dropdown::pick_list;
mod history;
mod recipient_box;
mod recipients;
mod sessions;
#[cfg(test)]
#[path = "composing_tests.rs"]
mod tests;
use history::{Edit, History};
use iced::{
    Alignment, Length,
    widget::{button, column, container, row, scrollable, space, text},
};
use recipients::Recipients;

const ADDRESS_FIELDS: [&str; 3] = ["to", "cc", "bcc"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Undo,
    Redo,
}

impl Step {
    fn apply<T: history::Weight>(
        self,
        history: &mut History<T>,
        current: impl FnOnce() -> T,
    ) -> Option<T> {
        match self {
            Step::Undo => history.undo(current),
            Step::Redo => history.redo(current),
        }
    }
}

/// Ctrl+Z undoes; Ctrl+Shift+Z and Ctrl+Y redo (Command on macOS).
pub(super) fn history_step(key: &Key, modifiers: keyboard::Modifiers) -> Option<Step> {
    let Key::Character(text) = key else {
        return None;
    };
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => step_for(c, modifiers),
        _ => None,
    }
}

fn step_for(c: char, modifiers: keyboard::Modifiers) -> Option<Step> {
    if !modifiers.command() || modifiers.alt() {
        return None;
    }
    match c.to_ascii_lowercase() {
        'z' if modifiers.shift() => Some(Step::Redo),
        'z' => Some(Step::Undo),
        'y' if !modifiers.shift() => Some(Step::Redo),
        _ => None,
    }
}

fn address_index(key: &str) -> Option<usize> {
    ADDRESS_FIELDS.iter().position(|field| *field == key)
}

fn address_mut(draft: &mut Draft, index: usize) -> &mut String {
    match index {
        0 => &mut draft.to,
        1 => &mut draft.cc,
        _ => &mut draft.bcc,
    }
}

fn addresses(draft: &Draft) -> [Recipients; 3] {
    [draft.to.as_str(), draft.cc.as_str(), draft.bcc.as_str()].map(Recipients::parse)
}

/// The message body with its cursor, as one undo step saves it.
pub(super) struct BodyState {
    text: String,
    cursor: text_editor::Cursor,
}

impl BodyState {
    fn capture(editor: &text_editor::Content) -> Self {
        Self {
            text: editor.text(),
            cursor: editor.cursor(),
        }
    }

    fn restore(self) -> text_editor::Content {
        let mut editor = text_editor::Content::with_text(&self.text);
        editor.move_to(self.cursor);
        editor
    }
}

impl history::Weight for BodyState {
    fn weight(&self) -> usize {
        self.text.len()
    }
}

/// A recipient entry's new text, with the view it was drawn from.
#[derive(Debug, Clone)]
pub struct EntryInput {
    generation: u64,
    /// The entry text that view gave the input.
    base: String,
    value: String,
}

/// The text a recipient entry widget holds, which can trail the model until
/// the next view: every event in one batch edits the input's previous text.
#[derive(Default)]
pub(super) struct EntryWidget {
    generation: u64,
    seen: Option<u64>,
    value: String,
}

#[derive(Default)]
pub(super) struct Histories {
    body: History<BodyState>,
    subject: History<String>,
    addresses: [History<Recipients>; 3],
}

fn compose_binding(
    key: text_editor::KeyPress,
    shortcuts: &crate::shortcuts::Keymap,
) -> Option<text_editor::Binding<Message>> {
    let modifiers = key.modifiers;
    if !matches!(key.status, text_editor::Status::Focused { .. }) {
        return None;
    }
    if let Some(step) = key
        .key
        .to_latin(key.physical_key)
        .and_then(|c| step_for(c, modifiers))
    {
        return Some(text_editor::Binding::Custom(Message::ComposeHistory(
            "compose-body",
            step,
        )));
    }
    let navigation = modifiers.command()
        && chord(&key.key, modifiers)
            .as_deref()
            .and_then(|chord| shortcuts.resolve(chord))
            .is_some_and(|action| {
                matches!(
                    action,
                    Action::Search
                        | Action::Find
                        | Action::Mail
                        | Action::Calendar
                        | Action::Settings
                        | Action::Activity
                )
            });
    if navigation {
        return None;
    }
    let binding = text_editor::Binding::from_key_press(key)?;
    if matches!(binding, text_editor::Binding::Insert(_)) && modifiers.command() && !modifiers.alt()
    {
        return None;
    }
    Some(binding)
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Exit {
    Tab(Tab),
    Window(iced::window::Id),
}

#[derive(Default)]
pub(super) struct Session {
    pub ui_key: u64,
    pub draft: Draft,
    pub editor: text_editor::Content,
    pub dirty: Option<Instant>,
    pub pending: Option<u64>,
    pub saved_revision: Option<u64>,
    pub save_error: Option<(u64, String)>,
    pub explicit_save: bool,
    pub show_recipients: bool,
    pub minimized: bool,
    /// To, Cc and Bcc as chips; the draft keeps their comma-separated value.
    pub addresses: [Recipients; 3],
    pub entries: [EntryWidget; 3],
    pub history: Histories,
}
#[derive(Default)]
pub(super) struct Composer {
    pub next_ui_key: u64,
    pub current: Session,
    pub parked: HashMap<String, Session>,
    pub resume: Option<String>,
    pub close: Option<iced::window::Id>,
    pub dismissed_for: Option<String>,
    pub io: Option<String>,
    pub picker: Option<String>,
    pub context: Option<DraftMenu>,
    discard: Option<Draft>,
    discard_return: Option<Dialog>,
    pub discard_pending: bool,
    pub forward_pending: Option<(String, String, u64)>,
    forward_error: Option<String>,
}

pub(super) struct DraftMenu {
    pub id: String,
    pub position: iced::Point,
    pub discard: bool,
}

impl App {
    pub(super) fn begin_forward(&mut self, source: String) {
        if self.dialog.is_some() || self.composer.forward_pending.is_some() {
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        if self.try_command(Command::ForwardDraft(source.clone(), id.clone())) {
            self.composer.forward_pending = Some((id, source, self.detail_revision));
        }
    }

    pub(super) fn forward_ready(
        &mut self,
        id: String,
        result: Result<Arc<DraftState>, String>,
    ) -> Task<Message> {
        let Some((request, source, revision)) = self.composer.forward_pending.as_ref() else {
            return Task::none();
        };
        if request != &id {
            return Task::none();
        }
        let open = self.tab == Tab::Mail
            && self.dialog.is_none()
            && self.composer.current.draft.id.is_empty()
            && self.reader_id() == Some(source.as_str())
            && self.detail_revision == *revision;
        self.composer.forward_pending = None;
        match result {
            Ok(state) => {
                if let Some(error) = self.composer.forward_error.take()
                    && self
                        .notice
                        .as_ref()
                        .is_some_and(|notice| notice.1 && notice.0 == error)
                {
                    self.notice = None;
                }
                self.observe_drafts(&state);
                if open
                    && let Some(draft) = self
                        .workspace
                        .drafts
                        .iter()
                        .find(|draft| draft.id == id)
                        .cloned()
                {
                    self.load_draft(draft);
                    return self.focus_after_layout("to");
                }
                self.notice("Forward saved in Drafts.", false);
            }
            Err(error) => {
                self.pending_close = None;
                self.composer.close = None;
                self.composer.forward_error = Some(error.clone());
                self.notice(error, true);
            }
        }
        Task::none()
    }

    pub(super) fn review_discard_draft(&mut self, id: String) {
        if self.composer.io.as_deref() == Some(id.as_str()) {
            self.notice("Finish attaching files before discarding this draft.", true);
            return;
        }
        if self.busy.contains(&format!("send:{id}")) || self.composer.discard_pending {
            return;
        }
        if self.workspace.outgoing_drafts.contains(&id) {
            self.notice(
                "Review this message in Outbox before discarding its draft.",
                true,
            );
            return;
        }
        let draft = self.owned_draft(&id);
        if let Some(draft) = draft {
            self.composer.discard = Some(draft);
            self.composer.discard_return = self.dialog;
            self.composer.close = None;
            self.composer.context = None;
            self.dialog = Some(Dialog::DiscardDraft);
        }
    }

    pub(super) fn cancel_discard_draft(&mut self) {
        if !self.composer.discard_pending {
            self.dialog = self.composer.discard_return.take();
            self.composer.discard = None;
        }
    }

    pub(super) fn confirm_discard_draft(&mut self) {
        if self.dialog != Some(Dialog::DiscardDraft) || self.composer.discard_pending {
            return;
        }
        if let Some(draft) = &self.composer.discard
            && self.try_command(Command::DeleteDraft(draft.id.clone()))
        {
            self.composer.discard_pending = true;
        }
    }

    pub(super) fn draft_deleted(&mut self, id: String, result: Result<Arc<DraftState>, String>) {
        let current = self
            .composer
            .discard
            .as_ref()
            .is_some_and(|draft| draft.id == id);
        let owned_dependency = current && self.composer.discard_pending;
        if current {
            self.composer.discard_pending = false;
        }
        match result {
            Ok(state) => {
                self.observe_drafts(&state);
                self.retire_draft(&id, None);
                if current {
                    self.composer.discard = None;
                    self.composer.discard_return = None;
                    if self.dialog == Some(Dialog::DiscardDraft) {
                        self.dialog = None;
                    }
                }
                self.notice("Draft discarded.", false);
            }
            Err(error) => {
                if owned_dependency {
                    self.pending_close = None;
                    self.composer.close = None;
                }
                self.notice(error, true);
            }
        }
    }

    pub(super) fn discard_draft_form(&self) -> Element<'_, Message> {
        let Some(draft) = &self.composer.discard else {
            return space().into();
        };
        column![
            text(if draft.subject.is_empty() {
                "Untitled draft"
            } else {
                &draft.subject
            })
            .size(16)
            .font(BOLD),
            text(if draft.attachments.is_empty() {
                "This permanently deletes the draft.".into()
            } else {
                format!(
                    "This permanently deletes the draft and its {} attached {}.",
                    draft.attachments.len(),
                    if draft.attachments.len() == 1 {
                        "file"
                    } else {
                        "files"
                    }
                )
            })
            .size(13),
            row![
                action("Keep draft", Message::Close),
                space().width(Length::Fill),
                button(
                    text(if self.composer.discard_pending {
                        "Discarding…"
                    } else {
                        "Discard draft"
                    })
                    .size(12)
                )
                .padding([12, 16])
                .style(destructive)
                .on_press_maybe(
                    (!self.composer.discard_pending).then_some(Message::ConfirmDiscardDraft)
                )
            ]
            .spacing(10)
            .align_y(Alignment::Center)
        ]
        .spacing(18)
        .into()
    }

    pub(super) fn draft_context_view(&self) -> Element<'_, Message> {
        let Some(menu) = &self.composer.context else {
            return space().into();
        };
        let mut items = column![].spacing(2);
        for (discard, name, glyph) in [
            (false, "Open draft", "compose"),
            (true, "Discard draft…", "trash"),
        ] {
            items = items.push(
                button(
                    row![icon(glyph, 18.), text(name).size(12)]
                        .spacing(10)
                        .align_y(Alignment::Center),
                )
                .width(Length::Fill)
                .padding([10, 12])
                .style(if menu.discard == discard {
                    selected
                } else {
                    ghost
                })
                .on_press(Message::DraftContextAction(discard)),
            );
        }
        let scale = self.preferences.interface_scale as f32 / 100.;
        container(widget::opaque(
            container(items).width(220).padding(6).style(card),
        ))
        .padding(iced::Padding {
            left: menu
                .position
                .x
                .clamp(8., (self.size.width / scale - 230.).max(8.)),
            top: menu
                .position
                .y
                .clamp(8., (self.size.height / scale - 100.).max(8.)),
            ..Default::default()
        })
        .into()
    }

    pub(super) fn inline_reader(&self) -> Element<'_, Message> {
        // Keep the editor's native widget ancestry stable as the original loads
        // or changes its rendered surface; background work must not steal focus.
        let mut content = column![self.compose_card()].spacing(16);
        if self.composer.current.draft.reply_context.is_some() {
            if self.conversation_visible()
                && self.conversation.page.total > crate::store::CONVERSATION_PAGE_SIZE
            {
                content = content.push(self.conversation_controls());
            } else {
                content = content.push(muted("Conversation").size(12));
            }
            if self.conversation.error.is_some() {
                content = content.push(action(
                    "Reload related messages",
                    Message::RetryConversation,
                ));
            }
            if self.conversation_visible() {
                content = content.push(self.conversation_cards());
            } else if let Some(detail) = &self.detail {
                content = content.push(self.reader_surface(
                    detail,
                    container(self.reader_body(detail, true)).padding(16).into(),
                ));
            } else {
                content = content.push(muted("Loading the original message…"));
            }
        }
        column![
            self.find_bar(),
            widget::keyed_column([(
                self.composer.current.ui_key,
                scrollable(container(content).padding(14))
                    .id("compose-reader")
                    .height(Length::Fill)
                    .into(),
            )])
            .height(Length::Fill)
            .width(Length::Fill)
        ]
        .height(Length::Fill)
        .into()
    }

    pub(super) fn compose_card(&self) -> Element<'_, Message> {
        let title = if self.composer.current.draft.reply_context.is_some() {
            "Reply"
        } else if self.composer.current.draft.forward.is_some() {
            "Forward"
        } else {
            "New message"
        };
        let session = &self.composer.current;
        let status = if session.save_error.is_some() {
            text("Not saved").font(BOLD)
        } else {
            muted(if session.pending.is_some() || session.dirty.is_some() {
                "Saving…"
            } else if session.saved_revision.is_some() {
                "Saved"
            } else {
                ""
            })
        };
        let header = row![
            text(title).size(16).font(BOLD),
            status.size(11),
            space().width(Length::Fill),
            self.icon_action(
                if self.composer.current.minimized {
                    "chevron"
                } else {
                    "down"
                },
                if self.composer.current.minimized {
                    "Expand draft"
                } else {
                    "Collapse draft"
                },
                Message::ToggleComposer
            ),
            self.icon_action("close", "Close draft", Message::CloseComposer),
        ]
        .align_y(Alignment::Center)
        .spacing(6);
        let mut content = column![header].spacing(12);
        if !self.composer.current.minimized {
            content = content.push(line()).push(self.compose_form());
        }
        container(content)
            .padding(16)
            .width(Length::Fill)
            .style(card)
            .into()
    }

    pub(super) fn compose_form(&self) -> Element<'_, Message> {
        let choices: Vec<_> = self
            .workspace
            .accounts
            .iter()
            .map(|a| Choice(a.id.clone(), a.email.clone()))
            .collect();
        let chosen = choices
            .iter()
            .find(|a| a.0 == self.compose_field("account"))
            .cloned();
        let mut form = column![
            row![
                muted("From").width(52),
                pick_list(choices, chosen, |c: Choice| Message::ComposeField(
                    "account", c.0
                ))
                .style(select_input)
                .menu_style(select_menu)
                .text_size(12)
                .padding(10)
                .width(Length::Fill)
            ]
            .align_y(Alignment::Center)
        ]
        .spacing(10);
        let address_row = |label: &'static str,
                           index: usize,
                           placeholder: &'static str|
         -> Element<'_, Message> {
            row![
                muted(label).width(52),
                self.recipient_field(index, placeholder)
            ]
            .align_y(Alignment::Center)
            .into()
        };
        form = form.push(
            row![
                address_row("To", 0, "name@example.com"),
                button(text("Cc / Bcc").size(11))
                    .padding([11, 10])
                    .style(ghost)
                    .on_press(Message::ShowRecipients)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
        if self.composer.current.show_recipients
            || !self.compose_field("cc").is_empty()
            || !self.compose_field("bcc").is_empty()
        {
            form = form
                .push(address_row("Cc", 1, "Copy recipients"))
                .push(address_row("Bcc", 2, "Hidden recipients"));
        }
        form = form.push(
            row![
                muted("Subject").width(52),
                input("Add a subject", self.compose_field("subject"), |v| {
                    Message::ComposeField("subject", v)
                })
                .id("subject")
            ]
            .align_y(Alignment::Center),
        );
        form = form.push(
            super::text_context::TextContext::editor(
                widget::text_editor(&self.composer.current.editor)
                    .id("compose-body")
                    .key_binding(|key| compose_binding(key, &self.preferences.shortcuts))
                    .on_action(Message::Editor)
                    .placeholder("Write your message…")
                    .style(editor_field)
                    .size(13)
                    .padding(15)
                    .height(
                        (self.size.height / (self.preferences.interface_scale as f32 / 100.)
                            * 0.32)
                            .clamp(
                                120.,
                                if self.composer.current.draft.reply_context.is_some() {
                                    200.
                                } else {
                                    300.
                                },
                            ),
                    ),
                &self.composer.current.editor,
                true,
                self.composer.current.draft.id.clone(),
            )
            .menu_theme(self.theme()),
        );
        if let Some(context) = &self.composer.current.draft.reply_context {
            form = form.push(
                widget::checkbox(context.include_quote)
                    .label("Include original message")
                    .size(15)
                    .text_size(11)
                    .on_toggle(Message::IncludeOriginal),
            );
        }
        if !self.composer.current.draft.attachments.is_empty() {
            let mut files = row![].spacing(6);
            for file in &self.composer.current.draft.attachments {
                files = files.push(
                    container(
                        row![
                            icon("clip", 16.),
                            text(truncate(&file.name, 28)).size(11),
                            muted(format!("{} KB", file.size.div_ceil(1024))).size(10),
                            button(icon("close", 14.))
                                .padding(8)
                                .style(ghost)
                                .on_press_maybe(
                                    (!self.compose_locked() && self.composer.io.is_none())
                                        .then(|| Message::RemoveDraftAttachment(file.id.clone()))
                                )
                        ]
                        .spacing(5)
                        .align_y(Alignment::Center),
                    )
                    .padding([1, 7])
                    .style(subtle),
                );
            }
            form = form
                .push(container(scrollable(files.wrap()).height(Length::Shrink)).max_height(70));
        }
        form = form.push(
            row![
                button(
                    text(if self.compose_locked() {
                        "Sending…"
                    } else {
                        "Send"
                    })
                    .size(12)
                    .line_height(1.)
                )
                .padding([12, 16])
                .style(primary)
                .on_press_maybe(
                    (!self.compose_locked()
                        && self.composer.io.as_deref() != Some(&self.composer.current.draft.id))
                    .then_some(Message::Send)
                ),
                button(
                    row![
                        icon("clip", 18.),
                        text(if self.composer.io.is_some() {
                            "Attaching…"
                        } else {
                            "Attach"
                        })
                        .size(12)
                        .line_height(1.)
                    ]
                    .spacing(7)
                    .align_y(Alignment::Center)
                )
                .padding([10, 12])
                .style(outline)
                .on_press_maybe(
                    (!self.compose_locked() && self.composer.io.is_none())
                        .then_some(Message::ChooseAttachments)
                ),
                self.icon_action(
                    "trash",
                    "Discard draft",
                    Message::ReviewDiscardDraft(self.composer.current.draft.id.clone())
                ),
                button(
                    text(if self.composer.current.save_error.is_some() {
                        "Retry save"
                    } else {
                        "Save draft"
                    })
                    .size(12)
                )
                .padding([12, 14])
                .style(ghost)
                .on_press_maybe((!self.compose_locked()).then_some(Message::SaveDraft)),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .wrap(),
        );
        let session = &self.composer.current;
        if let Some((_, error)) = &session.save_error {
            form = form.push(text(format!("Draft not saved. {error}")).size(11));
        } else if session.pending.is_some() || session.dirty.is_some() {
            form = form.push(muted("Saving draft…").size(11));
        } else if session.saved_revision.is_some() {
            form = form.push(muted("Draft saved on this device").size(11));
        }
        form.into()
    }

    pub(super) fn compose_field(&self, key: &str) -> &str {
        let draft = &self.composer.current.draft;
        match key {
            "account" => &draft.account_id,
            "to" => &draft.to,
            "cc" => &draft.cc,
            "bcc" => &draft.bcc,
            "subject" => &draft.subject,
            _ => "",
        }
    }

    /// Opens a new draft prefilled from a `mailto` link the desktop sent.
    pub(super) fn compose_mailto(&mut self, link: &str) -> Task<Message> {
        let Some(mailto) = crate::mailto::Mailto::parse(link) else {
            return Task::none();
        };
        self.load_draft(Draft {
            to: mailto.to,
            cc: mailto.cc,
            bcc: mailto.bcc,
            subject: mailto.subject,
            body: mailto.body,
            ..self.blank_draft()
        });
        self.focus_after_layout("to")
    }

    fn compose_editable(&self) -> bool {
        self.compose_visible() && self.dialog.is_none() && !self.compose_locked()
    }

    pub(super) fn edit_compose_field(&mut self, key: &str, value: String) {
        if !self.compose_editable() {
            return;
        }
        if let Some(index) = address_index(key) {
            return self.set_address(index, &value);
        }
        let session = &mut self.composer.current;
        let field = match key {
            "account" => &mut session.draft.account_id,
            "subject" => &mut session.draft.subject,
            _ => return,
        };
        if *field == value {
            return;
        }
        if key == "subject" {
            let before = std::mem::replace(field, value);
            let edit = Edit::between(&before, field);
            session
                .history
                .subject
                .record(edit, Instant::now(), || before);
        } else {
            *field = value;
        }
        self.draft_edited();
    }

    /// Replaces a whole recipient field, keeping any unfinished text as the entry.
    fn set_address(&mut self, index: usize, value: &str) {
        let session = &mut self.composer.current;
        if address_mut(&mut session.draft, index).as_str() == value {
            return;
        }
        let mut next = Recipients::default();
        next.input(value);
        let before = std::mem::replace(&mut session.addresses[index], next);
        session.history.addresses[index].record(Edit::Other, Instant::now(), || before);
        self.write_address(index);
    }

    pub(super) fn edit_recipient_entry(&mut self, key: &str, input: EntryInput) {
        let Some(index) = address_index(key) else {
            return;
        };
        if !self.compose_editable() {
            return;
        }
        let session = &mut self.composer.current;
        let widget = &mut session.entries[index];
        let shown = if widget.seen == Some(input.generation) {
            std::mem::take(&mut widget.value)
        } else {
            input.base
        };
        widget.seen = Some(input.generation);
        widget.generation += 1;
        widget.value.clone_from(&input.value);
        let recipients = &mut session.addresses[index];
        let mut next = recipients.clone();
        // The input reports an unchanged empty value for Backspace with
        // nothing left to delete: that reopens the last chip for editing.
        let edit = if shown.is_empty() && input.value.is_empty() {
            next.reopen_last();
            Edit::Other
        } else {
            next.input(&recipients::apply_input(
                &recipients.entry,
                &shown,
                &input.value,
            ));
            if next.chips.len() == recipients.chips.len() {
                Edit::between(&recipients.entry, &next.entry)
            } else {
                Edit::Other
            }
        };
        if next == *recipients {
            return;
        }
        let before = std::mem::replace(recipients, next);
        session.history.addresses[index].record(edit, Instant::now(), || before);
        self.write_address(index);
    }

    /// Copies a recipient field's chips back into the draft value.
    fn write_address(&mut self, index: usize) {
        let session = &mut self.composer.current;
        // The next view gives the input new text, so later edits start there.
        session.entries[index].generation += 1;
        let value = session.addresses[index].value();
        let field = address_mut(&mut session.draft, index);
        if *field != value {
            *field = value;
            self.draft_edited();
        }
    }

    pub(super) fn remove_recipient(&mut self, key: &str, position: usize, text: &str) {
        let Some(index) = address_index(key) else {
            return;
        };
        if !self.compose_editable() {
            return;
        }
        let session = &mut self.composer.current;
        let before = session.addresses[index].clone();
        if !session.addresses[index].remove(position, text) {
            return;
        }
        session.history.addresses[index].record(Edit::Other, Instant::now(), || before);
        self.write_address(index);
    }

    pub(super) fn commit_recipients(&mut self, key: &str) {
        if let Some(index) = address_index(key)
            && self.compose_editable()
        {
            self.commit_address(index);
        }
    }

    /// Turns unfinished recipient text into chips once focus leaves its field.
    pub(super) fn commit_recipients_except(&mut self, focused: Option<&str>) {
        if !self.compose_editable() {
            return;
        }
        for (index, key) in ADDRESS_FIELDS.into_iter().enumerate() {
            if focused != Some(key) {
                self.commit_address(index);
            }
        }
    }

    fn commit_address(&mut self, index: usize) {
        let session = &mut self.composer.current;
        if session.addresses[index].commit() {
            // Undo after a commit returns to the typed text, not the chip.
            session.history.addresses[index].seal();
            self.write_address(index);
        }
    }

    pub(super) fn compose_history(&mut self, field: &str, step: Step) -> Task<Message> {
        if !self.compose_editable() {
            return Task::none();
        }
        let session = &mut self.composer.current;
        if field == "compose-body" {
            let editor = &session.editor;
            let Some(state) = step.apply(&mut session.history.body, || BodyState::capture(editor))
            else {
                return Task::none();
            };
            session.editor = state.restore();
        } else if field == "subject" {
            let subject = &session.draft.subject;
            let Some(state) = step.apply(&mut session.history.subject, || subject.clone()) else {
                return Task::none();
            };
            session.draft.subject = state;
        } else if let Some(index) = address_index(field) {
            let recipients = &session.addresses[index];
            let Some(state) =
                step.apply(&mut session.history.addresses[index], || recipients.clone())
            else {
                return Task::none();
            };
            session.addresses[index] = state;
            session.entries[index].generation += 1;
            *address_mut(&mut session.draft, index) = session.addresses[index].value();
        } else {
            return Task::none();
        }
        self.draft_edited();
        Task::none()
    }

    /// Saves the body before an edit so Ctrl+Z can return to it.
    pub(super) fn record_body_edit(&mut self, action: &text_editor::Action) {
        let session = &mut self.composer.current;
        let text_editor::Action::Edit(edit) = action else {
            // Moving the cursor ends the current typing step.
            if !matches!(action, text_editor::Action::Scroll { .. }) {
                session.history.body.seal();
            }
            return;
        };
        let kind = match edit {
            _ if session.editor.cursor().selection.is_some() => Edit::Other,
            text_editor::Edit::Insert(c) => Edit::Insert {
                space: c.is_whitespace(),
            },
            text_editor::Edit::Backspace | text_editor::Edit::Delete => Edit::Delete,
            _ => Edit::Other,
        };
        let editor = &session.editor;
        session
            .history
            .body
            .record(kind, Instant::now(), || BodyState::capture(editor));
    }

    fn recipient_field(&self, index: usize, placeholder: &'static str) -> Element<'_, Message> {
        let key = ADDRESS_FIELDS[index];
        let recipients = &self.composer.current.addresses[index];
        let generation = self.composer.current.entries[index].generation;
        let editable = !self.compose_locked();
        let entry = input(
            if recipients.chips.is_empty() {
                placeholder
            } else {
                ""
            },
            &recipients.entry,
            move |value| {
                Message::RecipientEntry(
                    key,
                    EntryInput {
                        generation,
                        base: recipients.entry.clone(),
                        value,
                    },
                )
            },
        )
        .id(key)
        .on_submit(Message::CommitRecipients(key))
        .map_paste(recipients::paste_lines)
        .padding([7, 6])
        .style(bare_field);
        let chips = recipients
            .chips
            .iter()
            .enumerate()
            .map(|(position, chip)| self.recipient_chip(key, position, chip, editable));
        let field = recipient_box::RecipientBox::new(entry, recipients.entry.is_empty(), chips);
        #[cfg(feature = "test-support")]
        let field = field.observed(key, self.draw_log.clone());
        field.into()
    }

    fn recipient_chip<'a>(
        &self,
        key: &'static str,
        position: usize,
        chip: &'a recipients::Chip,
        editable: bool,
    ) -> Element<'a, Message> {
        let remove = button(icon("close", 12.))
            .padding(4)
            .style(ghost)
            .on_press_maybe(
                editable.then(|| Message::RemoveRecipient(key, position, chip.text.clone())),
            );
        let remove: Element<'a, Message> = if self.preferences.tooltips {
            widget::tooltip(
                remove,
                container(text(format!("Remove {}", truncate(&chip.text, 48))).size(12))
                    .padding(8)
                    .style(card),
                widget::tooltip::Position::Bottom,
            )
            .gap(5)
            .into()
        } else {
            remove.into()
        };
        // Long addresses are cut with an ellipsis, keeping room for the x.
        let address = || super::ellipsis::Ellipsis::new(chip.text.clone(), 12.).shrink(24.);
        let label: Element<'a, Message> = if chip.valid {
            address().into()
        } else {
            // Send would reject this address; mark it with more than colour.
            row![
                text("!")
                    .size(12)
                    .font(BOLD)
                    .style(|theme: &Theme| text::Style {
                        color: Some(colors(theme).flag)
                    }),
                address()
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .into()
        };
        container(row![label, remove].spacing(2).align_y(Alignment::Center))
            .padding(iced::Padding {
                left: 8.,
                right: 1.,
                top: 1.,
                bottom: 1.,
            })
            .style(if chip.valid {
                recipient_chip
            } else {
                invalid_recipient_chip
            })
            .into()
    }

    pub(super) fn recipients_observation(&self) -> serde_json::Value {
        let session = &self.composer.current;
        ADDRESS_FIELDS
            .iter()
            .zip(&session.addresses)
            .map(|(key, recipients)| {
                (
                    key.to_string(),
                    serde_json::json!({
                        "chips": recipients.chips.iter().map(|chip| &chip.text).collect::<Vec<_>>(),
                        "invalid": recipients.chips.iter().filter(|chip| !chip.valid).map(|chip| &chip.text).collect::<Vec<_>>(),
                        "entry": recipients.entry,
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>()
            .into()
    }

    pub(super) fn current_draft(&self) -> Draft {
        Draft {
            body: self.composer.current.editor.text(),
            ..self.composer.current.draft.clone()
        }
    }

    pub(super) fn compose_locked(&self) -> bool {
        self.busy
            .contains(&format!("send:{}", self.composer.current.draft.id))
    }

    pub(super) fn draft_edited(&mut self) {
        self.composer.current.draft.revision += 1;
        self.composer.current.dirty = Some(Instant::now());
    }

    pub(super) fn choose_attachments(&mut self) -> Task<Message> {
        if self.compose_locked() || self.composer.io.is_some() {
            return Task::none();
        }
        let draft = self.current_draft();
        self.composer.io = Some(draft.id.clone());
        self.composer.picker = Some(draft.id.clone());
        Task::perform(
            async move {
                let files = rfd::AsyncFileDialog::new()
                    .set_title("Attach files")
                    .pick_files()
                    .await
                    .unwrap_or_default();
                (
                    draft,
                    files
                        .into_iter()
                        .map(|file| file.path().to_owned())
                        .collect(),
                )
            },
            |(draft, files)| Message::ChosenAttachments(draft, files),
        )
    }

    pub(super) fn attach_chosen(&mut self, captured: Draft, files: Vec<std::path::PathBuf>) {
        if self.composer.picker.as_deref() == Some(&captured.id) {
            self.composer.picker = None;
        }
        if files.is_empty() {
            self.composer.io = None;
            return;
        }
        let draft = if let Some(session) = self.composer.session_mut(&captured.id) {
            session.draft.revision += 1;
            session.dirty = Some(Instant::now());
            session.snapshot()
        } else {
            captured
        };
        if !self.try_command(Command::AddDraftFiles(draft, files)) {
            self.composer.io = None;
        }
    }

    pub(super) fn remove_draft_attachment(&mut self, id: String) {
        if self.compose_locked() || self.composer.io.is_some() {
            return;
        }
        if self.try_command(Command::RemoveDraftFile(
            self.composer.current.draft.id.clone(),
            id,
        )) {
            self.composer.io = Some(self.composer.current.draft.id.clone());
            self.draft_edited();
        }
    }
}
