use super::views::{Choice, truncate};
use super::*;
use crate::store::DraftState;
#[cfg(test)]
#[path = "composing_tests.rs"]
mod tests;
use iced::{
    Alignment, Length,
    widget::{button, column, container, pick_list, row, scrollable, space, text},
};

#[derive(Debug, Clone, Copy)]
pub(super) enum Exit {
    Dialog,
    Tab(Tab),
    Window(iced::window::Id),
}

#[derive(Default)]
pub(super) struct Session {
    pub draft: Draft,
    pub editor: text_editor::Content,
    pub dirty: Option<Instant>,
    pub pending: Option<u64>,
    pub show_recipients: bool,
}
#[derive(Default)]
pub(super) struct Composer {
    pub current: Session,
    pub io: Option<String>,
    saving: Option<(String, u64, Exit)>,
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
                self.composer.forward_error = Some(error.clone());
                self.notice(error, true);
            }
        }
        Task::none()
    }

    pub(super) fn review_discard_draft(&mut self, id: String) {
        if self.compose_locked() || self.composer.discard_pending {
            return;
        }
        if self.workspace.outgoing_drafts.contains(&id) {
            self.notice(
                "Review this message in Outbox before discarding its draft.",
                true,
            );
            return;
        }
        let draft = if self.dialog == Some(Dialog::Compose) && self.composer.current.draft.id == id
        {
            Some(self.current_draft())
        } else {
            self.workspace
                .drafts
                .iter()
                .find(|draft| draft.id == id)
                .cloned()
        };
        if let Some(draft) = draft {
            self.composer.discard = Some(draft);
            self.composer.discard_return = self.dialog;
            self.composer.saving = None;
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
        if current {
            self.composer.discard_pending = false;
        }
        match result {
            Ok(state) => {
                self.observe_drafts(&state);
                if current {
                    self.composer.discard = None;
                    self.composer.discard_return = None;
                    if self.dialog == Some(Dialog::DiscardDraft) {
                        self.dialog = None;
                        if self.composer.current.draft.id == id {
                            self.composer.current = Session::default();
                        }
                    }
                }
                self.notice("Draft discarded.", false);
            }
            Err(error) => self.notice(error, true),
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
        let extra = if self.composer.current.show_recipients {
            96.
        } else {
            0.
        } + if self.composer.current.draft.attachments.is_empty() {
            0.
        } else {
            100.
        };
        form = form.push(
            widget::text_editor(&self.composer.current.editor)
                .on_action(Message::Editor)
                .placeholder("Write your message…")
                .style(editor_field)
                .size(13)
                .padding(15)
                .height((self.size.height - 410. - extra).clamp(80., 300.)),
        );
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
                        "Send message"
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
                            "Attach files"
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
                space().width(Length::Fill),
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
            .spacing(8)
            .align_y(Alignment::Center),
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
        if self.dialog != Some(Dialog::Compose) || self.compose_locked() {
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
        self.dialog == Some(Dialog::Compose)
            && self
                .busy
                .contains(&format!("send:{}", self.composer.current.draft.id))
    }

    pub(super) fn draft_edited(&mut self) {
        self.composer.current.draft.revision += 1;
        self.composer.current.dirty = Some(Instant::now());
    }

    pub(super) fn load_draft(&mut self, draft: Draft) {
        self.open(Dialog::Compose);
        self.composer.current.editor = text_editor::Content::with_text(&draft.body);
        self.composer.current.show_recipients = !draft.cc.is_empty() || !draft.bcc.is_empty();
        self.composer.current.draft = draft;
    }

    pub(super) fn observe_drafts(&mut self, state: &DraftState) {
        if state.revision < self.workspace.drafts_revision {
            return;
        }
        let workspace = Arc::make_mut(&mut self.workspace);
        workspace.drafts = state.drafts.clone();
        workspace.drafts_revision = state.revision;
        self.observe_draft_files();
    }

    pub(super) fn observe_draft_files(&mut self) {
        if let Some(draft) = self
            .workspace
            .drafts
            .iter()
            .find(|d| d.id == self.composer.current.draft.id)
        {
            // Body/recipient edits belong to the open editor. Only file metadata
            // comes from background updates, including late autosave snapshots.
            self.composer.current.draft.attachments = draft.attachments.clone();
        }
    }

    pub(super) fn defer_draft_exit(&mut self, exit: Exit) -> bool {
        if self.compose_locked() {
            return false;
        }
        if self.dialog != Some(Dialog::Compose)
            && (!matches!(exit, Exit::Window(_))
                || (self.composer.current.dirty.is_none()
                    && self.composer.current.pending.is_none()))
        {
            return false;
        }
        let draft = self.current_draft();
        if draft.to.is_empty()
            && draft.cc.is_empty()
            && draft.bcc.is_empty()
            && draft.subject.is_empty()
            && draft.body.trim().is_empty()
            && draft.attachments.is_empty()
            && self.composer.io.as_deref() != Some(&self.composer.current.draft.id)
            && self.composer.current.dirty.is_none()
            && self.composer.current.pending.is_none()
        {
            return false;
        }
        self.save_and_exit(exit);
        true
    }

    pub(super) fn autosave_draft(&mut self) {
        if self.composer.current.pending.is_some()
            || self.composer.discard_pending
            || self
                .composer
                .current
                .dirty
                .is_none_or(|t| t.elapsed().as_secs() < 1)
        {
            return;
        }
        let draft = self.current_draft();
        let revision = draft.revision;
        if self.try_command(Command::AutoSaveDraft(draft)) {
            self.composer.current.pending = Some(revision);
            self.composer.current.dirty = None;
        }
    }

    pub(super) fn save_and_exit(&mut self, exit: Exit) {
        if self.compose_locked() {
            return;
        }
        if let Some(revision) = self.composer.current.pending {
            self.composer.saving = Some((self.composer.current.draft.id.clone(), revision, exit));
            return;
        }
        let draft = self.current_draft();
        let request = (draft.id.clone(), draft.revision, exit);
        if self.try_command(Command::SaveDraft(draft)) {
            self.composer.current.pending = Some(request.1);
            self.composer.saving = Some(request);
            self.composer.current.dirty = None;
        }
    }

    pub(super) fn draft_saved(
        &mut self,
        id: String,
        revision: u64,
        result: Result<Arc<DraftState>, String>,
    ) -> Task<Message> {
        let current = id == self.composer.current.draft.id;
        let pending = current && self.composer.current.pending == Some(revision);
        if pending {
            self.composer.current.pending = None;
        }
        match result {
            Err(error) => {
                // A delayed failure cannot cancel a newer save/close request.
                if current && (pending || self.composer.current.draft.revision == revision) {
                    if self
                        .composer
                        .saving
                        .as_ref()
                        .is_some_and(|(request, version, _)| request == &id && *version == revision)
                    {
                        self.composer.saving = None;
                    }
                    self.composer.current.dirty = Some(Instant::now());
                }
                self.notice(error, true);
            }
            Ok(state) => {
                self.observe_drafts(&state);
                if let Some((request, version, exit)) = self.composer.saving.clone()
                    && request == id
                    && version == revision
                {
                    self.composer.saving = None;
                    if current
                        && (self.dialog == Some(Dialog::Compose) || matches!(exit, Exit::Window(_)))
                    {
                        if self.composer.current.draft.revision != revision {
                            self.save_and_exit(exit);
                        } else {
                            if self.dialog == Some(Dialog::Compose) {
                                self.dialog = None;
                            }
                            self.composer.current.dirty = None;
                            return match exit {
                                Exit::Dialog => widget::operation::focus("unfocused"),
                                Exit::Tab(tab) => self.handle(Message::Tab(tab)),
                                Exit::Window(window) => self.handle(Message::WindowClose(window)),
                            };
                        }
                    }
                }
            }
        }
        Task::none()
    }

    pub(super) fn choose_attachments(&mut self) -> Task<Message> {
        if self.compose_locked() || self.composer.io.is_some() {
            return Task::none();
        }
        let draft = self.current_draft();
        self.composer.io = Some(draft.id.clone());
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
        if files.is_empty() {
            self.composer.io = None;
            return;
        }
        let draft = if captured.id == self.composer.current.draft.id {
            self.draft_edited();
            self.current_draft()
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
