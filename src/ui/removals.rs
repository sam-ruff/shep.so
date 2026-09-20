use super::*;
use crate::store::{ConnectionKind, ConnectionRef, RemovalJob, RemovalPreview, RemovalStage};
use iced::{
    Alignment, Length,
    widget::{button, checkbox, column, container, row, space, text},
};

#[derive(Default)]
pub(super) struct Removal {
    pub request: u64,
    pub target: Option<ConnectionRef>,
    pub preview: Option<RemovalPreview>,
    pub removing: Option<u64>,
    pending: Option<String>,
    observed: std::collections::HashMap<String, u64>,
    pub loading: bool,
    pub waiting_drafts: bool,
    pub cancel_transfers: bool,
    pub error: Option<String>,
}
impl App {
    pub(super) fn removal_admitted(
        &mut self,
        request: u64,
        id: String,
        result: Result<RemovalJob, String>,
    ) {
        if self.removal.removing != Some(request) || self.removal.pending.as_ref() != Some(&id) {
            return;
        }
        match result {
            Ok(job) => {
                self.connection_removed(request, Ok(0));
                self.removal_changed(job);
                self.notice(
                    "Connection removed from view. Local cleanup will finish in the background.",
                    false,
                );
                self.request_page();
            }
            Err(error) => {
                self.pending_close = None;
                self.connection_removed(request, Err(error));
            }
        }
    }

    pub(super) fn removal_changed(&mut self, job: RemovalJob) {
        if self
            .removal
            .observed
            .get(&job.id)
            .is_some_and(|revision| *revision >= job.revision)
            || self.workspace.connections_revision > job.revision
                && !self.workspace.removals.iter().any(|row| row.id == job.id)
        {
            return;
        }
        self.removal.observed.insert(job.id.clone(), job.revision);
        let workspace = Arc::make_mut(&mut self.workspace);
        workspace.connections_revision = workspace.connections_revision.max(job.revision);
        workspace.removals.retain(|row| row.id != job.id);
        if job.stage != RemovalStage::Succeeded {
            workspace.removals.push(job.clone());
        }
        match job.target.kind {
            ConnectionKind::Account => {
                workspace
                    .accounts
                    .retain(|account| account.id != job.target.id);
                workspace.account_folders.remove(&job.target.id);
                workspace.folder_trees.remove(&job.target.id);
                workspace
                    .drafts
                    .retain(|draft| draft.account_id != job.target.id);
            }
            ConnectionKind::Calendar => workspace
                .calendars
                .retain(|source| source.id != job.target.id),
        }
        if let Some(revision) = job.calendar_revision {
            self.events_revision = self.events_revision.max(revision);
            Arc::make_mut(&mut self.calendar_actions.base)
                .retain(|event| event.source_id != job.target.id);
            self.project_calendar_actions();
        }
        if let Some(error) = job.error {
            self.notice(error, true);
        }
        self.send(Command::BulkRun(String::new()));
    }

    pub(super) fn removal_progress_view(&self, kind: ConnectionKind) -> Element<'_, Message> {
        let mut body = column![].spacing(10);
        for job in self
            .workspace
            .removals
            .iter()
            .filter(|job| job.target.kind == kind)
        {
            let title = if kind == ConnectionKind::Account {
                "Account removal"
            } else {
                "Calendar removal"
            };
            body = body
                .push(text(format!("{title}: {}", job.label)).size(14))
                .push(
                    text(match job.stage {
                        RemovalStage::Queued => {
                            "Hidden from this device. Waiting for active work before local cleanup."
                        }
                        RemovalStage::Cleanup => {
                            "Local data removed. Cleaning up saved credentials."
                        }
                        RemovalStage::Failed => "Local removal needs attention.",
                        RemovalStage::Succeeded => "Local removal finished.",
                    })
                    .size(12),
                );
            if let Some(error) = &job.error {
                body = body.push(text(error).size(12));
            }
            if job.stage == RemovalStage::Failed {
                body = body.push(action(
                    "Retry local cleanup",
                    Message::RetryRemoval(job.id.clone()),
                ));
            }
        }
        body.into()
    }

    pub(super) fn review_removal(&mut self, target: ConnectionRef) {
        if self.removal.removing.is_some() {
            return;
        }
        self.open(Dialog::Removal);
        self.removal.target = Some(target.clone());
        self.removal.request += 1;
        self.removal.preview = None;
        self.removal.cancel_transfers = false;
        self.removal.error = None;
        self.removal.waiting_drafts = true;
        self.removal.loading = true;
        self.continue_removal_review();
    }
    pub(super) fn continue_removal_review(&mut self) {
        if !self.removal.waiting_drafts || self.dialog != Some(Dialog::Removal) {
            return;
        }
        let Some(target) = self.removal.target.clone() else {
            return;
        };
        if target.kind == ConnectionKind::Account {
            self.flush_draft_saves(true);
            let pending = std::iter::once(&self.composer.current)
                .chain(self.composer.parked.values())
                .any(|session| {
                    session.draft.account_id == target.id
                        && (session.dirty.is_some()
                            || session.pending.is_some()
                            || self.composer.io.as_deref() == Some(session.draft.id.as_str()))
                });
            if pending {
                return;
            }
        }
        self.removal.waiting_drafts = false;
        self.removal.loading =
            self.try_command(Command::RemovalPreview(self.removal.request, target));
        if !self.removal.loading {
            self.removal.error =
                Some("Could not check local data. Review again when Shep is ready.".into());
        }
    }
    pub(super) fn fail_removal_draft_wait(&mut self, id: &str, error: &str) {
        if self.removal.waiting_drafts
            && self.removal.target.as_ref().is_some_and(|target| {
                target.kind == ConnectionKind::Account
                    && self
                        .owned_draft(id)
                        .is_some_and(|d| d.account_id == target.id)
            })
        {
            self.removal.waiting_drafts = false;
            self.removal.loading = false;
            self.removal.error = Some(format!(
                "Save the draft before removing this account. {error}"
            ));
        }
    }
    pub(super) fn removal_preview(&mut self, request: u64, result: Result<RemovalPreview, String>) {
        if request != self.removal.request || self.dialog != Some(Dialog::Removal) {
            return;
        }
        self.removal.loading = false;
        match result {
            Ok(preview) => self.removal.preview = Some(preview),
            Err(error) => self.removal.error = Some(error),
        }
    }
    pub(super) fn confirm_removal(&mut self) {
        if self.removal.removing.is_some() || self.removal.loading {
            return;
        }
        if let Some(preview) = self.removal.preview.clone() {
            if preview.transfers > 0 && !self.removal.cancel_transfers {
                return;
            }
            let id = uuid::Uuid::new_v4().to_string();
            self.pending_close = None;
            self.cancel_account_setup_stop();
            if self.bulk.stopped || self.bulk.stop_requested {
                if !self.try_command(Command::BulkResume(String::new())) {
                    return;
                }
                self.bulk.stopped = false;
                self.bulk.stop_requested = false;
            }
            if self.try_command(Command::AdmitRemoval(
                self.removal.request,
                id.clone(),
                preview,
                self.removal.cancel_transfers,
            )) {
                self.removal.removing = Some(self.removal.request);
                self.removal.pending = Some(id);
                self.removal.error = None;
            } else {
                self.removal.error = Some("Could not save this removal. Try again.".into());
            }
        }
    }
    pub(super) fn connection_removed(&mut self, request: u64, result: Result<usize, String>) {
        if self.removal.removing != Some(request) {
            return;
        }
        self.removal.removing = None;
        self.removal.pending = None;
        match result {
            Ok(failed) => {
                if self
                    .removal
                    .target
                    .as_ref()
                    .is_some_and(|t| t.kind == ConnectionKind::Account)
                {
                    self.profile_sync.connection_removed();
                    if let Some(target) = self.removal.target.clone() {
                        self.account_setup_removed(&target.id);
                        self.folder_creation_removed(&target.id);
                    }
                    if let Some(target) = &self.removal.target {
                        self.composer
                            .parked
                            .retain(|_, session| session.draft.account_id != target.id);
                        if self.composer.current.draft.account_id == target.id {
                            self.composer.current = Default::default();
                        }
                    }
                    self.selected = None;
                    self.detail = None;
                    self.conversation.page = Default::default();
                    self.conversation.generation += 1;
                    self.query.account = None;
                    self.query.offset = 0;
                }
                if self.dialog == Some(Dialog::Removal) {
                    self.dialog = None;
                    self.fields.clear();
                    if self.tab == Tab::Preferences {
                        self.settings_fields();
                    }
                }
                self.removal.preview = None;
                if failed > 0 {
                    self.notice("Connection removed. Some saved credentials could not be deleted. Unlock your credential store and use Retry credential cleanup in Preferences.", true);
                } else {
                    self.notice("Connection removed from this device.", false);
                }
            }
            Err(error) => {
                // The reviewed snapshot can change during a running sync. Require
                // another review rather than silently deleting newly arrived data.
                self.removal.preview = None;
                self.removal.error = Some(error.clone());
                if self.dialog != Some(Dialog::Removal) {
                    self.notice(error, true);
                }
            }
        }
    }
    pub(super) fn removal_form(&self) -> Element<'_, Message> {
        let state = &self.removal;
        let mut body = column![].spacing(16);
        if let Some(preview) = &state.preview {
            let mut identity = column![text(&preview.name).size(18).font(BOLD)].spacing(5);
            if !preview.address.is_empty() {
                identity = identity.push(muted(&preview.address).size(12));
            }
            body = body.push(identity);
            body = body.push(
                text(match preview.target.kind {
                    ConnectionKind::Account => format!(
                        "Remove this account and its local data: {} downloaded {}, {} {}.",
                        preview.messages,
                        if preview.messages == 1 {
                            "message"
                        } else {
                            "messages"
                        },
                        preview.drafts,
                        if preview.drafts == 1 {
                            "draft"
                        } else {
                            "drafts"
                        }
                    ),
                    ConnectionKind::Calendar => format!(
                        "Remove this calendar connection and {} cached {}.",
                        preview.events,
                        if preview.events == 1 {
                            "event"
                        } else {
                            "events"
                        }
                    ),
                })
                .size(13),
            );
            body = body.push(
                muted("Server messages, server calendars and saved backups are kept.").size(12),
            );
            if preview.outgoing > 0 {
                body=body.push(text(format!("{} outgoing recovery records will also be removed. Review Outbox first if delivery is uncertain.",preview.outgoing)).size(12));
            }
            if preview.mail_history > 0 {
                body=body.push(text(format!("{} mail-change history entries and their Undo receipts will also be removed.",preview.mail_history)).size(12));
            }
            if preview.transfers > 0 {
                body = body.push(container(column![
                    text(format!("{} unfinished {} this account. Copies may already exist at the destination.",preview.transfers, if preview.transfers == 1 { "mail change involves" } else { "mail changes involve" })).size(12),
                    checkbox(state.cancel_transfers).label("Cancel these unfinished mail changes locally").text_size(13).on_toggle_maybe(state.removing.is_none().then_some(Message::CancelPendingTransfers)),
                ].spacing(10)).padding(12).style(subtle));
            }
        } else if state.loading {
            body = body.push(muted(if state.waiting_drafts {
                "Saving drafts before checking local data…"
            } else {
                "Checking local data…"
            }));
        }
        if let Some(error) = &state.error {
            body = body.push(container(text(error).size(12)).padding(12).style(subtle));
            if let Some(target) = &state.target {
                body = body.push(action(
                    "Review again",
                    Message::ReviewRemoval(target.clone()),
                ));
            }
        }
        let can_remove = state.removing.is_none()
            && state
                .preview
                .as_ref()
                .is_some_and(|p| p.transfers == 0 || state.cancel_transfers);
        body.push(
            row![
                action(
                    if state.removing.is_some() {
                        "Close"
                    } else {
                        "Cancel"
                    },
                    Message::Close
                ),
                space().width(Length::Fill),
                button(
                    text(if state.removing.is_some() {
                        "Removing…"
                    } else {
                        "Remove from this device"
                    })
                    .size(12)
                )
                .padding([12, 18])
                .style(destructive)
                .on_press_maybe(can_remove.then_some(Message::ConfirmRemoval))
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .into()
    }
    pub(super) fn cleanup_preferences(&self) -> Element<'_, Message> {
        if self.workspace.credential_cleanup == 0 {
            return space().into();
        }
        container(column![text("Credential cleanup pending").font(BOLD).size(13), muted("Unused saved credentials need cleanup. Unlock your device credential store, then retry.").size(12),
            button(text("Retry credential cleanup").size(12)).padding([12,18]).style(outline).on_press_maybe((!self.busy.contains("credential-cleanup")).then_some(Message::CleanupCredentials))
        ].spacing(10)).padding(16).style(subtle).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_reply_is_correlated_and_late_progress_cannot_hide_reconnected_source() {
        let (mut app, _) = App::new();
        let mut job = RemovalJob {
            id: uuid::Uuid::new_v4().to_string(),
            target: target("home"),
            fingerprint: "review".into(),
            review_epoch: 0,
            cancel_transfers: false,
            label: "Home".into(),
            stage: RemovalStage::Queued,
            local_done: false,
            device_credentials: true,
            calendar_key: true,
            calendar_revision: Some(1),
            revision: 1,
            error: None,
        };
        app.removal.request = 3;
        app.removal.removing = Some(3);
        app.removal.pending = Some(job.id.clone());
        app.removal.target = Some(job.target.clone());
        let start = chrono::Utc::now();
        let old_events = Arc::new(vec![CalendarEvent {
            id: "old-event".into(),
            source_id: "home".into(),
            title: "Removed event".into(),
            start,
            end: start + chrono::Duration::hours(1),
            all_day: false,
            remote_url: None,
            etag: None,
            location: String::new(),
            description: String::new(),
        }]);
        app.calendar_actions.base = old_events.clone();
        app.project_calendar_actions();
        app.removal_admitted(3, "another attempt".into(), Ok(job.clone()));
        assert_eq!(app.removal.removing, Some(3));
        app.open(Dialog::Event);
        let _ = app.handle(Message::Field("title", "New editor".into()));
        app.removal_admitted(3, job.id.clone(), Ok(job.clone()));
        assert!(app.removal.removing.is_none());
        assert_eq!(app.dialog, Some(Dialog::Event));
        assert_eq!(app.field("title"), "New editor");
        assert_eq!(app.workspace.removals.len(), 1);
        assert!(app.events.is_empty());
        let _ = app.handle(Message::Backend(Event::Calendar(0, old_events)));
        assert!(
            app.events.is_empty(),
            "stale calendar reads cannot undo the admitted removal"
        );
        job.revision = 2;
        job.local_done = true;
        job.stage = RemovalStage::Succeeded;
        app.removal_changed(job.clone());
        assert!(app.workspace.removals.is_empty());
        let workspace = Arc::make_mut(&mut app.workspace);
        workspace.connections_revision = 3;
        workspace.calendars.push(CalendarSource {
            id: "home".into(),
            name: "Reconnected".into(),
            kind: CalendarKind::CalDav,
            url: "https://calendar.example.test/".into(),
            username: "alex".into(),
            access: Default::default(),
        });
        job.stage = RemovalStage::Failed;
        job.error = Some("Old failure".into());
        app.removal_changed(job);
        assert!(app.workspace.removals.is_empty());
        assert_eq!(app.workspace.calendars[0].name, "Reconnected");
    }
    fn target(id: &str) -> ConnectionRef {
        ConnectionRef {
            kind: ConnectionKind::Calendar,
            id: id.into(),
        }
    }
    fn preview(id: &str) -> RemovalPreview {
        RemovalPreview {
            removal_epoch: 0,
            target: target(id),
            name: id.into(),
            address: "https://calendar.example.test/".into(),
            messages: 0,
            drafts: 0,
            events: 1,
            transfers: 0,
            outgoing: 0,
            mail_history: 0,
            fingerprint: "fixture".into(),
        }
    }
    #[test]
    fn removal_review_and_acknowledgment_are_scoped_to_the_current_form() {
        let (mut app, _) = App::new();
        app.review_removal(target("home"));
        let old = app.removal.request;
        app.review_removal(target("work"));
        app.removal_preview(old, Ok(preview("home")));
        assert!(app.removal.preview.is_none());
        let request = app.removal.request;
        app.removal_preview(request, Ok(preview("work")));
        assert_eq!(app.removal.preview.as_ref().unwrap().target.id, "work");
        app.removal.removing = Some(request);
        app.open(Dialog::Event);
        let _ = app.handle(Message::Field("title", "Keep this editor".into()));
        app.connection_removed(request, Ok(0));
        assert_eq!(app.dialog, Some(Dialog::Event));
        assert_eq!(app.field("title"), "Keep this editor");
        assert!(app.removal.removing.is_none());
    }
    #[tokio::test]
    async fn old_workspaces_and_event_lists_cannot_restore_removed_connections() {
        let store = crate::store::Store::memory().unwrap();
        let source = CalendarSource {
            id: "home".into(),
            name: "Home".into(),
            url: "https://calendar.example.test/".into(),
            username: "alex".into(),
            kind: CalendarKind::CalDav,
            access: Default::default(),
        };
        store.save_source(source).await.unwrap();
        let start = chrono::Utc::now();
        let event = CalendarEvent {
            id: "event".into(),
            source_id: "home".into(),
            title: "Old event".into(),
            start,
            end: start + chrono::Duration::hours(1),
            all_day: false,
            remote_url: None,
            etag: None,
            location: String::new(),
            description: String::new(),
        };
        store.save_event(event).await.unwrap();
        let old = store.workspace().await.unwrap();
        let (old_revision, old_events) = store.calendar_snapshot().await.unwrap();
        let preview = store.removal_preview(target("home")).await.unwrap();
        store.remove_connection(preview, false).await.unwrap();
        let current = store.workspace().await.unwrap();
        let (revision, events) = store.calendar_snapshot().await.unwrap();
        let (mut app, _) = App::new();
        let _ = app.handle(Message::Backend(Event::Workspace(Arc::new(current))));
        let _ = app.handle(Message::Backend(Event::Calendar(
            revision,
            Arc::new(events),
        )));
        let _ = app.handle(Message::Backend(Event::Workspace(Arc::new(old))));
        let _ = app.handle(Message::Backend(Event::Calendar(
            old_revision,
            Arc::new(old_events),
        )));
        assert!(app.workspace.calendars.is_empty());
        assert!(app.events.is_empty());
        assert_eq!(app.workspace.credential_cleanup, 1);
        assert_eq!(app.events_revision, revision);
    }
}

#[cfg(test)]
mod inline_draft_tests {
    use super::*;
    #[test]
    fn review_waits_for_owned_saves_and_attachments_then_retires_only_removed_accounts() {
        let (mut app, _) = App::new();
        let (tx, mut rx, mut reads) = crate::engine::CommandSender::draft_review_test_channels();
        app.tx = Some(tx);
        let draft = Draft {
            id: "draft".into(),
            account_id: "account".into(),
            body: "Unsaved reply".into(),
            ..Default::default()
        };
        app.load_draft(draft.clone());
        app.composer.io = Some(draft.id.clone());
        app.review_removal(ConnectionRef {
            kind: ConnectionKind::Account,
            id: "account".into(),
        });
        assert!(app.removal.waiting_drafts);
        assert!(matches!(rx.try_recv().unwrap(), Command::AutoSaveDraft(_)));
        assert!(rx.try_recv().is_err());
        app.confirm_removal();
        assert!(rx.try_recv().is_err());
        let state = Arc::new(crate::store::DraftState {
            revision: 1,
            drafts: vec![draft.clone()],
        });
        let _ = app.draft_saved(draft.id.clone(), draft.revision, Ok(state.clone()));
        assert!(app.removal.waiting_drafts);
        assert!(rx.try_recv().is_err());
        let _ = app.handle(Message::Backend(Event::DraftFiles(draft.id, Ok(state))));
        assert!(!app.removal.waiting_drafts);
        assert!(
            matches!(reads.try_recv().unwrap(), Command::RemovalPreview(_, target) if target.id == "account")
        );
        app.composer.parked.insert(
            "other".into(),
            composing::Session {
                draft: Draft {
                    id: "other".into(),
                    account_id: "other".into(),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        app.removal.removing = Some(app.removal.request);
        app.connection_removed(app.removal.request, Ok(0));
        assert!(app.composer.current.draft.id.is_empty());
        assert!(app.composer.parked.contains_key("other"));
    }
    #[test]
    fn save_failure_stops_the_removal_review_until_explicit_retry() {
        let (mut app, _) = App::new();
        let (tx, mut rx, mut reads) = crate::engine::CommandSender::draft_review_test_channels();
        app.tx = Some(tx);
        let draft = Draft {
            id: "draft".into(),
            account_id: "account".into(),
            body: "Keep this reply".into(),
            ..Default::default()
        };
        app.load_draft(draft.clone());
        let target = ConnectionRef {
            kind: ConnectionKind::Account,
            id: "account".into(),
        };
        app.review_removal(target.clone());
        assert!(matches!(rx.try_recv().unwrap(), Command::AutoSaveDraft(_)));
        let _ = app.draft_saved(draft.id.clone(), draft.revision, Err("Disk full".into()));
        app.continue_removal_review();
        assert!(rx.try_recv().is_err());
        assert!(!app.removal.loading);
        assert!(app.removal.error.as_ref().unwrap().contains("Disk full"));
        app.review_removal(target);
        let Command::AutoSaveDraft(retry) = rx.try_recv().unwrap() else {
            panic!("retry owned draft")
        };
        assert!(retry.revision > draft.revision);
        assert_eq!(retry.body, draft.body);
        let _ = app.draft_saved(
            retry.id.clone(),
            retry.revision,
            Ok(Arc::new(crate::store::DraftState {
                revision: 1,
                drafts: vec![retry],
            })),
        );
        assert!(matches!(
            reads.try_recv().unwrap(),
            Command::RemovalPreview(..)
        ));
    }
}
