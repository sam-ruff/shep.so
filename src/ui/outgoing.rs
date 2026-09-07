use super::*;
use crate::outgoing::{DeliveryState, OUTGOING_PAGE_SIZE, OutgoingPage, RecoveryAction, SentState};
use iced::{
    Alignment, Length,
    widget::{button, checkbox, column, container, row, space, text},
};

#[derive(Default)]
pub(super) struct Outbox {
    pub page: Arc<OutgoingPage>,
    pub request: u64,
    pub loading: bool,
    pub selected: Option<String>,
    pub confirmed: bool,
    pub error: Option<String>,
}
impl App {
    pub(super) fn open_outbox(&mut self) {
        self.open(Dialog::Outbox);
        self.outbox.confirmed = false;
        self.load_outbox(0);
    }
    pub(super) fn load_outbox(&mut self, offset: usize) {
        self.outbox.request += 1;
        self.outbox.error = None;
        self.outbox.loading = self.try_command(Command::OutgoingPage(self.outbox.request, offset));
        if !self.outbox.loading {
            self.outbox.error = Some("Outbox could not be loaded. Try again.".into());
        }
    }
    pub(super) fn outgoing_page(
        &mut self,
        request: u64,
        result: Result<Arc<OutgoingPage>, String>,
    ) {
        if request != self.outbox.request || self.dialog != Some(Dialog::Outbox) {
            return;
        }
        self.outbox.loading = false;
        match result {
            Ok(page) => {
                if page.revision < self.outbox.page.revision {
                    return;
                }
                if page.rows.is_empty() && page.total > 0 && page.offset > 0 {
                    self.load_outbox((page.total - 1) / OUTGOING_PAGE_SIZE * OUTGOING_PAGE_SIZE);
                    return;
                }
                self.outbox.error = None;
                let old = self
                    .outbox
                    .page
                    .rows
                    .iter()
                    .find(|r| Some(&r.attempt) == self.outbox.selected.as_ref())
                    .map(|r| (r.attempt.clone(), r.delivery, r.sent));
                if !page
                    .rows
                    .iter()
                    .any(|r| Some(&r.attempt) == self.outbox.selected.as_ref())
                {
                    self.outbox.selected = page.rows.first().map(|r| r.attempt.clone());
                }
                let new = page
                    .rows
                    .iter()
                    .find(|r| Some(&r.attempt) == self.outbox.selected.as_ref())
                    .map(|r| (r.attempt.clone(), r.delivery, r.sent));
                if old != new {
                    self.outbox.confirmed = false;
                }
                self.outbox.page = page;
            }
            Err(error) => self.outbox.error = Some(error),
        }
    }
    pub(super) fn resolve_outbox(&mut self, action: RecoveryAction) {
        if let Some(attempt) = self.outbox.selected.clone() {
            let key = format!("outgoing:{attempt}");
            if !self.busy.contains(&key)
                && self.try_command(Command::ResolveOutgoing(
                    attempt,
                    action,
                    self.outbox.confirmed,
                ))
            {
                self.busy.insert(key);
                self.outbox.error = None;
            }
        }
    }
    pub(super) fn outbox_view(&self) -> Element<'_, Message> {
        let mut body = column![].spacing(12);
        if let Some(error) = &self.outbox.error {
            body = body.push(text(error).size(12));
        }
        if self.outbox.page.rows.is_empty() {
            return body
                .push(muted(if self.outbox.loading {
                    "Loading Outbox…"
                } else {
                    "No outgoing messages need attention."
                }))
                .push(action("Refresh", Message::OutboxPage(0)))
                .into();
        }
        for info in &self.outbox.page.rows {
            let selected = self.outbox.selected.as_deref() == Some(&info.attempt);
            let status = match info.delivery {
                DeliveryState::Submitting | DeliveryState::Uncertain => "Delivery not confirmed",
                DeliveryState::Rejected => "Not sent",
                DeliveryState::Accepted => "Sent · copy needs attention",
                DeliveryState::Complete => "Sent",
                DeliveryState::Released => "Returned to drafts",
            };
            let title = if info.subject.is_empty() {
                "Untitled message"
            } else {
                &info.subject
            };
            let mut item = column![
                button(
                    row![
                        column![text(title).size(13).font(BOLD), muted(status).size(11)].spacing(4),
                        space().width(Length::Fill),
                        icon(if selected { "chevron-down" } else { "chevron" }, 16.)
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                )
                .padding(10)
                .width(Length::Fill)
                .style(ghost)
                .on_press(Message::SelectOutgoing(info.attempt.clone()))
            ]
            .spacing(12);
            if selected {
                let busy = self.busy.contains(&format!("outgoing:{}", info.attempt))
                    || self.busy.contains(&format!("send:{}", info.draft_id));
                let when = chrono::DateTime::from_timestamp(info.created, 0)
                    .unwrap_or_default()
                    .with_timezone(&chrono::Local)
                    .format("%d %b · %H:%M")
                    .to_string();
                item = item.push(
                    row![
                        muted(&info.from).size(12),
                        space().width(Length::Fill),
                        muted(when).size(11)
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center),
                );
                item = item.push(muted(format!("To: {}", info.to)).size(12));
                if let Some(error) = &info.error {
                    item = item.push(container(text(error).size(12)).padding(10).style(subtle));
                }
                let control = |label: &'static str, action: RecoveryAction, enabled: bool| {
                    button(text(label).size(12))
                        .padding([11, 14])
                        .style(outline)
                        .on_press_maybe(
                            (enabled && !busy).then_some(Message::ResolveOutgoing(action)),
                        )
                };
                if info.needs_delivery_review() {
                    item=item.push(text("This message may already have been sent. Check Sent or confirm what happened before trying another send.").size(12))
                        .push(control("Check server Sent",RecoveryAction::CheckSent,true))
                        .push(checkbox(self.outbox.confirmed).label("I reviewed delivery; another send could create a duplicate").text_size(12).on_toggle_maybe((!busy).then_some(Message::ConfirmOutgoing)))
                        .push(row![control("Record as sent",RecoveryAction::MarkSent,self.outbox.confirmed),control("Return to drafts",RecoveryAction::ReturnDraft,self.outbox.confirmed)].spacing(8).wrap());
                } else if info.delivery == DeliveryState::Rejected {
                    item = item.push(control(
                        "Return to drafts",
                        RecoveryAction::ReturnDraft,
                        true,
                    ));
                } else if info.delivery == DeliveryState::Accepted {
                    item = item.push(
                        text("Delivery was confirmed. These actions only manage the Sent copy.")
                            .size(12),
                    );
                    let ambiguous =
                        matches!(info.sent, SentState::Appending | SentState::Uncertain);
                    if ambiguous {
                        item = item.push(
                            checkbox(self.outbox.confirmed)
                                .label("I checked Sent; another saved copy may create a duplicate")
                                .text_size(12)
                                .on_toggle_maybe((!busy).then_some(Message::ConfirmOutgoing)),
                        );
                    }
                    item = item.push(
                        row![
                            control("Check server Sent", RecoveryAction::CheckSent, true),
                            control(
                                "Save server copy",
                                RecoveryAction::RetryCopy,
                                !ambiguous || self.outbox.confirmed
                            ),
                            control("Keep local copy", RecoveryAction::KeepLocal, true)
                        ]
                        .spacing(8)
                        .wrap(),
                    );
                }
                if busy {
                    item = item.push(muted("Working…").size(11));
                }
            }
            body = body.push(container(item).padding(12).style(card));
        }
        let page = &self.outbox.page;
        if page.offset == 0 && page.total <= OUTGOING_PAGE_SIZE {
            return body.into();
        }
        body.push(
            row![
                button(text("Previous").size(12))
                    .padding(10)
                    .style(outline)
                    .on_press_maybe((page.offset > 0).then(|| Message::OutboxPage(
                        page.offset.saturating_sub(OUTGOING_PAGE_SIZE)
                    ))),
                space().width(Length::Fill),
                muted(format!(
                    "{}–{} of {}",
                    page.offset + 1,
                    page.offset + page.rows.len(),
                    page.total
                ))
                .size(11),
                space().width(Length::Fill),
                button(text("Next").size(12))
                    .padding(10)
                    .style(outline)
                    .on_press_maybe(
                        (page.offset + page.rows.len() < page.total)
                            .then_some(Message::OutboxPage(page.offset + OUTGOING_PAGE_SIZE))
                    )
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outgoing::OutgoingInfo;
    fn page(revision: u64, delivery: DeliveryState) -> Arc<OutgoingPage> {
        Arc::new(OutgoingPage {
            revision,
            total: 1,
            offset: 0,
            rows: vec![OutgoingInfo {
                attempt: "attempt".into(),
                draft_id: "draft".into(),
                draft_revision: 1,
                account_id: "work".into(),
                from: "sender@example.test".into(),
                subject: "Recovery".into(),
                to: "friend@example.test".into(),
                created: 1,
                message_id: "<id@shep.local>".into(),
                delivery,
                sent: SentState::Pending,
                folder: None,
                error: None,
            }],
        })
    }
    #[test]
    fn outbox_pages_reject_old_results_and_new_recovery_stages_reset_confirmation() {
        let (mut app, _) = App::new();
        app.open_outbox();
        let request = app.outbox.request;
        app.outgoing_page(request, Ok(page(1, DeliveryState::Uncertain)));
        app.outbox.confirmed = true;
        app.outgoing_page(request, Ok(page(2, DeliveryState::Accepted)));
        assert!(!app.outbox.confirmed);
        app.outgoing_page(request, Ok(page(1, DeliveryState::Uncertain)));
        assert_eq!(app.outbox.page.rows[0].delivery, DeliveryState::Accepted);
        app.open(Dialog::Event);
        app.outgoing_page(request, Err("Old failure".into()));
        assert!(app.outbox.error.is_none());
    }
    #[test]
    fn durable_submission_closes_only_its_composer_and_leaves_navigation_available() {
        let (mut app, _) = App::new();
        app.open(Dialog::Compose);
        let id = app.composer.current.draft.id.clone();
        let revision = app.composer.current.draft.revision;
        app.busy.insert(format!("send:{id}"));
        let _ = app.handle(Message::Backend(Event::SubmissionQueued(
            id.clone(),
            revision,
        )));
        assert_eq!(app.dialog, None);
        let _ = app.handle(Message::Tab(Tab::Calendar));
        assert_eq!(app.tab, Tab::Calendar);
        app.open(Dialog::Event);
        let _ = app.handle(Message::Field("title", "Keep this event".into()));
        let _ = app.handle(Message::Backend(Event::Sent(id.clone(), revision)));
        let _ = app.handle(Message::Backend(Event::ReviewOutgoing(id, revision)));
        assert_eq!(app.dialog, Some(Dialog::Event));
        assert_eq!(app.field("title"), "Keep this event");
    }
}
