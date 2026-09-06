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
        self.removal.loading =
            self.try_command(Command::RemovalPreview(self.removal.request, target));
        if !self.removal.loading {
            self.removal.error =
                Some("Could not check local data. Review again when Shep is ready.".into());
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
            if preview.transfers > 0 {
                body = body.push(container(column![
                    text(format!("{} unfinished {} this account. Copies may already exist at the destination.",preview.transfers, if preview.transfers == 1 { "move involves" } else { "moves involve" })).size(12),
                    checkbox(state.cancel_transfers).label("Cancel these unfinished moves locally").text_size(13).on_toggle_maybe(state.removing.is_none().then_some(Message::CancelPendingTransfers)),
                ].spacing(10)).padding(12).style(subtle));
            }
        } else if state.loading {
            body = body.push(muted("Checking local data…"));
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
