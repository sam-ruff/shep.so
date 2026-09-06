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
pub(super) struct Composer {
    pub draft: Draft,
    pub show_recipients: bool,
    pub io: Option<String>,
    saving: Option<(String, u64, Exit)>,
}

impl App {
    pub(super) fn compose_form(&self) -> Element<'_, Message> {
        let choices: Vec<_> = self
            .workspace
            .accounts
            .iter()
            .map(|a| Choice(a.id.clone(), a.email.clone()))
            .collect();
        let chosen = choices
            .iter()
            .find(|a| a.0 == self.field("account"))
            .cloned();
        let mut form = column![
            row![
                muted("From").width(52),
                pick_list(choices, chosen, |c: Choice| Message::Field("account", c.0))
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
                input(placeholder, self.field(key), move |v| Message::Field(
                    key, v
                ))
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
        if self.composer.show_recipients
            || !self.field("cc").is_empty()
            || !self.field("bcc").is_empty()
        {
            form = form
                .push(address_row("Cc", "cc", "Copy recipients"))
                .push(address_row("Bcc", "bcc", "Hidden recipients"));
        }
        form = form.push(address_row("Subject", "subject", "Add a subject"));
        let extra = if self.composer.show_recipients {
            96.
        } else {
            0.
        } + if self.composer.draft.attachments.is_empty() {
            0.
        } else {
            100.
        };
        form = form.push(
            widget::text_editor(&self.editor)
                .on_action(Message::Editor)
                .placeholder("Write your message…")
                .style(editor_field)
                .size(13)
                .padding(15)
                .height((self.size.height - 410. - extra).clamp(80., 300.)),
        );
        if !self.composer.draft.attachments.is_empty() {
            let mut files = row![].spacing(6);
            for file in &self.composer.draft.attachments {
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
                    (!self.compose_locked() && self.composer.io.as_deref() != Some(&self.draft_id))
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

    pub(super) fn current_draft(&self) -> Draft {
        Draft {
            id: self.draft_id.clone(),
            account_id: self.field("account").into(),
            to: self.field("to").into(),
            cc: self.field("cc").into(),
            bcc: self.field("bcc").into(),
            subject: self.field("subject").into(),
            body: self.editor.text(),
            ..self.composer.draft.clone()
        }
    }

    pub(super) fn compose_locked(&self) -> bool {
        self.dialog == Some(Dialog::Compose)
            && self.busy.contains(&format!("send:{}", self.draft_id))
    }

    pub(super) fn draft_edited(&mut self) {
        self.composer.draft.revision += 1;
        self.draft_dirty = Some(Instant::now());
    }

    pub(super) fn load_draft(&mut self, draft: Draft) {
        self.open(Dialog::Compose);
        self.draft_id = draft.id.clone();
        self.fields.insert("account", draft.account_id.clone());
        self.fields.insert("to", draft.to.clone());
        self.fields.insert("cc", draft.cc.clone());
        self.fields.insert("bcc", draft.bcc.clone());
        self.fields.insert("subject", draft.subject.clone());
        self.editor = text_editor::Content::with_text(&draft.body);
        self.composer.show_recipients = !draft.cc.is_empty() || !draft.bcc.is_empty();
        self.composer.draft = draft;
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
        if let Some(draft) = self.workspace.drafts.iter().find(|d| d.id == self.draft_id) {
            // Body/recipient edits belong to the open editor. Only file metadata
            // comes from background updates, including late autosave snapshots.
            self.composer.draft.attachments = draft.attachments.clone();
        }
    }

    pub(super) fn defer_draft_exit(&mut self, exit: Exit) -> bool {
        if self.dialog != Some(Dialog::Compose) || self.compose_locked() {
            return false;
        }
        let draft = self.current_draft();
        if draft.to.is_empty()
            && draft.cc.is_empty()
            && draft.bcc.is_empty()
            && draft.subject.is_empty()
            && draft.body.trim().is_empty()
            && draft.attachments.is_empty()
            && self.composer.io.as_deref() != Some(&self.draft_id)
        {
            return false;
        }
        self.save_and_exit(exit);
        true
    }

    pub(super) fn save_and_exit(&mut self, exit: Exit) {
        if self.compose_locked() {
            return;
        }
        let draft = self.current_draft();
        let request = (draft.id.clone(), draft.revision, exit);
        if self.try_command(Command::SaveDraft(draft)) {
            self.composer.saving = Some(request);
            self.draft_dirty = None;
        }
    }

    pub(super) fn draft_saved(
        &mut self,
        id: String,
        revision: u64,
        result: Result<Arc<DraftState>, String>,
    ) -> Task<Message> {
        match result {
            Err(error) => {
                if id == self.draft_id {
                    self.composer.saving = None;
                    self.draft_dirty = Some(Instant::now());
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
                    if self.dialog == Some(Dialog::Compose) && self.draft_id == id {
                        if self.composer.draft.revision != revision {
                            self.save_and_exit(exit);
                        } else {
                            self.dialog = None;
                            self.draft_dirty = None;
                            return match exit {
                                Exit::Dialog => widget::operation::focus("unfocused"),
                                Exit::Tab(tab) => self.handle(Message::Tab(tab)),
                                Exit::Window(window) => iced::window::close(window),
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
        let draft = if self.dialog == Some(Dialog::Compose) && captured.id == self.draft_id {
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
        if self.try_command(Command::RemoveDraftFile(self.draft_id.clone(), id)) {
            self.composer.io = Some(self.draft_id.clone());
            self.draft_edited();
        }
    }
}
