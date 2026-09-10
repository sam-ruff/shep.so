use super::*;
use crate::mail_actions::journal::{MoveRecord, MoveStage, RecoveryAction};
use iced::{
    Alignment, Length,
    widget::{button, checkbox, column, container, row, space, text},
};

#[derive(Debug, Clone)]
pub enum Message {
    Open(Option<Arc<MoveRecord>>),
    Select(String),
    Choose(RecoveryAction),
    Confirm(bool),
    Submit,
    Page(Option<String>),
}
#[derive(Default)]
pub(super) struct Recovery {
    pub rows: Arc<Vec<MoveRecord>>,
    pub selected: Option<Arc<MoveRecord>>,
    pub action: Option<RecoveryAction>,
    pub confirmed: bool,
    pub error: Option<String>,
    pub sequence: u64,
    pub page_request: u64,
    pub pending: HashMap<String, u64>,
    pub loading: bool,
    pub after: Option<String>,
}
fn wrap(message: Message) -> super::Message {
    super::Message::MoveRecovery(message)
}
fn label(stage: MoveStage) -> &'static str {
    match stage {
        MoveStage::Started => "Move needs review",
        MoveStage::Copied => "Copy saved · finish move",
        MoveStage::Committed => "Move confirmed · recovery needed",
        _ => "Recovery complete",
    }
}
impl App {
    pub(super) fn handle_move_recovery(&mut self, message: Message) {
        match message {
            Message::Open(record) => {
                self.open(Dialog::MoveRecovery);
                self.move_recovery.error = None;
                self.choose_recovery(record);
                self.load_move_recoveries(None);
            }
            Message::Select(token) => {
                let record = self
                    .move_recovery
                    .rows
                    .iter()
                    .find(|r| r.token == token)
                    .cloned()
                    .map(Arc::new);
                self.choose_recovery(record);
            }
            Message::Choose(action) => {
                self.move_recovery.action = Some(action);
                self.move_recovery.confirmed = false;
            }
            Message::Confirm(value) => self.move_recovery.confirmed = value,
            Message::Page(after) => {
                self.choose_recovery(None);
                self.load_move_recoveries(after);
            }
            Message::Submit => {
                let Some(record) = self.move_recovery.selected.clone() else {
                    return;
                };
                let Some(action) = self.move_recovery.action else {
                    return;
                };
                if self.move_recovery.loading
                    || self.move_recovery.pending.contains_key(&record.token)
                    || record.finished()
                    || (action != RecoveryAction::Retry && !self.move_recovery.confirmed)
                {
                    return;
                }
                if self.move_recovery.pending.len() >= CHANNEL_CAPACITY {
                    self.move_recovery.error =
                        Some("Let a recovery finish before starting another.".into());
                    return;
                }
                self.move_recovery.sequence += 1;
                let request = self.move_recovery.sequence;
                if self.try_command(Command::RecoverMailMove(
                    request,
                    record.clone(),
                    action,
                    self.move_recovery.confirmed,
                )) {
                    self.move_recovery
                        .pending
                        .insert(record.token.clone(), request);
                    self.move_recovery.error = None;
                } else {
                    self.move_recovery.error =
                        Some("The work queue is full. Try recovery again shortly.".into());
                }
            }
        }
    }
    fn choose_recovery(&mut self, record: Option<Arc<MoveRecord>>) {
        self.move_recovery.action = record.as_ref().map(|r| {
            if r.stage == MoveStage::Started {
                RecoveryAction::UseExistingCopy
            } else {
                RecoveryAction::Retry
            }
        });
        self.move_recovery.selected = record;
        self.move_recovery.confirmed = false;
        self.move_recovery.error = None;
    }
    pub(super) fn load_move_recoveries(&mut self, after: Option<String>) {
        self.move_recovery.after = after.clone();
        self.move_recovery.page_request += 1;
        self.move_recovery.loading = self.try_command(Command::MoveRecoveries(
            self.move_recovery.page_request,
            after,
        ));
        if !self.move_recovery.loading {
            self.move_recovery.error = Some("Recovery could not be loaded. Try again.".into());
        }
    }
    pub(super) fn move_recoveries_loaded(
        &mut self,
        request: u64,
        result: Result<Arc<Vec<MoveRecord>>, String>,
    ) {
        if request != self.move_recovery.page_request || self.dialog != Some(Dialog::MoveRecovery) {
            return;
        }
        self.move_recovery.loading = false;
        match result {
            Err(error) => self.move_recovery.error = Some(error),
            Ok(rows) => {
                if let Some(selected) = self.move_recovery.selected.clone()
                    && !self.move_recovery.pending.contains_key(&selected.token)
                    && let Some(current) = rows.iter().find(|r| r.token == selected.token)
                {
                    if current.stage != selected.stage {
                        self.move_recovery.confirmed = false;
                        if self.move_recovery.action != Some(RecoveryAction::KeepLocal) {
                            self.move_recovery.action =
                                Some(if current.stage == MoveStage::Started {
                                    RecoveryAction::UseExistingCopy
                                } else {
                                    RecoveryAction::Retry
                                });
                        }
                    }
                    self.move_recovery.selected = Some(Arc::new(current.clone()));
                }
                if self.move_recovery.selected.is_none() {
                    self.choose_recovery(rows.first().cloned().map(Arc::new));
                }
                self.move_recovery.rows = rows;
            }
        }
    }
    pub(super) fn move_recovery_finished(
        &mut self,
        request: u64,
        token: String,
        result: Result<Arc<MoveRecord>, String>,
    ) -> Task<super::Message> {
        if self.move_recovery.pending.get(&token) != Some(&request) {
            return Task::none();
        }
        self.move_recovery.pending.remove(&token);
        let reviewing = self.dialog == Some(Dialog::MoveRecovery)
            && self
                .move_recovery
                .selected
                .as_ref()
                .is_some_and(|r| r.token == token);
        match result {
            Ok(record) => {
                if reviewing {
                    self.dialog = None;
                    self.move_recovery.selected = None;
                }
                self.notice(
                    if record.stage == MoveStage::Kept {
                        "Local copy kept. Server copies are unchanged."
                    } else {
                        "Move recovered."
                    },
                    false,
                );
            }
            Err(error) => {
                self.pending_close = None;
                if reviewing {
                    self.move_recovery.error = Some(error.clone());
                    self.move_recovery.confirmed = false;
                } else {
                    self.notice(error, true);
                }
                if self.dialog == Some(Dialog::MoveRecovery) {
                    self.load_move_recoveries(None);
                }
            }
        }
        if self.move_recovery.pending.is_empty()
            && self.mail_actions.pending() == 0
            && let Some(window) = self.pending_close.take()
        {
            return self.handle(super::Message::WindowClose(window));
        }
        Task::none()
    }
    pub(super) fn move_recovery_bar(&self, id: &str) -> Element<'_, super::Message> {
        let Some(record) = self.page.move_recovery.get(id) else {
            return space().into();
        };
        let busy = self.move_recovery.pending.contains_key(&record.token);
        container(
            row![
                text(if busy {
                    "Recovering move…"
                } else {
                    label(record.stage)
                })
                .size(12),
                space().width(Length::Fill),
                button(text("Review").size(12))
                    .padding([10, 14])
                    .style(outline)
                    .on_press(wrap(Message::Open(Some(Arc::new(record.clone())))))
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .padding([8, 12])
        .style(subtle)
        .width(Length::Fill)
        .into()
    }
    pub(super) fn move_recovery_form(&self) -> Element<'_, super::Message> {
        let state = &self.move_recovery;
        let mut body = column![].spacing(14);
        if let Some(error) = &state.error {
            body = body.push(container(text(error).size(12)).padding(12).style(subtle));
        }
        if state.rows.len() > 1 {
            let mut choices = column![].spacing(4);
            for record in state.rows.iter() {
                let active = state
                    .selected
                    .as_ref()
                    .is_some_and(|r| r.token == record.token);
                choices = choices.push(
                    button(text(&record.original.subject).size(12))
                        .padding([10, 12])
                        .width(Length::Fill)
                        .style(if active { selected } else { outline })
                        .on_press(wrap(Message::Select(record.token.clone()))),
                );
            }
            body = body.push(iced::widget::scrollable(choices).height(140));
        }
        if state.after.is_some() || state.rows.len() == 50 {
            body = body.push(
                row![
                    button(text("First page").size(12))
                        .padding(10)
                        .style(outline)
                        .on_press_maybe((!state.loading).then_some(wrap(Message::Page(None)))),
                    button(text("More moves").size(12))
                        .padding(10)
                        .style(outline)
                        .on_press_maybe((!state.loading && state.rows.len() == 50).then_some(
                            wrap(Message::Page(state.rows.last().map(|r| r.token.clone())))
                        ))
                ]
                .spacing(8),
            );
        }
        if state.selected.is_none() {
            return body
                .push(muted(if state.loading {
                    "Loading unfinished moves…"
                } else {
                    "No moves need recovery."
                }))
                .into();
        }
        let record = state.selected.as_ref().unwrap();
        let busy = state.pending.contains_key(&record.token) || state.loading;
        let source = self.workspace.folder_label(
            Some(record.original.account_id.as_str()),
            &record.original.folder,
        );
        let destination = self.workspace.folder_label(
            Some(record.receipt.account.as_str()),
            &record.receipt.folder,
        );
        let account = |id: &str| {
            self.workspace
                .accounts
                .iter()
                .find(|a| a.id == id)
                .map(|a| a.name.as_str())
                .unwrap_or("Removed account")
                .to_owned()
        };
        body = body
            .push(text(&record.original.subject).size(16).font(BOLD))
            .push(
                muted(format!(
                    "{} · {source} → {} · {destination}",
                    account(&record.original.account_id),
                    account(&record.receipt.account)
                ))
                .size(12),
            )
            .push(text(label(record.stage)).size(13));
        if state.error.is_none()
            && let Some(error) = &record.error
        {
            body = body.push(muted(error).size(12));
        }
        let primary_action = if record.stage == MoveStage::Started {
            RecoveryAction::UseExistingCopy
        } else {
            RecoveryAction::Retry
        };
        let mut choices = row![].spacing(8);
        for (action, label) in [
            (
                primary_action,
                if primary_action == RecoveryAction::Retry {
                    "Finish recovery"
                } else {
                    "Use existing destination copy"
                },
            ),
            (RecoveryAction::KeepLocal, "Keep a local copy"),
        ] {
            choices = choices.push(
                button(text(label).size(12))
                    .padding([11, 13])
                    .style(if state.action == Some(action) {
                        selected
                    } else {
                        outline
                    })
                    .on_press_maybe((!busy).then_some(wrap(Message::Choose(action)))),
            );
        }
        body = body.push(choices.wrap());
        let action = state.action.unwrap_or(primary_action);
        let explanation = match action {
            RecoveryAction::Retry if record.stage == MoveStage::Copied => format!(
                "Verify the copy in {destination}, then finish removing the source message."
            ),
            RecoveryAction::Retry => format!(
                "Find the confirmed copy in {destination} and reconnect it to this cached message."
            ),
            RecoveryAction::UseExistingCopy => format!(
                "Find an unchanged copy in {destination}, then finish removing any remaining source message. Review both folders before choosing this."
            ),
            RecoveryAction::KeepLocal => format!(
                "Keep the original in {source} on this device. Any copies on the server remain unchanged."
            ),
        };
        body = body.push(text(explanation).size(12));
        if action != RecoveryAction::Retry {
            body = body.push(
                checkbox(state.confirmed)
                    .label(if action == RecoveryAction::KeepLocal {
                        "Keep this local copy and stop recovering the server move"
                    } else {
                        "I reviewed both folders and want to use the existing copy"
                    })
                    .text_size(12)
                    .on_toggle_maybe((!busy).then_some(|value| wrap(Message::Confirm(value)))),
            );
        }
        let enabled = !busy && (action == RecoveryAction::Retry || state.confirmed);
        body.push(
            row![
                button(text("Close").size(12))
                    .padding([11, 15])
                    .style(outline)
                    .on_press(super::Message::Close),
                space().width(Length::Fill),
                button(
                    text(if busy {
                        "Recovering…"
                    } else {
                        match action {
                            RecoveryAction::Retry => "Retry recovery",
                            RecoveryAction::UseExistingCopy => "Finish move",
                            RecoveryAction::KeepLocal => "Keep local copy",
                        }
                    })
                    .size(12)
                )
                .padding([11, 15])
                .style(primary)
                .on_press_maybe(enabled.then_some(wrap(Message::Submit)))
            ]
            .align_y(Alignment::Center)
            .spacing(8),
        )
        .into()
    }
}

#[cfg(test)]
#[path = "move_recovery_tests.rs"]
mod tests;
