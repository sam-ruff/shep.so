use super::*;
use crate::mail_actions::Flags;

#[derive(Default)]
pub(super) struct Actions {
    pub base_page: Arc<MailPage>,
    flags: HashMap<String, PendingFlags>,
    sequence: u64,
    moves: HashMap<String, PendingMove>,
}

struct PendingMove {
    mail: Mail,
    destination: String,
    request: Option<u64>,
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
    pub fn pending(&self) -> usize {
        self.flags
            .values()
            .filter(|entry| entry.request.is_some())
            .count()
            + self.moves.len()
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
        let reader = self.reader_id().map(str::to_owned);
        self.mail_actions
            .flags
            .retain(|id, entry| entry.request.is_some() || reader.as_ref() == Some(id));
        self.mail_actions.base_page = page;
        self.project_mail_flags();
    }

    fn project_mail_flags(&mut self) {
        if self.mail_actions.flags.is_empty() && self.mail_actions.moves.is_empty() {
            self.page = self.mail_actions.base_page.clone();
            return;
        }
        let mut page = (*self.mail_actions.base_page).clone();
        page.rows.retain_mut(|mail| {
            if self.mail_actions.moves.contains_key(&mail.id) {
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
        self.page = Arc::new(page);
    }

    pub(super) fn toggle_mail_flag(&mut self, mail: Mail, unread: bool) {
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

    pub(super) fn move_mail(&mut self, mail: Mail, destination: String) {
        if mail.folder == destination || self.mail_actions.moves.contains_key(&mail.id) {
            return;
        }
        let id = mail.id.clone();
        self.mail_actions.moves.insert(
            id.clone(),
            PendingMove {
                mail,
                destination,
                request: None,
            },
        );
        self.dispatch_move(&id);
        if !self.mail_actions.moves.contains_key(&id) {
            return;
        }
        self.dialog = None;
        self.project_mail_flags();
        if self.selected.as_ref() == Some(&id) || self.reader_id() == Some(id.as_str()) {
            self.selected = None;
            self.detail = None;
            self.conversation = Default::default();
            if let Some(next) = self.page.rows.first() {
                self.select(next.id.clone());
            }
        }
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
            self.mail_actions.moves.remove(id);
            self.pending_close = None;
        }
    }

    pub(super) fn move_finished(
        &mut self,
        request: u64,
        mail: Mail,
        folder: String,
        result: Result<(), String>,
    ) -> Task<Message> {
        if self
            .mail_actions
            .moves
            .get(&mail.id)
            .is_none_or(|entry| entry.request != Some(request))
        {
            return Task::none();
        }
        self.mail_actions.moves.remove(&mail.id);
        let failed = result.is_err();
        match result {
            Ok(()) => {
                let mut base = (*self.mail_actions.base_page).clone();
                if let Some(index) = base.rows.iter().position(|m| m.id == mail.id) {
                    let removed = base.rows.remove(index);
                    base.total = base.total.saturating_sub(1);
                    if removed.unread {
                        base.unread = base.unread.saturating_sub(1);
                        if removed.folder.eq_ignore_ascii_case("INBOX") {
                            let count = base.inbox_unread.entry(removed.account_id).or_default();
                            *count = count.saturating_sub(1);
                        }
                    }
                }
                self.mail_actions.base_page = Arc::new(base);
                self.notice(
                    format!(
                        "Moved to {}.",
                        if folder.eq_ignore_ascii_case("INBOX") {
                            "Inbox"
                        } else {
                            &folder
                        }
                    ),
                    false,
                );
            }
            Err(error) => {
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
        self.project_mail_flags();
        let failure_notice = failed.then(|| self.notice.clone()).flatten();
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
        if result.is_ok() {
            entry.confirmed = entry.sent.clone();
        }
        entry.desired = entry.confirmed.clone();
        newer.apply(&mut entry.desired);
        // Keep existing bodies in place. Only their small metadata is overlaid.
        let confirmed = entry.confirmed.clone();
        let mut base = (*self.mail_actions.base_page).clone();
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
                if mail.folder.eq_ignore_ascii_case("INBOX") {
                    adjust(
                        base.inbox_unread
                            .entry(mail.account_id.clone())
                            .or_default(),
                    );
                }
            }
            mail.unread = confirmed.unread;
            mail.starred = confirmed.starred;
        }
        self.mail_actions.base_page = Arc::new(base);
        let failed = result.is_err();
        if let Err(error) = result {
            self.pending_close = None;
            self.notice(
                format!("Could not update this message. The change was restored. {error}"),
                true,
            );
        }
        let failure_notice = failed.then(|| self.notice.clone()).flatten();
        self.dispatch_flags(&sent.id);
        self.dispatch_move(&sent.id);
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
    async fn fixture() -> (App, tokio::sync::mpsc::Receiver<Command>, Arc<MailDetail>) {
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
}
