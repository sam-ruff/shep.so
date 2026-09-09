use super::*;
use crate::mail_actions::{Flags, MoveReceipt};
mod counts;
mod navigation;
mod projection;
mod undo;

#[derive(Default)]
pub(super) struct Actions {
    pub(super) read_candidate: Option<Mail>,
    pub base_page: Arc<MailPage>,
    flags: HashMap<String, PendingFlags>,
    sequence: u64,
    pub(super) follow: Option<navigation::Follow>,
    moves: HashMap<String, PendingMove>,
    transfers: HashMap<String, PendingTransfer>,
    undo: HashMap<u64, undo::Record>,
}

struct PendingTransfer {
    mail: Mail,
    account: String,
    folder: String,
    request: Option<u64>,
    toast: u64,
}

struct PendingMove {
    mail: Mail,
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

impl Actions {
    pub fn moving(&self, id: &str) -> bool {
        self.moves.contains_key(id) || self.transfers.contains_key(id)
    }
    pub fn pending(&self) -> usize {
        self.flags
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
    pub(super) fn set_mail_page(&mut self, page: Arc<MailPage>) {
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
            }
            keep
        });
        self.project_undo(&mut page);
        page.inbox_unread = self.project_inbox_counts();
        self.page = Arc::new(page);
        self.project_bulk();
    }

    pub(super) fn toggle_mail_flag(&mut self, mail: Mail, unread: bool) {
        if self.mail_actions.restoring(&mail.id) || self.bulk_owns_mail(&mail.id) {
            return;
        }
        if unread
            && self
                .mail_actions
                .read_candidate
                .as_ref()
                .is_some_and(|candidate| candidate.id == mail.id)
        {
            self.mail_actions.read_candidate = None;
        }
        let id = mail.id.clone();
        let current = self.mail_actions.effective(&mail).clone();
        let entry = self
            .mail_actions
            .flags
            .entry(id.clone())
            .or_insert_with(|| PendingFlags {
                confirmed: current.clone(),
                desired: current.clone(),
                sent: current,
                request: None,
                edits: (0, 0),
                sent_edits: (0, 0),
            });
        if unread {
            entry.desired.unread = !entry.desired.unread;
            entry.edits.0 += 1;
        } else {
            entry.desired.starred = !entry.desired.starred;
            entry.edits.1 += 1;
        }
        self.dispatch_flags(&id);
        self.invalidate_action_snapshot();
        self.project_mail_flags();
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
        if self.bulk_owns_mail(&mail.id) {
            return;
        }
        let id = mail.id.clone();
        if self.mail_actions.restoring(&id)
            || self.mail_actions.transfers.contains_key(&id)
            || self.mail_actions.moves.contains_key(&id)
        {
            return;
        }
        let neighbors = self.removal_neighbors(&mail.id);
        if self
            .mail_actions
            .read_candidate
            .as_ref()
            .is_some_and(|m| m.id == id)
        {
            self.finish_read();
        }
        let mail = self.mail_actions.effective(&mail).clone();
        let toast = self.action_toasts.add(&account, &folder, Instant::now());
        self.remember_move(toast, &mail);
        self.mail_actions.transfers.insert(
            id.clone(),
            PendingTransfer {
                mail,
                account,
                folder,
                request: None,
                toast,
            },
        );
        self.dispatch_transfer(&id);
        if self.mail_actions.transfers.contains_key(&id) {
            self.invalidate_action_snapshot();
            self.dialog = None;
            self.focused_input = None;
            self.pending_focus = None;
            self.project_mail_flags();
            self.select_after_removal(&id, neighbors);
        }
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
                destination: entry.folder.clone(),
                request: Some(request),
                toast: entry.toast,
            },
        );
        self.move_receipt(request, mail, entry.folder, result)
    }

    pub(super) fn move_mail(&mut self, mail: Mail, destination: String) {
        if self.mail_actions.restoring(&mail.id)
            || self.bulk_owns_mail(&mail.id)
            || mail.folder == destination
            || self.mail_actions.moves.contains_key(&mail.id)
            || self.mail_actions.transfers.contains_key(&mail.id)
        {
            return;
        }
        let neighbors = self.removal_neighbors(&mail.id);
        if self
            .mail_actions
            .read_candidate
            .as_ref()
            .is_some_and(|candidate| candidate.id == mail.id)
        {
            self.finish_read();
        }
        let mail = self.mail_actions.effective(&mail).clone();
        let id = mail.id.clone();
        let toast = self
            .action_toasts
            .add(&mail.account_id, &destination, Instant::now());
        self.remember_move(toast, &mail);
        self.mail_actions.moves.insert(
            id.clone(),
            PendingMove {
                toast,
                mail,
                destination,
                request: None,
            },
        );
        self.dispatch_move(&id);
        if !self.mail_actions.moves.contains_key(&id) {
            return;
        }
        self.invalidate_action_snapshot();
        self.dialog = None;
        self.focused_input = None;
        self.pending_focus = None;
        self.project_mail_flags();
        self.select_after_removal(&id, neighbors);
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
                self.confirm_move_display(&entry.mail, &receipt);
                self.mail_actions.flags.remove(&mail.id);
            }
            Err(error) => {
                self.reconcile_move_row(&entry.mail, None, true);
                let undo_requested = self
                    .mail_actions
                    .undo
                    .remove(&entry.toast)
                    .is_some_and(|r| r.restoring());
                if !undo_requested {
                    self.action_toasts.failed(entry.toast);
                }
                self.pending_close = None;
                self.notice(
                    format!(
                        "Could not move this message. It was restored to {}. {error}",
                        if mail.folder.eq_ignore_ascii_case("INBOX") {
                            "Inbox"
                        } else {
                            &mail.folder
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

    #[cfg(test)]
    fn move_finished(
        &mut self,
        request: u64,
        mail: Mail,
        folder: String,
        result: Result<(), String>,
    ) -> Task<Message> {
        let result = result.map(|()| Arc::new(MoveReceipt::local(&mail, &folder)));
        self.move_receipt(request, mail, folder, result)
    }
    #[cfg(test)]
    fn transfer_finished(
        &mut self,
        request: u64,
        mail: Mail,
        result: Result<(), String>,
    ) -> Task<Message> {
        let folder = self
            .mail_actions
            .transfers
            .get(&mail.id)
            .map(|e| e.folder.as_str())
            .unwrap_or("Archive");
        let result = result.map(|()| Arc::new(MoveReceipt::local(&mail, folder)));
        self.transfer_receipt(request, mail, result)
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
            counts::confirm_flags(&mut base, &confirmed);
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
mod tests {
    use super::*;
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
            let Command::Flags(request, mail, _) = commands.try_recv().unwrap() else {
                panic!("Expected read change");
            };
            assert_eq!(mail.id, original.summary.id);
            let _ = app.flags_finished(request, mail, Ok(()));
            let _ = app.key(
                Key::Character("m".into()),
                keyboard::Modifiers::empty(),
                false,
            );
            assert_eq!(app.dialog, Some(Dialog::Move));
            app.focused_input = Some("folder-search");
            let _ = app.handle(Message::Move("Archive".into()));
            let Command::Move(_, mail, destination) = commands.try_recv().unwrap() else {
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
        let Command::Flags(_, mail, _) = commands.try_recv().unwrap() else {
            panic!("Expected read change");
        };
        assert_eq!(mail.id, older.id);
        assert_eq!(mail.folder, "Archive");
    }

    pub(super) async fn fixture() -> (App, tokio::sync::mpsc::Receiver<Command>, Arc<MailDetail>) {
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
        (app, commands, detail)
    }
    fn command(commands: &mut tokio::sync::mpsc::Receiver<Command>) -> (u64, Mail, Flags) {
        match commands.try_recv().unwrap() {
            Command::Flags(request, mail, flags) => (request, mail, flags),
            other => panic!("Expected flags, got {other:?}"),
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
        let Command::Move(_, _, destination) = commands.try_recv().unwrap() else {
            panic!("Expected current Enter destination");
        };
        assert_eq!(destination, "INBOX");
    }
    #[tokio::test]
    async fn archive_removes_immediately_and_failed_move_restores_the_row() {
        let (mut app, mut commands, original) = fixture().await;
        let _ = app.handle(Message::Move("Archive".into()));
        assert!(app.page.rows.is_empty());
        assert_eq!(app.page.total, 0);
        assert_eq!(app.mail_actions.pending(), 1);
        let Command::Move(request, mail, folder) = commands.try_recv().unwrap() else {
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
        let _ = app.move_finished(request, mail, folder, Err("Read-only folder".into()));
        assert_eq!(app.page.total, 1);
        assert_eq!(app.page.rows[0].id, original.summary.id);
        assert_eq!(app.mail_actions.pending(), 0);
        assert!(app.notice.as_ref().unwrap().0.contains("restored"));
    }
    #[tokio::test]
    async fn toast_is_immediate_failure_only_changes_its_own_count_and_dismissal_sticks() {
        let (mut app, mut commands, original) = fixture().await;
        let _ = app.handle(Message::Move("Archive".into()));
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Archived 1 message"
        );
        let Command::Move(request, mail, folder) = commands.try_recv().unwrap() else {
            panic!("Expected move")
        };
        let mut another = original.summary.clone();
        another.id = "second".into();
        app.move_mail(another, "Archive".into());
        let Command::Move(second, other, other_folder) = commands.try_recv().unwrap() else {
            panic!("Expected move")
        };
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Archived 2 messages"
        );
        let _ = app.move_finished(
            request,
            mail.clone(),
            folder.clone(),
            Err("First rejected".into()),
        );
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Archived 1 message"
        );
        let _ = app.move_finished(request, mail, folder, Err("Stale rejection".into()));
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Archived 1 message"
        );
        let _ = app.handle(Message::DismissActionToast);
        let _ = app.move_finished(second, other, other_folder, Ok(()));
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
        let Command::Transfer(request, mail, _, _) = commands.try_recv().unwrap() else {
            panic!("Expected transfer")
        };
        app.project_mail_flags();
        assert_eq!(app.page.total, 0);
        let _ = app.transfer_finished(request, mail.clone(), Err("Upload rejected".into()));
        assert_eq!(app.page.total, 1);
        assert_eq!(app.page.rows[0].id, original.summary.id);
        assert!(app.action_toasts.current.is_none());
        assert!(app.notice.as_ref().unwrap().0.contains("Upload rejected"));
        let _ = app.transfer_finished(request, mail, Ok(()));
        assert_eq!(app.page.total, 1);
    }

    #[tokio::test]
    async fn archive_waits_for_latest_flags_without_waiting_to_hide_the_row() {
        let (mut app, mut commands, _) = fixture().await;
        let _ = app.handle(Message::ToggleRead);
        let (request, sent, _) = command(&mut commands);
        let _ = app.handle(Message::ToggleStar);
        let _ = app.handle(Message::Move("Archive".into()));
        assert!(app.page.rows.is_empty());
        assert!(commands.try_recv().is_err());
        let _ = app.flags_finished(request, sent, Ok(()));
        let (request, sent, _) = command(&mut commands);
        assert!(commands.try_recv().is_err());
        let _ = app.flags_finished(request, sent, Ok(()));
        let Command::Move(request, mail, folder) = commands.try_recv().unwrap() else {
            panic!("Expected queued move");
        };
        let _ = app.move_finished(request, mail.clone(), folder.clone(), Ok(()));
        assert_eq!(app.page.total, 0);
        assert_eq!(app.mail_actions.pending(), 0);
        let _ = app.move_finished(request, mail, folder, Err("Stale failure".into()));
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
    async fn immediate_read_and_flag_coalesce_and_preserve_body_and_newer_intent() {
        let (mut app, mut commands, original) = fixture().await;
        let _ = app.handle(Message::ToggleRead);
        assert!(!app.page.rows[0].unread);
        assert_eq!(app.page.inbox_unread["fixture"], 0);
        assert!(Arc::ptr_eq(app.detail.as_ref().unwrap(), &original));
        let (first, sent, patch) = command(&mut commands);
        assert_eq!(
            patch,
            Flags {
                unread: Some(false),
                starred: None
            }
        );
        let _ = app.handle(Message::ToggleStar);
        let _ = app.handle(Message::ToggleRead);
        assert!(commands.try_recv().is_err());
        assert!(app.page.rows[0].unread && app.page.rows[0].starred);
        // A background refresh cannot erase pending changes.
        let _ = app.handle(Message::Backend(Event::Page(
            app.generation,
            app.mail_actions.base_page.clone(),
            false,
        )));
        assert!(app.page.rows[0].starred);
        let _ = app.flags_finished(first, sent.clone(), Ok(()));
        let (second, latest, patch) = command(&mut commands);
        assert_eq!(
            patch,
            Flags {
                unread: Some(true),
                starred: Some(true)
            }
        );
        assert!(app.page.rows[0].unread && app.page.rows[0].starred);
        let _ = app.flags_finished(first, sent, Err("obsolete result".into()));
        assert_eq!(app.mail_actions.pending(), 1);
        let _ = app.flags_finished(second, latest, Ok(()));
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
        let (request, sent, _) = command(&mut commands);
        let _ = app.handle(Message::ToggleStar);
        let _ = app.flags_finished(request, sent, Err("Rejected by server".into()));
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
        let (first, sent, _) = command(&mut commands);
        let _ = app.handle(Message::ToggleStar);
        let _ = app.handle(Message::ToggleStar);
        let _ = app.flags_finished(first, sent, Err("Earlier attempt failed".into()));
        assert!(app.page.rows[0].starred);
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
        let (request, sent, _) = command(&mut commands);
        let _ = app.flags_finished(request, sent, Err("Save failed".into()));
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
                assert!(matches!(commands.try_recv(), Ok(Command::Move(_, _, _))));
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
        assert!(matches!(commands.try_recv(), Ok(Command::Move(_, _, _))));
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
