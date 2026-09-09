use super::*;
use crate::store::{ConnectionKind, ConnectionRef, RemovalPreview};
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
    pub loading: bool,
    pub waiting_drafts: bool,
    pub cancel_transfers: bool,
    pub error: Option<String>,
}
impl App {
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
            if self.try_command(Command::RemoveConnection(
                self.removal.request,
                preview,
                self.removal.cancel_transfers,
            )) {
                self.removal.removing = Some(self.removal.request);
                self.removal.error = None;
            }
        }
    }
    pub(super) fn connection_removed(&mut self, request: u64, result: Result<usize, String>) {
        if self.removal.removing != Some(request) {
            return;
        }
        self.removal.removing = None;
        match result {
            Ok(failed) => {
                if self
                    .removal
                    .target
                    .as_ref()
                    .is_some_and(|t| t.kind == ConnectionKind::Account)
                {
                    self.profile_sync.connection_removed();
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
        container(column![text("Credential cleanup needs attention").font(BOLD).size(13), muted("A removed connection still has credentials in the OS store. Unlock it, then retry.").size(12),
            button(text("Retry credential cleanup").size(12)).padding([12,18]).style(outline).on_press_maybe((!self.busy.contains("credential-cleanup")).then_some(Message::CleanupCredentials))
        ].spacing(10)).padding(16).style(subtle).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target(id: &str) -> ConnectionRef {
        ConnectionRef {
            kind: ConnectionKind::Calendar,
            id: id.into(),
        }
    }
    fn preview(id: &str) -> RemovalPreview {
        RemovalPreview {
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
        assert!(matches!(rx.try_recv().unwrap(), Command::AutoSaveDraft(_)));
        let _ = app.draft_saved(
            draft.id.clone(),
            draft.revision,
            Ok(Arc::new(crate::store::DraftState {
                revision: 1,
                drafts: vec![draft],
            })),
        );
        assert!(matches!(
            reads.try_recv().unwrap(),
            Command::RemovalPreview(..)
        ));
    }
}
