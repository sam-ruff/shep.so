use super::views::{Choice, truncate};
use super::*;
use crate::store::DraftState;
mod sessions;
#[cfg(test)]
#[path = "composing_tests.rs"]
mod tests;
use iced::{
    Alignment, Length,
    widget::{button, column, container, pick_list, row, scrollable, space, text},
};

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
    pub explicit_save: bool,
    pub show_recipients: bool,
    pub minimized: bool,
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
                    return focus_after_layout("to");
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
        let header = row![
            text(title).size(16).font(BOLD),
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
                           key: &'static str,
                           placeholder: &'static str|
         -> Element<'_, Message> {
            row![
                muted(label).width(52),
                input(placeholder, self.compose_field(key), move |v| {
                    Message::ComposeField(key, v)
                })
                .id(key)
            ]
            .align_y(Alignment::Center)
            .into()
        };
        form = form.push(
            row![
                address_row("To", "to", "name@example.com"),
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
                .push(address_row("Cc", "cc", "Copy recipients"))
                .push(address_row("Bcc", "bcc", "Hidden recipients"));
        }
        form = form.push(address_row("Subject", "subject", "Add a subject"));
        form = form.push(
            super::text_context::TextContext::editor(
                widget::text_editor(&self.composer.current.editor)
                    .id("compose-body")
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
                button(text("Save draft").size(12))
                    .padding([12, 14])
                    .style(ghost)
                    .on_press_maybe((!self.compose_locked()).then_some(Message::SaveDraft)),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .wrap(),
        );
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

    pub(super) fn edit_compose_field(&mut self, key: &str, value: String) {
        if !self.compose_visible() || self.dialog.is_some() || self.compose_locked() {
            return;
        }
        let draft = &mut self.composer.current.draft;
        let field = match key {
            "account" => &mut draft.account_id,
            "to" => &mut draft.to,
            "cc" => &mut draft.cc,
            "bcc" => &mut draft.bcc,
            "subject" => &mut draft.subject,
            _ => return,
        };
        if *field != value {
            *field = value;
            self.draft_edited();
        }
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
