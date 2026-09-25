use super::*;
use crate::mail_actions::{Flags, MoveReceipt};
mod admission;
mod counts;
mod navigation;
mod projection;
mod undo;
pub(super) use counts::badge_total;

#[derive(Default)]
pub(super) struct Actions {
    pub(super) read_candidate: Option<Mail>,
    pub(super) read_candidate_lineage: Option<String>,
    pub(super) move_review: Option<(Mail, Option<String>)>,
    pub base_page: Arc<MailPage>,
    flags: HashMap<String, PendingFlags>,
    sequence: u64,
    pub(super) follow: Option<navigation::Follow>,
    moves: HashMap<String, PendingMove>,
    transfers: HashMap<String, PendingTransfer>,
    undo: HashMap<u64, undo::Record>,
    admissions: VecDeque<admission::Admission>,
    journal_jobs: HashSet<String>,
}

struct PendingTransfer {
    mail: Mail,
    recovered: Option<Mail>,
    account: String,
    folder: String,
    request: Option<u64>,
    toast: u64,
}

struct PendingMove {
    mail: Mail,
    recovered: Option<Mail>,
    destination: String,
    request: Option<u64>,
    toast: u64,
}

struct PendingFlags {
    confirmed: Mail,
    desired: Mail,
    sent: Mail,
    request: Option<u64>,
    edits: (u64, u64),
    sent_edits: (u64, u64),
}

/// Plain words for a move the server refused and Shep completed locally.
fn local_only_notice(source_folder: &str) -> String {
    let source = if source_folder.eq_ignore_ascii_case("INBOX") {
        "Inbox"
    } else {
        source_folder
    };
    format!(
        "Moved on this device only. The mail server refused the move, so Shep will retry it during later checks. Until then, other devices still show the message in {source}."
    )
}

impl Actions {
    pub(super) fn selection_state(
        &self,
        id: &str,
        observed: &crate::store::SelectionObservation,
    ) -> crate::store::SelectionObservation {
        let mut state = observed.clone();
        if let Some(entry) = self.flags.get(id) {
            state.unread = entry.desired.unread;
            state.starred = entry.desired.starred;
        }
        if let Some((_, account, folder)) = self.move_target(id) {
            state.account = account.into();
            state.folder = folder.into();
        }
        for entry in self
            .admissions
            .iter()
            .filter(|entry| entry.original.id == id)
        {
            match &entry.action {
                crate::bulk::Action::Flags(flags) => {
                    if let Some(unread) = flags.unread {
                        state.unread = unread;
                    }
                    if let Some(starred) = flags.starred {
                        state.starred = starred;
                    }
                }
                crate::bulk::Action::Move { account, folder } => {
                    if let Some(account) = account {
                        state.account.clone_from(account);
                    }
                    state.folder.clone_from(folder);
                }
            }
        }
        state
    }

    pub(super) fn confirmed_flag_states(&self) -> Vec<Mail> {
        self.flags
            .values()
            .map(|entry| entry.confirmed.clone())
            .collect()
    }

    pub(super) fn flag_ids(&self) -> Vec<String> {
        self.flags
            .keys()
            .cloned()
            .chain(
                self.admissions
                    .iter()
                    .map(|entry| entry.original.id.clone()),
            )
            .collect()
    }

    pub fn moving(&self, id: &str) -> bool {
        self.move_target(id).is_some()
    }
    pub fn pending(&self) -> usize {
        self.admissions
            .iter()
            .filter(|entry| entry.revision.is_none())
            .count()
            + self
                .flags
                .values()
                .filter(|entry| entry.request.is_some())
                .count()
            + self.moves.len()
            + self.transfers.len()
            + self.undo.values().filter(|entry| entry.pending()).count()
    }
    pub fn effective<'a>(&'a self, mail: &'a Mail) -> &'a Mail {
        self.flags
            .get(&mail.id)
            .map_or(mail, |entry| &entry.desired)
    }
    pub fn observe_detail(&mut self, mail: &Mail) {
        if self
            .flags
            .get(&mail.id)
            .is_some_and(|entry| entry.request.is_none())
        {
            self.flags.remove(&mail.id);
        }
    }
}

impl App {
    pub(super) fn displayed_mail_flags(&self, mail: &Mail) -> (bool, bool) {
        if let Some(row) = self.page.rows.iter().find(|row| row.id == mail.id) {
            return (row.unread, row.starred);
        }
        if !self.bulk_owns_mail(&mail.id)
            && let Some(entry) = self.mail_actions.flags.get(&mail.id)
        {
            return (entry.desired.unread, entry.desired.starred);
        }
        if let Some(Some(observed)) = self.page.observed.get(&mail.id) {
            return (observed.unread, observed.starred);
        }
        (mail.unread, mail.starred)
    }

    pub(super) fn move_action_mail(&self) -> Option<&Mail> {
        if matches!(self.dialog, Some(Dialog::Move | Dialog::MoveConfirm))
            && let Some((mail, _)) = &self.mail_actions.move_review
        {
            return Some(mail);
        }
        if let Some(mail) = self.action_mail() {
            return Some(mail);
        }
        let id = self.reader_id()?;
        if self.mail_actions.restoring(id) || self.move_is_blocked(id) {
            return None;
        }
        let record = self.page.move_recovery.get(id)?;
        record.stage.unsubmitted().then_some(&record.original)
    }

    /// Explains a durable move the server refused and this device completed.
    pub(super) fn moved_on_this_device(&mut self, source_folder: &str) {
        self.notice(local_only_notice(source_folder), true);
    }

    /// The server refused this row's move; it sits at the destination on this
    /// device only until a later retry succeeds.
    pub(super) fn local_only_move(&self, id: &str) -> bool {
        self.page
            .move_recovery
            .get(id)
            .is_some_and(|record| record.stage == crate::mail_actions::journal::MoveStage::Local)
    }

    fn move_is_blocked(&self, id: &str) -> bool {
        if self.page.move_recovery.get(id).is_some_and(|record| {
            record.stage.unsubmitted() && !self.move_recovery.pending.contains_key(&record.token)
        }) {
            return self.bulk_action_owns_mail(id);
        }
        self.bulk_owns_mail(id)
    }

    pub(super) fn set_mail_page(&mut self, page: Arc<MailPage>) {
        if let Some(record) = page.move_recovery.values().find(|record| {
            record.stage == crate::mail_actions::journal::MoveStage::Local
                && !self
                    .mail_actions
                    .base_page
                    .move_recovery
                    .contains_key(&record.original.id)
        }) {
            self.notice(local_only_notice(&record.original.folder), true);
        }
        Arc::make_mut(&mut self.workspace).move_pending_total = page.move_pending_total;
        if let Some(id) = self.selected.clone()
            && let Some(current) = page.relocated.get(&id)
        {
            let original = self
                .page
                .rows
                .iter()
                .find(|m| m.id == id)
                .cloned()
                .or_else(|| {
                    self.detail
                        .as_ref()
                        .filter(|d| d.summary.id == id)
                        .map(|d| d.summary.clone())
                });
            if let Some(original) = original {
                self.reconcile_move_row(&original, Some(current), false);
            }
        }
        let reader = self.reader_id().map(str::to_owned);
        self.mail_actions
            .flags
            .retain(|id, entry| entry.request.is_some() || reader.as_ref() == Some(id));
        self.mail_actions.base_page = page;
        self.project_mail_flags();
        if let Some(detail) = &mut self.detail
            && self.page.lineages.get(&detail.summary.id) == detail.lineage.as_ref()
            && let Some(mail) = self
                .page
                .rows
                .iter()
                .find(|mail| mail.id == detail.summary.id)
            && (mail.account_id != detail.summary.account_id
                || mail.folder != detail.summary.folder
                || mail.remote_id != detail.summary.remote_id)
        {
            Arc::make_mut(detail).summary = mail.clone();
        }
        self.selection_page_changed();
    }

    pub(super) fn project_mail_flags(&mut self) {
        if self.mail_actions.flags.is_empty()
            && self.mail_actions.moves.is_empty()
            && self.mail_actions.transfers.is_empty()
            && !self
                .mail_actions
                .undo
                .values()
                .any(|entry| entry.restoring())
        {
            self.page = self.mail_actions.base_page.clone();
            self.project_mail_admissions();
            self.project_bulk();
            return;
        }
        let mut page = (*self.mail_actions.base_page).clone();
        page.rows.retain_mut(|mail| {
            if let Some((_, account, folder)) = self.mail_actions.move_target(&mail.id) {
                // A page may already contain the projected destination, or the
                // real local destination after its write and before its receipt.
                let restoring = self.mail_actions.restoring(&mail.id);
                let keep = !restoring && self.bulk_scope_contains(&self.query, account, folder);
                if keep {
                    if mail.account_id != account || mail.folder != folder {
                        mail.account_id = account.into();
                        mail.folder = folder.into();
                        mail.remote_id.clear();
                        page.move_placeholders.insert(mail.id.clone());
                    }
                } else {
                    page.total = page.total.saturating_sub(1);
                    if mail.unread {
                        page.unread = page.unread.saturating_sub(1);
                        if mail.folder.eq_ignore_ascii_case("INBOX") {
                            let count = page
                                .inbox_unread
                                .entry(mail.account_id.clone())
                                .or_default();
                            *count = count.saturating_sub(1);
                        }
                    }
                    return false;
                }
            }
            let effective = self.mail_actions.effective(mail);
            if mail.unread != effective.unread {
                let adjust = |count: &mut usize| {
                    *count = if effective.unread {
                        count.saturating_add(1)
                    } else {
                        count.saturating_sub(1)
                    };
                };
                adjust(&mut page.unread);
                if mail.folder.eq_ignore_ascii_case("INBOX") {
                    adjust(
                        page.inbox_unread
                            .entry(mail.account_id.clone())
                            .or_default(),
                    );
                }
            }
            let (unread, starred) = (effective.unread, effective.starred);
            mail.unread = unread;
            mail.starred = starred;
            let keep = !(self.query.unread_only && !mail.unread
                || self.query.read_only && mail.unread
                || self.query.starred_only && !mail.starred);
            if !keep {
                page.total = page.total.saturating_sub(1);
                page.unread = page.unread.saturating_sub(usize::from(mail.unread));
            }
            keep
        });
        self.project_undo(&mut page);
        page.inbox_unread = self.project_inbox_counts();
        self.page = Arc::new(page);
        self.project_mail_admissions();
        self.project_bulk();
    }

    pub(super) fn toggle_mail_flag(&mut self, mail: Mail, unread: bool) {
        let lineage = self.mail_input_lineage(&mail);
        self.toggle_mail_flag_with_lineage(mail, unread, lineage);
    }
    pub(super) fn toggle_mail_flag_with_lineage(
        &mut self,
        mail: Mail,
        unread: bool,
        lineage: Option<String>,
    ) {
        if self.mail_actions.restoring(&mail.id) || self.page.is_placeholder(&mail.id) {
            return;
        }
        if unread
            && self
                .mail_actions
                .read_candidate
                .as_ref()
                .is_some_and(|m| m.id == mail.id)
        {
            self.mail_actions.read_candidate = None;
            self.mail_actions.read_candidate_lineage = None;
        }
        let (read_state, flag_state) = self.displayed_mail_flags(&mail);
        let flags = Flags {
            unread: unread.then_some(!read_state),
            starred: (!unread).then_some(!flag_state),
        };
        self.admit_mail_action_with_lineage(mail, crate::bulk::Action::Flags(flags), lineage);
    }

    fn dispatch_flags(&mut self, id: &str) {
        let Some(entry) = self.mail_actions.flags.get_mut(id) else {
            return;
        };
        if entry.request.is_some() {
            return;
        }
        let changes = Flags::between(&entry.confirmed, &entry.desired);
        if changes.is_empty() {
            return;
        }
        self.mail_actions.sequence += 1;
        let request = self.mail_actions.sequence;
        entry.request = Some(request);
        entry.sent = entry.desired.clone();
        entry.sent_edits = entry.edits;
        let command = Command::Flags(request, entry.sent.clone(), changes);
        if !self.try_command(command) {
            self.mail_actions.flags.remove(id);
            self.pending_close = None;
        }
    }

    pub(super) fn transfer_mail(&mut self, mail: Mail, account: String, folder: String) {
        let lineage = self.mail_input_lineage(&mail);
        self.admit_mail_move(mail, Some(account), folder, lineage);
    }

    /// A hidden source row leaves a full page short until its receipt refreshes
    /// the list. Ask for the projected page now so a slow server does not show
    /// a gap at the bottom while the move is pending.
    fn refill_short_page(&mut self) {
        if self.mail_actions.follow.is_some()
            || self.page.rows.len() >= PAGE_SIZE
            || self.query.offset + self.page.rows.len() >= self.page.total
        {
            return;
        }
        self.request_page();
    }

    fn dispatch_transfer(&mut self, id: &str) {
        if self
            .mail_actions
            .flags
            .get(id)
            .is_some_and(|entry| entry.request.is_some())
        {
            return;
        }
        let Some(entry) = self.mail_actions.transfers.get_mut(id) else {
            return;
        };
        if entry.request.is_some() {
            return;
        }
        self.mail_actions.sequence += 1;
        let request = self.mail_actions.sequence;
        entry.request = Some(request);
        let command = Command::Transfer(
            request,
            entry.mail.clone(),
            entry.account.clone(),
            entry.folder.clone(),
        );
        if !self.try_command(command) {
            if let Some(entry) = self.mail_actions.transfers.remove(id) {
                self.action_toasts.failed(entry.toast);
                self.mail_actions.undo.remove(&entry.toast);
            }
            self.pending_close = None;
        }
    }

    pub(super) fn transfer_receipt(
        &mut self,
        request: u64,
        mail: Mail,
        result: Result<Arc<MoveReceipt>, String>,
    ) -> Task<Message> {
        if self
            .mail_actions
            .transfers
            .get(&mail.id)
            .is_none_or(|entry| entry.request != Some(request))
        {
            return Task::none();
        }
        let entry = self.mail_actions.transfers.remove(&mail.id).unwrap();
        // Share the same typed completion and rollback path as a folder move.
        self.mail_actions.moves.insert(
            mail.id.clone(),
            PendingMove {
                mail: entry.mail,
                recovered: entry.recovered,
                destination: entry.folder.clone(),
                request: Some(request),
                toast: entry.toast,
            },
        );
        self.move_receipt(request, mail, entry.folder, result)
    }

    pub(super) fn move_mail(&mut self, mail: Mail, destination: String) {
        let lineage = self.mail_input_lineage(&mail);
        self.admit_mail_move(mail, None, destination, lineage);
    }

    fn dispatch_move(&mut self, id: &str) {
        if self
            .mail_actions
            .flags
            .get(id)
            .is_some_and(|entry| entry.request.is_some())
        {
            return;
        }
        let Some(entry) = self.mail_actions.moves.get_mut(id) else {
            return;
        };
        if entry.request.is_some() {
            return;
        }
        self.mail_actions.sequence += 1;
        let request = self.mail_actions.sequence;
        entry.request = Some(request);
        let command = Command::Move(request, entry.mail.clone(), entry.destination.clone());
        if !self.try_command(command) {
            if let Some(entry) = self.mail_actions.moves.remove(id) {
                self.action_toasts.failed(entry.toast);
                self.mail_actions.undo.remove(&entry.toast);
            }
            self.pending_close = None;
        }
    }

    pub(super) fn move_receipt(
        &mut self,
        request: u64,
        mail: Mail,
        _folder: String,
        result: Result<Arc<MoveReceipt>, String>,
    ) -> Task<Message> {
        if self
            .mail_actions
            .moves
            .get(&mail.id)
            .is_none_or(|entry| entry.request != Some(request))
        {
            return Task::none();
        }
        let entry = self.mail_actions.moves.remove(&mail.id).unwrap();
        match result {
            Ok(receipt) => {
                if let Some(record) = self.mail_actions.undo.get_mut(&entry.toast) {
                    record.original = entry.mail.clone();
                    record.receipt = Some(receipt.clone());
                }
                self.action_toasts
                    .acknowledged(entry.toast, &receipt.account, &receipt.folder);
                self.confirm_move_display(
                    entry.recovered.as_ref().unwrap_or(&entry.mail),
                    &receipt,
                );
                self.mail_actions.flags.remove(&mail.id);
                if receipt.local_only {
                    let source = entry.recovered.as_ref().unwrap_or(&entry.mail);
                    self.notice(local_only_notice(&source.folder), true);
                }
            }
            Err(error) => {
                if entry.recovered.is_none() {
                    self.reconcile_move_row(&entry.mail, None, true);
                }
                let undo_requested = self
                    .mail_actions
                    .undo
                    .remove(&entry.toast)
                    .is_some_and(|r| r.restoring());
                if !undo_requested {
                    self.action_toasts.failed(entry.toast);
                }
                self.pending_close = None;
                let retained = entry.recovered.as_ref().unwrap_or(&entry.mail);
                self.notice(
                    format!(
                        "Could not complete this move. The message remains in {}. {error}",
                        if retained.folder.eq_ignore_ascii_case("INBOX") {
                            "Inbox"
                        } else {
                            &retained.folder
                        }
                    ),
                    true,
                );
            }
        }
        self.dispatch_undos();
        self.project_mail_flags();
        let failure_notice = self.notice.clone().filter(|(_, error, _)| *error);
        let refresh = self.handle(Message::Backend(Event::Changed));
        if let Some(notice) = failure_notice {
            self.notice = Some(notice);
        }
        if self.mail_actions.pending() == 0
            && let Some(window) = self.pending_close.take()
        {
            return Task::batch([refresh, self.handle(Message::WindowClose(window))]);
        }
        refresh
    }

    pub(super) fn flags_finished(
        &mut self,
        request: u64,
        sent: Mail,
        result: Result<(), String>,
    ) -> Task<Message> {
        let Some(entry) = self.mail_actions.flags.get_mut(&sent.id) else {
            return Task::none();
        };
        if entry.request != Some(request) {
            return Task::none();
        }
        entry.request = None;
        let newer = Flags {
            unread: (entry.edits.0 > entry.sent_edits.0).then_some(entry.desired.unread),
            starred: (entry.edits.1 > entry.sent_edits.1).then_some(entry.desired.starred),
        };
        let previous = entry.confirmed.clone();
        if result.is_ok() {
            entry.confirmed = entry.sent.clone();
        }
        entry.desired = entry.confirmed.clone();
        newer.apply(&mut entry.desired);
        // Keep existing bodies in place. Only their small metadata is overlaid.
        let confirmed = entry.confirmed.clone();
        for record in self
            .mail_actions
            .undo
            .values_mut()
            .filter(|r| r.original.id == sent.id)
        {
            record.original.unread = confirmed.unread;
            record.original.starred = confirmed.starred;
        }
        if let Some(moving) = self.mail_actions.moves.get_mut(&sent.id) {
            moving.mail.unread = confirmed.unread;
            moving.mail.starred = confirmed.starred;
        }
        if let Some(entry) = self.mail_actions.transfers.get_mut(&sent.id) {
            entry.mail.unread = confirmed.unread;
            entry.mail.starred = confirmed.starred;
        }
        let mut base = (*self.mail_actions.base_page).clone();
        if result.is_ok() {
            counts::confirm_flags(&mut base, &previous, &confirmed);
        }
        if let Some(mail) = base.rows.iter_mut().find(|m| m.id == sent.id) {
            if mail.unread != confirmed.unread {
                let adjust = |count: &mut usize| {
                    *count = if confirmed.unread {
                        count.saturating_add(1)
                    } else {
                        count.saturating_sub(1)
                    };
                };
                adjust(&mut base.unread);
            }
            mail.unread = confirmed.unread;
            mail.starred = confirmed.starred;
        }
        self.mail_actions.base_page = Arc::new(base);
        self.reconcile_bulk_flags(&confirmed);
        if let Err(error) = result {
            self.pending_close = None;
            self.notice(
                format!("Could not update this message. The change was restored. {error}"),
                true,
            );
        }
        let failure_notice = self.notice.clone().filter(|(_, error, _)| *error);
        self.dispatch_flags(&sent.id);
        self.dispatch_move(&sent.id);
        self.dispatch_transfer(&sent.id);
        self.project_mail_flags();
        let refresh = self.handle(Message::Backend(Event::Changed));
        if let Some(notice) = failure_notice {
            self.notice = Some(notice);
        }
        if self.mail_actions.pending() == 0
            && let Some(window) = self.pending_close.take()
        {
            return Task::batch([refresh, self.handle(Message::WindowClose(window))]);
        }
        refresh
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    #[tokio::test]
    async fn reader_flags_follow_projected_rows_and_offscreen_observations() {
        let (mut app, _, original) = fixture().await;
        let mut projected = original.summary.clone();
        projected.unread = !projected.unread;
        projected.starred = !projected.starred;
        let mut page = (*app.page).clone();
        page.rows = vec![projected.clone()];
        app.page = Arc::new(page.clone());
        assert_eq!(
            app.displayed_mail_flags(&original.summary),
            (projected.unread, projected.starred)
        );
        page.rows.clear();
        page.observed
            .insert(projected.id.clone(), Some(MailMembership::from(&projected)));
        app.page = Arc::new(page);
        assert_eq!(
            app.displayed_mail_flags(&original.summary),
            (projected.unread, projected.starred)
        );
        assert_ne!(original.summary.starred, projected.starred);
    }

    #[tokio::test]
    async fn metadata_actions_work_without_a_body_and_never_target_a_stale_body() {
        for stale in [false, true] {
            let (mut app, mut commands, original) = fixture().await;
            app.detail = if stale {
                let mut old = (*original).clone();
                old.summary.id = "another-message".into();
                Some(Arc::new(old))
            } else {
                None
            };
            let _ = app.handle(Message::ToggleRead);
            assert!(!app.page.rows[0].unread);
            let Command::AdmitMail(request, mail, crate::bulk::Action::Flags(_), _) =
                commands.try_recv().unwrap()
            else {
                panic!("Expected read change");
            };
            assert_eq!(mail.id, original.summary.id);
            finish(&mut app, request, Ok(())).await;
            let _ = app.key(
                Key::Character("m".into()),
                keyboard::Modifiers::empty(),
                false,
            );
            assert_eq!(app.dialog, Some(Dialog::Move));
            app.focused_input = Some("folder-search");
            let _ = app.handle(Message::Move("Archive".into()));
            let Command::AdmitMail(
                _,
                mail,
                crate::bulk::Action::Move {
                    folder: destination,
                    ..
                },
                _,
            ) = next(&mut commands).unwrap()
            else {
                panic!("Expected move");
            };
            assert_eq!(mail.id, original.summary.id);
            assert_eq!(destination, "Archive");
            assert!(app.focused_input.is_none());
            assert!(app.dialog.is_none());
        }
    }

    #[tokio::test]
    async fn pending_conversation_body_actions_keep_the_expanded_message_identity() {
        let (mut app, mut commands, original) = fixture().await;
        let mut older = original.summary.clone();
        older.id = "older-reply".into();
        older.folder = "Archive".into();
        app.conversation.focus = Some(older.id.clone());
        Arc::make_mut(&mut app.conversation.page).rows = vec![older.clone()];
        // The old anchor body is still cached while the expanded reply loads.
        let _ = app.handle(Message::ToggleRead);
        let Command::AdmitMail(_, mail, crate::bulk::Action::Flags(_), _) =
            commands.try_recv().unwrap()
        else {
            panic!("Expected read change");
        };
        assert_eq!(mail.id, older.id);
        assert_eq!(mail.folder, "Archive");
    }

    #[tokio::test]
    async fn a_pending_move_on_a_full_page_requests_the_projected_refill() {
        let store = crate::store::Store::memory().unwrap();
        let mails = (0..=PAGE_SIZE)
            .map(|index| {
                parse_mail(
                    "fixture",
                    &format!("{index}"),
                    "INBOX",
                    format!("From: a@example.test\r\nSubject: Row {index}\r\n\r\nbody")
                        .into_bytes(),
                    false,
                    false,
                )
                .unwrap()
            })
            .collect();
        store.upsert(mails).await.unwrap();
        let (sender, mut network, mut reads) = engine::CommandSender::move_test_channels();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        app.query.folder = "INBOX".into();
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        assert_eq!(
            (app.page.rows.len(), app.page.total),
            (PAGE_SIZE, PAGE_SIZE + 1)
        );
        let first = app.page.rows[0].clone();
        app.selected = Some(first.id.clone());
        app.move_mail(first.clone(), "Archive".into());
        let Command::AdmitMail(id, original, action, _) = network.try_recv().expect("admission")
        else {
            panic!("move admission")
        };
        store
            .start_individual_mail_action(id, original, action)
            .await
            .expect("saved admission");
        assert_eq!(
            (app.page.rows.len(), app.page.total),
            (PAGE_SIZE - 1, PAGE_SIZE)
        );
        // Selecting the neighbour also asks the read lane for its body.
        let queries = |reads: &mut tokio::sync::mpsc::Receiver<Command>| {
            std::iter::from_fn(|| reads.try_recv().ok())
                .filter_map(|command| match command {
                    Command::Query(generation, query, false) => Some((generation, query)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let mut refills = queries(&mut reads);
        assert_eq!(refills.len(), 1, "one refill per removal");
        let (generation, query) = refills.remove(0);
        assert_eq!(generation, app.generation);
        assert!(
            query.project_moves.is_empty(),
            "The journal owns saved projections"
        );
        assert_eq!(query.observe_bulk.len(), 1);
        // The store answers with the projection applied: a full page again.
        app.set_mail_page(Arc::new(store.query(query).await.unwrap()));
        assert_eq!(
            (app.page.rows.len(), app.page.total),
            (PAGE_SIZE, PAGE_SIZE)
        );
        assert!(!app.page.rows.iter().any(|mail| mail.id == first.id));
        // A short final page has nothing to pull in.
        let last = app.page.rows[PAGE_SIZE - 1].clone();
        app.move_mail(last, "Trash".into());
        assert!(matches!(
            next(&mut network),
            Ok(Command::AdmitMail(
                _,
                _,
                crate::bulk::Action::Move { .. },
                _
            ))
        ));
        assert!(queries(&mut reads).is_empty());
    }

    pub(super) async fn fixture() -> (App, tokio::sync::mpsc::Receiver<Command>, Arc<MailDetail>) {
        let (app, commands, detail, _) = fixture_store().await;
        (app, commands, detail)
    }
    pub(in crate::ui) async fn fixture_store() -> (
        App,
        tokio::sync::mpsc::Receiver<Command>,
        Arc<MailDetail>,
        crate::store::Store,
    ) {
        let store = crate::store::Store::memory().unwrap();
        let mail = parse_mail(
            "fixture",
            "42.7",
            "INBOX",
            b"From: fixture@example.test\r\nSubject: Actions\r\n\r\nSelectable unchanged body"
                .to_vec(),
            true,
            false,
        )
        .unwrap();
        let id = mail.summary.id.clone();
        store.upsert(vec![mail]).await.unwrap();
        let detail = Arc::new(store.detail(id.clone()).await.unwrap());
        let (sender, commands) = engine::CommandSender::network_test_channel();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        app.query.folder = "INBOX".into();
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        app.selected = Some(id);
        app.detail = Some(detail.clone());
        (app, commands, detail, store)
    }
    pub(super) async fn finish(app: &mut App, id: String, result: Result<(), String>) {
        if let Err(error) = result {
            app.mail_admitted(id, Err(error));
            return;
        }
        let Some(entry) = app
            .mail_actions
            .admissions
            .iter()
            .find(|entry| entry.id == id)
        else {
            return;
        };
        let original = entry.original.clone();
        let action = entry.action.clone();
        let store = crate::store::Store::memory().expect("store");
        let mut parsed = parse_mail(
            &original.account_id,
            &original.remote_id,
            &original.folder,
            b"From: fixture@example.test\r\nSubject: Actions\r\n\r\nBody".to_vec(),
            original.unread,
            original.starred,
        )
        .expect("mail");
        parsed.summary = original.clone();
        store.upsert(vec![parsed]).await.expect("stored");
        let job = store
            .start_individual_mail_action(id.clone(), original, action)
            .await
            .expect("admission");
        app.mail_admitted(id, Ok(Arc::new(job)));
    }
    fn command(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> (String, Mail, Flags) {
        match next(commands).unwrap() {
            Command::AdmitMail(request, mail, crate::bulk::Action::Flags(flags), _) => {
                (request, mail, flags)
            }
            other => panic!("Expected flags, got {other:?}"),
        }
    }
    pub(super) fn next(
        commands: &mut tokio::sync::mpsc::Receiver<Command>,
    ) -> Result<Command, tokio::sync::mpsc::error::TryRecvError> {
        loop {
            match commands.try_recv()? {
                Command::BulkRun(_) => continue,
                command => return Ok(command),
            }
        }
    }
    #[tokio::test]
    async fn enter_resolves_the_latest_typed_folder_without_waiting_for_a_view() {
        let (mut app, mut commands, detail) = fixture().await;
        let mut detail = (*detail).clone();
        detail.summary.folder = "Archive".into();
        app.detail = Some(Arc::new(detail));
        app.dialog = Some(Dialog::Move);
        Arc::make_mut(&mut app.workspace).folders = vec!["Archive".into(), "INBOX".into()];
        let _ = app.handle(Message::Field("folder_search", "inbox".into()));
        let _ = app.handle(Message::MoveFirst);
        let Command::AdmitMail(
            _,
            _,
            crate::bulk::Action::Move {
                folder: destination,
                ..
            },
            _,
        ) = commands.try_recv().unwrap()
        else {
            panic!("Expected current Enter destination");
        };
        assert_eq!(destination, "INBOX");
    }

    /// Two IMAP accounts, both move preferences on and the chooser open with
    /// a query that only the other account's folder matches.
    fn open_foreign_chooser(app: &mut App) {
        use super::super::move_candidates::test_account;
        let workspace = Arc::make_mut(&mut app.workspace);
        workspace.accounts = vec![
            test_account("fixture", "alex@studio.example", Protocol::Imap),
            test_account("other", "alex@example.com", Protocol::Imap),
        ];
        workspace
            .account_folders
            .insert("fixture".into(), vec!["INBOX".into(), "Archive".into()]);
        workspace
            .account_folders
            .insert("other".into(), vec!["INBOX".into(), "Home.Plans".into()]);
        app.preferences.cross_account_moves = true;
        app.preferences.foreign_move_folders = true;
        app.dialog = Some(Dialog::Move);
        let _ = app.handle(Message::Field("folder_search", "plans".into()));
    }
    fn press(app: &mut App, key: Key) {
        let _ = app.key(key, keyboard::Modifiers::empty(), false);
    }

    #[tokio::test]
    async fn foreign_first_result_opens_the_confirmation_instead_of_moving() {
        let (mut app, mut commands, _) = fixture().await;
        open_foreign_chooser(&mut app);
        let rows: Vec<_> = app
            .ranked_move_candidates()
            .into_iter()
            .map(|c| (c.account, c.folder, c.foreign))
            .collect();
        assert_eq!(rows, vec![("other".into(), "Home.Plans".into(), true)]);
        let _ = app.handle(Message::MoveFirst);
        assert_eq!(app.dialog, Some(Dialog::MoveConfirm));
        let confirm = app.move_confirm.as_ref().expect("pending choice");
        assert_eq!(
            (confirm.account.as_str(), confirm.folder.as_str()),
            ("other", "Home.Plans")
        );
        assert!(
            commands.try_recv().is_err(),
            "nothing moves before confirmation"
        );
        assert_eq!(app.page.rows.len(), 1);
        assert!(app.focused_input.is_none());
    }

    #[tokio::test]
    async fn escape_and_n_return_to_the_chooser_with_the_query_and_nothing_moves() {
        for key in [
            Key::Named(keyboard::key::Named::Escape),
            Key::Character("n".into()),
        ] {
            let (mut app, mut commands, _) = fixture().await;
            open_foreign_chooser(&mut app);
            let _ = app.handle(Message::MoveFirst);
            assert_eq!(app.dialog, Some(Dialog::MoveConfirm));
            press(&mut app, key);
            assert_eq!(app.dialog, Some(Dialog::Move));
            assert_eq!(app.field("folder_search"), "plans");
            assert!(app.move_confirm.is_none());
            assert!(commands.try_recv().is_err());
            assert_eq!(app.page.rows.len(), 1);
        }
    }

    #[tokio::test]
    async fn enter_and_y_confirm_and_dispatch_the_transfer() {
        for key in [
            Key::Named(keyboard::key::Named::Enter),
            Key::Character("y".into()),
        ] {
            let (mut app, mut commands, original) = fixture().await;
            open_foreign_chooser(&mut app);
            let _ = app.handle(Message::MoveFirst);
            press(&mut app, key);
            let Command::AdmitMail(
                _,
                mail,
                crate::bulk::Action::Move {
                    account: Some(account),
                    folder,
                },
                _,
            ) = commands.try_recv().unwrap()
            else {
                panic!("Expected a transfer");
            };
            assert_eq!(mail.id, original.summary.id);
            assert_eq!((account.as_str(), folder.as_str()), ("other", "Home.Plans"));
            assert_eq!(app.dialog, None);
            assert!(app.move_confirm.is_none());
            assert!(app.page.rows.is_empty(), "the source row hides at once");
        }
    }

    #[tokio::test]
    async fn close_clears_the_pending_choice() {
        let (mut app, mut commands, _) = fixture().await;
        open_foreign_chooser(&mut app);
        let _ = app.handle(Message::MoveFirst);
        let _ = app.handle(Message::Close);
        assert_eq!(app.dialog, None);
        assert!(app.move_confirm.is_none());
        let _ = app.handle(Message::ConfirmMove);
        assert!(commands.try_recv().is_err());
    }

    #[tokio::test]
    async fn confirmation_rechecks_the_message_and_preferences() {
        let (mut app, mut commands, _) = fixture().await;
        open_foreign_chooser(&mut app);
        let _ = app.handle(Message::MoveFirst);
        app.preferences.foreign_move_folders = false;
        let _ = app.handle(Message::ConfirmMove);
        assert!(commands.try_recv().is_err());
        assert_eq!(app.dialog, None);
        assert!(app.notice.as_ref().is_some_and(|n| n.1));

        let (mut app, mut commands, _) = fixture().await;
        open_foreign_chooser(&mut app);
        let _ = app.handle(Message::MoveFirst);
        Arc::make_mut(&mut app.workspace)
            .accounts
            .retain(|a| a.id != "other");
        let _ = app.handle(Message::ConfirmMove);
        assert!(commands.try_recv().is_err());
        assert_eq!(app.dialog, None);

        let (mut app, mut commands, _) = fixture().await;
        open_foreign_chooser(&mut app);
        let _ = app.handle(Message::MoveFirst);
        app.selected = Some("someone-else".into());
        app.detail = None;
        let _ = app.handle(Message::ConfirmMove);
        assert!(commands.try_recv().is_err());
        assert_eq!(app.dialog, None);
    }

    #[tokio::test]
    async fn foreign_rows_need_typing_and_both_preferences() {
        let (mut app, _, _) = fixture().await;
        open_foreign_chooser(&mut app);
        let _ = app.handle(Message::Field("folder_search", String::new()));
        let rows: Vec<_> = app
            .ranked_move_candidates()
            .into_iter()
            .map(|c| (c.folder, c.foreign))
            .collect();
        assert_eq!(
            rows,
            vec![("Archive".into(), false), ("INBOX".into(), false)]
        );
        let _ = app.handle(Message::Field("folder_search", "plans".into()));
        app.preferences.foreign_move_folders = false;
        assert!(app.ranked_move_candidates().is_empty());
        app.preferences.foreign_move_folders = true;
        app.preferences.cross_account_moves = false;
        assert!(app.ranked_move_candidates().is_empty());
        let _ = app.handle(Message::MoveForeign("other".into(), "Home.Plans".into()));
        assert_eq!(
            app.dialog,
            Some(Dialog::Move),
            "a disabled preference ignores the row"
        );
    }

    #[tokio::test]
    async fn explicit_pick_list_choice_still_transfers_immediately() {
        let (mut app, mut commands, original) = fixture().await;
        open_foreign_chooser(&mut app);
        let _ = app.handle(Message::Field("move_account", "other".into()));
        let rows: Vec<_> = app
            .ranked_move_candidates()
            .into_iter()
            .map(|c| (c.folder, c.foreign))
            .collect();
        assert_eq!(rows, vec![("Home.Plans".into(), false)]);
        let _ = app.handle(Message::MoveFirst);
        let Command::AdmitMail(
            _,
            mail,
            crate::bulk::Action::Move {
                account: Some(account),
                folder,
            },
            _,
        ) = commands.try_recv().unwrap()
        else {
            panic!("Expected an immediate transfer");
        };
        assert_eq!(mail.id, original.summary.id);
        assert_eq!((account.as_str(), folder.as_str()), ("other", "Home.Plans"));
        assert_eq!(app.dialog, None);
    }
    #[tokio::test]
    async fn archive_removes_immediately_and_failed_move_restores_the_row() {
        let (mut app, mut commands, original) = fixture().await;
        let _ = app.handle(Message::Move("Archive".into()));
        assert!(app.page.rows.is_empty());
        assert_eq!(app.page.total, 0);
        assert_eq!(app.mail_actions.pending(), 1);
        let Command::AdmitMail(request, _mail, crate::bulk::Action::Move { .. }, _) =
            commands.try_recv().unwrap()
        else {
            panic!("Expected move");
        };
        let _ = app.handle(Message::Backend(Event::Page(
            app.generation,
            app.mail_actions.base_page.clone(),
            false,
        )));
        assert!(
            app.page.rows.is_empty(),
            "A sync must not restore the pending move"
        );
        finish(&mut app, request, Err("Read-only folder".into())).await;
        assert_eq!(app.page.total, 1);
        assert_eq!(app.page.rows[0].id, original.summary.id);
        assert_eq!(app.mail_actions.pending(), 0);
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .0
                .contains("This change was not saved. Read-only folder")
        );
    }
    #[tokio::test]
    async fn refused_move_completes_on_this_device_and_says_so_in_plain_words() {
        use crate::mail_actions::{Fingerprint, journal::MoveStage};
        let (mut app, mut commands, original) = fixture().await;
        let _ = app.handle(Message::Move("Archive".into()));
        let Command::AdmitMail(request, mail, crate::bulk::Action::Move { folder, .. }, _) =
            commands.try_recv().unwrap()
        else {
            panic!("Expected move");
        };
        let mut receipt = MoveReceipt::server(
            &mail,
            &mail.account_id,
            &folder,
            None,
            Fingerprint::of(b"x"),
        );
        receipt.recovery = Some("device-only-token".into());
        receipt.local_only = true;
        finish(&mut app, request.clone(), Ok(())).await;
        let mut page = (*app.mail_actions.base_page).clone();
        page.rows.clear();
        page.total = 0;
        page.unread = 0;
        page.inbox_unread.clear();
        page.bulk_observed.insert(request, false);
        page.move_recovery.insert(
            mail.id.clone(),
            crate::mail_actions::journal::MoveRecord {
                token: "device-only-token".into(),
                original: mail,
                receipt,
                stage: MoveStage::Local,
                error: None,
                attempted: 0,
                retained: None,
            },
        );
        app.set_mail_page(Arc::new(page));
        assert_eq!(
            app.page.total, 0,
            "the row leaves Inbox instead of being restored"
        );
        assert_eq!(app.mail_actions.pending(), 0);
        let (notice, error, _) = app.notice.clone().unwrap();
        assert!(notice.contains("this device only"), "{notice}");
        assert!(
            notice.contains("retry") && notice.contains("Inbox"),
            "{notice}"
        );
        assert!(error, "the warning must survive the follow-up refresh");
        assert!(
            !notice.contains('\u{2014}') && notice.is_ascii(),
            "{notice}"
        );
        assert!(app.local_only_move(&original.summary.id));
        assert_eq!(
            app.page.move_recovery[&original.summary.id].stage,
            MoveStage::Local
        );
        assert!(
            app.action_toasts.current.is_some(),
            "Undo stays available for a device-only move"
        );
        // A later page carrying the stored device-only record keeps the marker.
        let mut page = (*app.mail_actions.base_page).clone();
        app.query.folder = "Archive".into();
        let mut row = original.summary.clone();
        row.folder = "Archive".into();
        row.remote_id.clear();
        page.rows = vec![row];
        page.total = 1;
        app.set_mail_page(Arc::new(page));
        assert!(app.local_only_move(&original.summary.id));
        assert_eq!(app.page.rows[0].folder, "Archive");
    }

    #[tokio::test]
    async fn toast_is_immediate_failure_only_changes_its_own_count_and_dismissal_sticks() {
        let (mut app, mut commands, original) = fixture().await;
        let _ = app.handle(Message::Move("Archive".into()));
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Archived 1 message"
        );
        let Command::AdmitMail(request, _, crate::bulk::Action::Move { .. }, _) =
            commands.try_recv().unwrap()
        else {
            panic!("Expected move")
        };
        let mut another = original.summary.clone();
        another.id = "second".into();
        app.move_mail(another, "Archive".into());
        let Command::AdmitMail(second, _, crate::bulk::Action::Move { .. }, _) =
            commands.try_recv().unwrap()
        else {
            panic!("Expected move")
        };
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Archived 2 messages"
        );
        finish(&mut app, request.clone(), Err("First rejected".into())).await;
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Archived 1 message"
        );
        finish(&mut app, request, Err("Stale rejection".into())).await;
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Archived 1 message"
        );
        let _ = app.handle(Message::DismissActionToast);
        finish(&mut app, second, Ok(())).await;
        assert!(
            app.action_toasts.current.is_none(),
            "Completion must not revive a dismissed toast"
        );
        assert!(app.notice.as_ref().unwrap().0.contains("First rejected"));
    }

    #[tokio::test]
    async fn transfer_feedback_is_immediate_and_failure_restores_without_changing_navigation() {
        let (mut app, mut commands, original) = fixture().await;
        app.transfer_mail(original.summary.clone(), "personal".into(), "Plans".into());
        assert_eq!(app.page.total, 0);
        assert_eq!(app.mail_actions.pending(), 1);
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Moved 1 message to Plans"
        );
        let Command::AdmitMail(request, _, crate::bulk::Action::Move { .. }, _) =
            commands.try_recv().unwrap()
        else {
            panic!("Expected transfer")
        };
        app.project_mail_flags();
        assert_eq!(app.page.total, 0);
        finish(&mut app, request.clone(), Err("Upload rejected".into())).await;
        assert_eq!(app.page.total, 1);
        assert_eq!(app.page.rows[0].id, original.summary.id);
        assert!(app.action_toasts.current.is_none());
        assert!(app.notice.as_ref().unwrap().0.contains("Upload rejected"));
        finish(&mut app, request, Ok(())).await;
        assert_eq!(app.page.total, 1);
    }

    #[tokio::test]
    async fn archive_is_admitted_after_each_flag_without_waiting_to_hide_the_row() {
        let (mut app, mut commands, _) = fixture().await;
        let _ = app.handle(Message::ToggleRead);
        let (read, _, _) = command(&mut commands);
        let _ = app.handle(Message::ToggleStar);
        let _ = app.handle(Message::Move("Archive".into()));
        assert!(app.page.rows.is_empty());
        let (star, _, _) = command(&mut commands);
        let Command::AdmitMail(request, _, crate::bulk::Action::Move { .. }, _) =
            commands.try_recv().unwrap()
        else {
            panic!("Expected immediately admitted move");
        };
        finish(&mut app, read, Ok(())).await;
        finish(&mut app, star, Ok(())).await;
        finish(&mut app, request.clone(), Ok(())).await;
        assert_eq!(app.page.total, 0);
        assert_eq!(app.mail_actions.pending(), 0);
        finish(&mut app, request, Err("Stale failure".into())).await;
        assert_eq!(app.page.total, 0);
    }
    #[tokio::test]
    async fn a_full_queue_cannot_hide_an_unaccepted_archive() {
        let (mut app, _commands, _) = fixture().await;
        while app
            .tx
            .as_ref()
            .unwrap()
            .try_send(Command::LoadImages(Vec::new()))
            .is_ok()
        {}
        let _ = app.handle(Message::Move("Archive".into()));
        assert_eq!(app.page.total, 1);
        assert_eq!(app.mail_actions.pending(), 0);
        assert!(app.action_toasts.current.is_none());
        assert!(app.notice.as_ref().unwrap().1);
    }
    #[tokio::test]
    async fn immediate_read_and_flag_admissions_preserve_body_and_newer_intent() {
        let (mut app, mut commands, original) = fixture().await;
        let _ = app.handle(Message::ToggleRead);
        assert!(!app.page.rows[0].unread);
        assert_eq!(app.page.inbox_unread["fixture"], 0);
        assert!(Arc::ptr_eq(app.detail.as_ref().unwrap(), &original));
        let (first, _, patch) = command(&mut commands);
        assert_eq!(
            patch,
            Flags {
                unread: Some(false),
                starred: None
            }
        );
        let _ = app.handle(Message::ToggleStar);
        let _ = app.handle(Message::ToggleRead);
        let (second, _, star) = command(&mut commands);
        let (third, _, read) = command(&mut commands);
        assert_eq!(
            star,
            Flags {
                unread: None,
                starred: Some(true)
            }
        );
        assert_eq!(
            read,
            Flags {
                unread: Some(true),
                starred: None
            }
        );
        assert!(app.page.rows[0].unread && app.page.rows[0].starred);
        // A background refresh cannot erase pending changes.
        let _ = app.handle(Message::Backend(Event::Page(
            app.generation,
            app.mail_actions.base_page.clone(),
            false,
        )));
        assert!(app.page.rows[0].starred);
        finish(&mut app, first.clone(), Ok(())).await;
        assert!(app.page.rows[0].unread && app.page.rows[0].starred);
        finish(&mut app, first, Err("obsolete result".into())).await;
        assert_eq!(app.mail_actions.pending(), 2);
        finish(&mut app, second, Ok(())).await;
        finish(&mut app, third, Ok(())).await;
        assert_eq!(app.mail_actions.pending(), 0);
        assert!(app.page.rows[0].unread && app.page.rows[0].starred);
        assert!(Arc::ptr_eq(app.detail.as_ref().unwrap(), &original));
    }
    #[tokio::test]
    async fn failed_read_restores_filtered_row_and_keeps_a_newer_flag() {
        let (mut app, mut commands, _) = fixture().await;
        app.query.unread_only = true;
        let _ = app.handle(Message::ToggleRead);
        assert_eq!(app.page.total, 0);
        assert!(app.page.rows.is_empty());
        let (request, _, _) = command(&mut commands);
        let _ = app.handle(Message::ToggleStar);
        finish(&mut app, request, Err("Rejected by server".into())).await;
        assert_eq!(app.page.total, 1);
        assert!(app.page.rows[0].unread && app.page.rows[0].starred);
        assert!(app.notice.as_ref().unwrap().1);
        let (_, _, patch) = command(&mut commands);
        assert_eq!(
            patch,
            Flags {
                unread: None,
                starred: Some(true)
            }
        );
    }
    #[tokio::test]
    async fn repeated_same_final_intent_survives_an_older_failure() {
        let (mut app, mut commands, _) = fixture().await;
        let _ = app.handle(Message::ToggleStar);
        let (first, _, _) = command(&mut commands);
        let _ = app.handle(Message::ToggleStar);
        let _ = app.handle(Message::ToggleStar);
        finish(&mut app, first, Err("Earlier attempt failed".into())).await;
        assert!(app.page.rows[0].starred);
        assert_eq!(command(&mut commands).2.starred, Some(false));
        assert_eq!(command(&mut commands).2.starred, Some(true));
    }
    #[tokio::test]
    async fn full_queue_restores_flags_and_close_waits_for_accepted_changes() {
        let (mut app, mut commands, _) = fixture().await;
        while app
            .tx
            .as_ref()
            .unwrap()
            .try_send(Command::LoadImages(Vec::new()))
            .is_ok()
        {}
        let _ = app.handle(Message::ToggleRead);
        assert!(app.page.rows[0].unread);
        assert_eq!(app.mail_actions.pending(), 0);
        while commands.try_recv().is_ok() {}
        let _ = app.handle(Message::ToggleRead);
        let window = iced::window::Id::unique();
        let _ = app.handle(Message::WindowClose(window));
        assert_eq!(app.pending_close, Some(window));
        let (request, _, _) = command(&mut commands);
        finish(&mut app, request, Err("Save failed".into())).await;
        assert!(app.pending_close.is_none());
        assert!(app.page.rows[0].unread);
    }
    #[tokio::test]
    async fn native_focus_blocks_unhandled_destructive_chords_in_search() {
        let (mut app, mut commands, _) = fixture().await;
        for focused in [true, false] {
            let _ = app.handle(Message::Key(
                Key::Character("d".into()),
                keyboard::Modifiers::CTRL,
                false,
                native_input::Focus {
                    search: focused,
                    ..Default::default()
                },
            ));
            if focused {
                assert!(commands.try_recv().is_err());
                assert_eq!(app.page.total, 1);
                assert!(app.action_toasts.current.is_none());
            } else {
                assert!(matches!(
                    commands.try_recv(),
                    Ok(Command::AdmitMail(
                        _,
                        _,
                        crate::bulk::Action::Move { .. },
                        _
                    ))
                ));
                assert_eq!(app.page.total, 0);
            }
        }
    }

    #[tokio::test]
    async fn full_reader_keys_do_not_wait_for_a_missing_search_widget() {
        let (mut app, mut commands, _) = fixture().await;
        app.full_reader = true;
        let _ = app.handle(Message::Key(
            Key::Named(keyboard::key::Named::Escape),
            keyboard::Modifiers::empty(),
            false,
            native_input::Focus::default(),
        ));
        assert!(!app.full_reader);
        app.full_reader = true;
        let _ = app.handle(Message::Key(
            Key::Character("d".into()),
            keyboard::Modifiers::CTRL,
            false,
            native_input::Focus::default(),
        ));
        assert!(matches!(
            commands.try_recv(),
            Ok(Command::AdmitMail(
                _,
                _,
                crate::bulk::Action::Move { .. },
                _
            ))
        ));
        app.tab = Tab::Preferences;
        let _ = app.handle(Message::Key(
            Key::Character("d".into()),
            keyboard::Modifiers::CTRL,
            false,
            native_input::Focus::default(),
        ));
        assert!(
            commands.try_recv().is_err(),
            "Mail shortcuts are scoped to mail"
        );
    }

    #[test]
    fn choosing_a_sort_cancels_stale_search_focus_observations_and_retries() {
        let (mut app, _) = App::new();
        app.focused_input = Some("search");
        app.pending_focus = Some("search");
        let _ = app.handle(Message::Sort(MailSort::Oldest));
        assert!(app.focused_input.is_none());
        assert!(app.pending_focus.is_none());
        let _ = app.handle(Message::Focus("search", 1));
        let _ = app.handle(Message::FocusChecked("search", true));
        assert!(app.focused_input.is_none());
        assert!(app.pending_focus.is_none());
    }
}
