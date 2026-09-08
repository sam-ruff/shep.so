use super::*;

impl Session {
    pub(in crate::ui) fn snapshot(&self) -> Draft {
        Draft {
            body: self.editor.text(),
            ..self.draft.clone()
        }
    }

    fn meaningful(&self) -> bool {
        !self.draft.id.is_empty()
            && (!self.draft.to.is_empty()
                || !self.draft.cc.is_empty()
                || !self.draft.bcc.is_empty()
                || !self.draft.subject.is_empty()
                || !self.editor.text().trim().is_empty()
                || !self.draft.attachments.is_empty()
                || self.dirty.is_some()
                || self.pending.is_some())
    }
}

impl Composer {
    pub(in crate::ui) fn session_mut(&mut self, id: &str) -> Option<&mut Session> {
        if self.current.draft.id == id {
            Some(&mut self.current)
        } else {
            self.parked.get_mut(id)
        }
    }
    pub(in crate::ui) fn pending(&self) -> bool {
        std::iter::once(&self.current)
            .chain(self.parked.values())
            .any(|session| session.dirty.is_some() || session.pending.is_some())
    }
}

impl App {
    pub(in crate::ui) fn compose_visible(&self) -> bool {
        self.tab == Tab::Mail && !self.composer.current.draft.id.is_empty()
    }

    pub(in crate::ui) fn owned_draft(&self, id: &str) -> Option<Draft> {
        if self.composer.current.draft.id == id {
            Some(self.current_draft())
        } else if let Some(session) = self.composer.parked.get(id) {
            Some(session.snapshot())
        } else {
            self.workspace
                .drafts
                .iter()
                .find(|draft| draft.id == id)
                .cloned()
        }
    }

    pub(in crate::ui) fn draft_labels(&self) -> Vec<(&str, &str)> {
        let mut drafts: Vec<_> = self
            .workspace
            .drafts
            .iter()
            .map(|d| (d.id.as_str(), d.subject.as_str()))
            .collect();
        for session in std::iter::once(&self.composer.current).chain(self.composer.parked.values())
        {
            if session.draft.id.is_empty() {
                continue;
            }
            let label = (session.draft.id.as_str(), session.draft.subject.as_str());
            if let Some(entry) = drafts.iter_mut().find(|entry| entry.0 == label.0) {
                *entry = label;
            } else {
                drafts.push(label);
            }
        }
        drafts.retain(|(id, _)| !self.workspace.outgoing_drafts.contains(*id));
        drafts.sort_by(|a, b| a.1.cmp(b.1).then_with(|| a.0.cmp(b.0)));
        drafts
    }

    pub(in crate::ui) fn park_composer(&mut self) {
        let session = std::mem::take(&mut self.composer.current);
        if session.meaningful()
            || self.composer.io.as_deref() == Some(&session.draft.id)
            || self.busy.contains(&format!("send:{}", session.draft.id))
        {
            self.composer
                .parked
                .insert(session.draft.id.clone(), session);
        }
        self.flush_draft_saves(true);
    }

    pub(in crate::ui) fn close_composer(&mut self) {
        self.composer.dismissed_for = self.selected.clone();
        self.composer.resume = None;
        self.park_composer();
    }

    pub(in crate::ui) fn load_draft(&mut self, draft: Draft) {
        self.composer.close = None;
        self.pending_close = None;
        if self.composer.current.draft.id == draft.id {
            self.composer.current.minimized = false;
            self.tab = Tab::Mail;
            self.dialog = None;
            return;
        }
        self.park_composer();
        if let Some(context) = &draft.reply_context {
            let source = self
                .detail
                .as_ref()
                .filter(|detail| {
                    detail.summary.account_id == context.account_id
                        && draft.in_reply_to.is_some()
                        && draft.in_reply_to == detail.reply.message_id
                })
                .map(|detail| detail.summary.id.clone())
                .unwrap_or_else(|| context.mail_id.clone());
            if self.reader_id() != Some(source.as_str()) {
                self.selected = Some(source.clone());
                self.conversation.page = Default::default();
                self.focus_conversation_message(source);
                self.request_conversation(None);
            }
        }
        self.composer.next_ui_key = self.composer.next_ui_key.wrapping_add(1);
        let ui_key = self.composer.next_ui_key;
        self.composer.current = self.composer.parked.remove(&draft.id).unwrap_or_else(|| {
            let saved = self
                .workspace
                .drafts
                .iter()
                .any(|saved| saved.id == draft.id && saved.revision >= draft.revision);
            Session {
                ui_key,
                editor: text_editor::Content::with_text(&draft.body),
                show_recipients: !draft.cc.is_empty() || !draft.bcc.is_empty(),
                dirty: (!saved).then(Instant::now),
                draft,
                ..Default::default()
            }
        });
        self.composer.current.minimized = false;
        self.composer.dismissed_for = None;
        self.pending_focus = None;
        self.focused_input = None;
        self.sidebar_focus = false;
        self.list_focus = false;
        self.clear_mail_selection();
        self.tab = Tab::Mail;
        self.dialog = None;
    }

    pub(in crate::ui) fn new_composer(&mut self) {
        let draft = Draft {
            id: uuid::Uuid::new_v4().to_string(),
            account_id: self
                .query
                .account
                .clone()
                .or_else(|| self.workspace.accounts.first().map(|a| a.id.clone()))
                .unwrap_or_default(),
            ..Default::default()
        };
        self.load_draft(draft);
        // A completely empty composer need not become a saved draft.
        self.composer.current.dirty = None;
    }

    pub(in crate::ui) fn restore_reply(&mut self) {
        if !self.composer.current.draft.id.is_empty()
            || self.mail_selection.mode
            || self.tab != Tab::Mail
            || self.dialog.is_some()
            || self.pending_close.is_some()
            || self.composer.close.is_some()
            || self.composer.dismissed_for == self.selected
        {
            return;
        }
        let matches = |draft: &Draft| {
            let Some(context) = &draft.reply_context else {
                return false;
            };
            let id_match = self.selected.as_deref() == Some(context.mail_id.as_str())
                || self.reader_id() == Some(context.mail_id.as_str())
                || self
                    .conversation
                    .page
                    .rows
                    .iter()
                    .any(|mail| mail.id == context.mail_id);
            let header_match = self.detail.as_ref().is_some_and(|detail| {
                detail.summary.account_id == context.account_id
                    && draft.in_reply_to.is_some()
                    && draft.in_reply_to == detail.reply.message_id
            });
            (id_match || header_match)
                && !self.workspace.outgoing_drafts.contains(&draft.id)
                && !self.busy.contains(&format!("send:{}", draft.id))
        };
        let id = self
            .composer
            .parked
            .values()
            .filter(|session| matches(&session.draft))
            .max_by_key(|session| session.ui_key)
            .map(|session| session.draft.id.clone())
            .or_else(|| {
                self.workspace
                    .drafts
                    .iter()
                    .find(|draft| matches(draft))
                    .map(|draft| draft.id.clone())
            });
        if let Some(draft) = id.and_then(|id| self.owned_draft(&id)) {
            self.load_draft(draft);
        }
    }

    pub(in crate::ui) fn observe_drafts(&mut self, state: &DraftState) {
        if state.revision < self.workspace.drafts_revision {
            return;
        }
        let workspace = Arc::make_mut(&mut self.workspace);
        workspace.drafts = state.drafts.clone();
        workspace.drafts_revision = state.revision;
        self.observe_draft_files();
    }

    pub(in crate::ui) fn observe_draft_files(&mut self) {
        for session in
            std::iter::once(&mut self.composer.current).chain(self.composer.parked.values_mut())
        {
            if let Some(draft) = self
                .workspace
                .drafts
                .iter()
                .find(|d| d.id == session.draft.id)
            {
                session.draft.attachments = draft.attachments.clone();
            }
        }
    }

    pub(in crate::ui) fn retire_draft(&mut self, id: &str, revision: Option<u64>) {
        if let Some(session) = self.composer.session_mut(id)
            && revision.is_none_or(|revision| revision == session.draft.revision)
        {
            if self.composer.current.draft.id == id {
                self.composer.current = Session::default();
            }
            self.composer.parked.remove(id);
            if self.composer.resume.as_deref() == Some(id) {
                self.composer.resume = None;
            }
        }
    }

    pub(in crate::ui) fn flush_draft_saves(&mut self, force: bool) {
        if self.composer.discard_pending {
            return;
        }
        // Prepare only a small number of snapshots per UI update. Pending saves
        // coalesce edits by identity; close is resumed by acknowledgments/Tick.
        let ready: Vec<_> = std::iter::once(&self.composer.current)
            .chain(self.composer.parked.values())
            .filter(|session| {
                session.pending.is_none()
                    && session
                        .dirty
                        .is_some_and(|at| force || at.elapsed().as_secs() >= 1)
                    && !self.busy.contains(&format!("send:{}", session.draft.id))
            })
            .take(4)
            .map(Session::snapshot)
            .collect();
        for draft in ready {
            let id = draft.id.clone();
            let revision = draft.revision;
            let explicit = self
                .composer
                .session_mut(&id)
                .is_some_and(|session| session.explicit_save);
            let command = if explicit {
                Command::SaveDraft(draft)
            } else {
                Command::AutoSaveDraft(draft)
            };
            if self.try_command(command) {
                if let Some(session) = self.composer.session_mut(&id) {
                    session.explicit_save = false;
                    session.pending = Some(revision);
                    session.dirty = None;
                }
            } else {
                break;
            }
        }
    }

    pub(in crate::ui) fn autosave_draft(&mut self) {
        self.flush_draft_saves(self.composer.close.is_some());
    }

    pub(in crate::ui) fn defer_draft_exit(&mut self, exit: Exit) -> bool {
        match exit {
            Exit::Window(window) => {
                if self.composer.pending() {
                    self.composer.close = Some(window);
                    self.flush_draft_saves(true);
                    true
                } else {
                    self.composer.close = None;
                    false
                }
            }
            Exit::Tab(tab) => {
                if self.tab == Tab::Mail && self.tab != tab {
                    self.composer.resume = (!self.composer.current.draft.id.is_empty())
                        .then(|| self.composer.current.draft.id.clone());
                    self.park_composer();
                }
                false
            }
        }
    }

    pub(in crate::ui) fn save_current_draft(&mut self) {
        if self.compose_locked() || self.composer.current.draft.id.is_empty() {
            return;
        }
        self.composer.current.dirty = Some(Instant::now());
        self.composer.current.explicit_save = true;
        self.flush_draft_saves(true);
    }

    pub(in crate::ui) fn draft_saved(
        &mut self,
        id: String,
        revision: u64,
        result: Result<Arc<DraftState>, String>,
    ) -> Task<Message> {
        let mut owned_error = false;
        if let Some(session) = self.composer.session_mut(&id) {
            let pending = session.pending == Some(revision);
            if pending {
                session.pending = None;
            }
            if result.is_err() && (pending || session.draft.revision == revision) {
                session.dirty = Some(Instant::now());
                owned_error = true;
            }
        }
        match result {
            Ok(state) => self.observe_drafts(&state),
            Err(error) => {
                if owned_error {
                    self.composer.close = None;
                    self.fail_removal_draft_wait(&id, &error);
                }
                self.notice(error, true);
                return Task::none();
            }
        }
        self.continue_removal_review();
        if let Some(window) = self.composer.close {
            self.flush_draft_saves(true);
            if !self.composer.pending() {
                self.composer.close = None;
                return self.handle(Message::WindowClose(window));
            }
        }
        Task::none()
    }
}
