use super::*;

/// Metadata only. The provider owns raw bytes, credentials and server identities.
pub(super) struct Record {
    pub original: Mail,
    origin: MailQuery,
    position: Option<usize>,
    pub receipt: Option<Arc<MoveReceipt>>,
    requested: bool,
    request: Option<u64>,
    error: Option<String>,
    error_notice: Option<Instant>,
}
impl Record {
    pub fn restoring(&self) -> bool {
        self.requested && self.error.is_none()
    }
    pub fn pending(&self) -> bool {
        self.restoring() && self.receipt.is_some()
    }
}
impl Actions {
    pub fn restoring(&self, id: &str) -> bool {
        self.undo.values().any(|r| {
            r.restoring()
                && (r.original.id == id
                    || r.receipt
                        .as_ref()
                        .and_then(|r| r.current.as_ref())
                        .is_some_and(|m| m.id == id))
        })
    }
    pub fn undo_failures(&self) -> Vec<u64> {
        let mut ids: Vec<_> = self
            .undo
            .iter()
            .filter(|(_, r)| r.error.is_some())
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }
}
impl App {
    pub(in crate::ui) fn remember_move(&mut self, token: u64, mail: &Mail) {
        self.prune_undos();
        self.mail_actions.undo.insert(
            token,
            Record {
                original: mail.clone(),
                origin: self.query.clone(),
                position: self.page.rows.iter().position(|m| m.id == mail.id),
                receipt: None,
                requested: false,
                request: None,
                error: None,
                error_notice: None,
            },
        );
    }
    pub(in crate::ui) fn prune_undos(&mut self) {
        self.mail_actions.undo.retain(|token, r| {
            r.receipt.is_none()
                || r.requested
                || self
                    .action_toasts
                    .current
                    .as_ref()
                    .is_some_and(|t| t.contains(*token))
        });
    }
    pub(in crate::ui) fn undo_actions(&mut self, tokens: Vec<u64>) {
        let mut accepted = vec![];
        for token in tokens {
            let Some(record) = self.mail_actions.undo.get_mut(&token) else {
                continue;
            };
            if record.requested && record.error.is_none() {
                continue;
            }
            record.requested = true;
            record.error = None;
            let id = record.original.id.clone();
            // A move waiting behind a flag save has not reached the provider yet.
            // Cancel it locally; the already accepted flag intent still persists.
            let unsent = self
                .mail_actions
                .moves
                .get(&id)
                .is_some_and(|e| e.request.is_none())
                || self
                    .mail_actions
                    .transfers
                    .get(&id)
                    .is_some_and(|e| e.request.is_none());
            if unsent {
                self.mail_actions.moves.remove(&id);
                self.mail_actions.transfers.remove(&id);
                self.mail_actions.undo.remove(&token);
            }
            accepted.push(token);
        }
        if !accepted.is_empty() {
            self.invalidate_action_snapshot();
            self.action_toasts.restored(accepted, Instant::now());
        }
        self.dispatch_undos();
        self.project_mail_flags();
        // A destination reader cannot keep requesting a UID now being restored.
        // Keep a deliberately selected source placeholder, or continue to the next row.
        if self.selected.as_ref().is_some_and(|id| {
            self.mail_actions.restoring(id) && !self.page.rows.iter().any(|m| &m.id == id)
        }) {
            self.selected = None;
            self.detail = None;
            self.conversation = Default::default();
            if let Some(next) = self.page.rows.first() {
                self.select(next.id.clone());
            }
        }
    }
    pub(in crate::ui) fn dismiss_undo_errors(&mut self, tokens: Vec<u64>) {
        for token in tokens {
            if self
                .mail_actions
                .undo
                .get(&token)
                .is_some_and(|r| r.error.is_some())
            {
                self.mail_actions.undo.remove(&token);
            }
        }
    }
    pub(in crate::ui) fn dispatch_undos(&mut self) {
        let mut tokens: Vec<_> = self
            .mail_actions
            .undo
            .iter()
            .filter(|(_, r)| r.pending() && r.request.is_none())
            .map(|(token, _)| *token)
            .collect();
        tokens.sort_unstable();
        for token in tokens {
            let record = &self.mail_actions.undo[&token];
            self.mail_actions.sequence += 1;
            let request = self.mail_actions.sequence;
            let command = Command::UndoMove(
                request,
                record.original.clone(),
                record.receipt.clone().unwrap(),
            );
            let result = self.tx.as_ref().map(|tx| tx.try_send(command));
            match result {
                Some(Ok(())) => {
                    self.mail_actions.undo.get_mut(&token).unwrap().request = Some(request)
                }
                Some(Err(error))
                    if matches!(*error, tokio::sync::mpsc::error::TrySendError::Full(_)) =>
                {
                    break;
                }
                _ => {
                    self.mail_actions.undo.get_mut(&token).unwrap().error = Some("The mail worker is unavailable. Reopen Shep and check the destination folder.".into());
                    self.action_toasts.failed(token);
                    self.pending_close = None;
                    self.notice(
                        "Undo could not start. The message remains in its destination folder.",
                        true,
                    );
                }
            }
        }
    }
    pub(in crate::ui) fn undo_finished(
        &mut self,
        request: u64,
        original: Mail,
        result: Result<Arc<MoveReceipt>, String>,
    ) -> Task<Message> {
        let Some(token) = self
            .mail_actions
            .undo
            .iter()
            .find(|(_, r)| r.request == Some(request) && r.original.id == original.id)
            .map(|(id, _)| *id)
        else {
            return Task::none();
        };
        match result {
            Ok(receipt) => {
                let record = self.mail_actions.undo.remove(&token).unwrap();
                if self
                    .notice
                    .as_ref()
                    .is_some_and(|n| Some(n.2) == record.error_notice)
                {
                    self.notice = None;
                }
                let mut page = (*self.mail_actions.base_page).clone();
                counts::confirm_restore(&mut page, &record, &receipt);
                let inbox_counts = page.inbox_unread.clone();
                remove_row(&mut page, &original.id);
                if let Some(current) = record.receipt.as_ref().and_then(|r| r.current.as_ref()) {
                    remove_row(&mut page, &current.id);
                }
                if let Some(current) = &receipt.current {
                    if self.undo_visible(&record, current) {
                        insert_row(&mut page, current.clone(), record.position);
                    }
                    if self.selected.as_ref() == Some(&original.id) {
                        self.selected = None;
                        self.detail = None;
                        self.conversation = Default::default();
                        self.select(current.id.clone());
                        if current.unread {
                            self.mail_actions.read_candidate = Some(current.clone());
                        }
                    }
                }
                self.mail_actions.flags.remove(&original.id);
                page.inbox_unread = inbox_counts;
                self.mail_actions.base_page = Arc::new(page);
            }
            Err(error) => {
                let record = self.mail_actions.undo.get_mut(&token).unwrap();
                record.request = None;
                record.error = Some(error.clone());
                self.action_toasts.failed(token);
                self.pending_close = None;
                self.notice(
                    format!(
                        "Could not undo the move. Check the destination or retry Undo. {error}"
                    ),
                    true,
                );
                self.mail_actions.undo.get_mut(&token).unwrap().error_notice =
                    self.notice.as_ref().map(|n| n.2);
            }
        }
        self.dispatch_undos();
        self.project_mail_flags();
        let failure = self.notice.clone().filter(|(_, error, _)| *error);
        let refresh = self.handle(Message::Backend(Event::Changed));
        if let Some(failure) = failure {
            self.notice = Some(failure);
        }
        if self.mail_actions.pending() == 0
            && let Some(window) = self.pending_close.take()
        {
            return Task::batch([refresh, self.handle(Message::WindowClose(window))]);
        }
        refresh
    }
    fn undo_visible(&self, record: &Record, mail: &Mail) -> bool {
        let query = &self.query;
        if query.unread_only && !mail.unread
            || query.read_only && mail.unread
            || query.starred_only && !mail.starred
            || query.attachments_only && mail.attachment_count == 0
        {
            return false;
        }
        // Search membership comes from the original indexed result, never guessed
        // from a short preview. Paging/sent-folder membership is likewise exact.
        if !query.search.is_empty()
            || query.offset != 0
            || query.sent_only
            || query
                .folders
                .as_ref()
                .is_some_and(|f| f.iter().any(|f| f.sent_only))
        {
            return *query == record.origin && record.position.is_some();
        }
        let scope = |account: &Option<String>, folder: &str| {
            account.as_ref().is_none_or(|a| a == &mail.account_id)
                && (folder.is_empty() || folder == mail.folder)
        };
        if let Some(folders) = &query.folders {
            folders.iter().any(|f| scope(&f.account, &f.folder))
        } else {
            scope(&query.account, &query.folder)
        }
    }
    pub(in crate::ui) fn project_undo(&self, page: &mut MailPage) {
        let mut records: Vec<_> = self
            .mail_actions
            .undo
            .iter()
            .filter(|(_, r)| r.restoring())
            .collect();
        records.sort_by_key(|(token, _)| std::cmp::Reverse(**token));
        for (_, record) in records {
            if let Some(current) = record.receipt.as_ref().and_then(|r| r.current.as_ref()) {
                remove_row(page, &current.id);
            }
            if self.undo_visible(record, &record.original) {
                insert_row(
                    page,
                    self.mail_actions.effective(&record.original).clone(),
                    record.position,
                );
            }
        }
    }
}
fn remove_row(page: &mut MailPage, id: &str) {
    if let Some(index) = page.rows.iter().position(|m| m.id == id) {
        let mail = page.rows.remove(index);
        page.total = page.total.saturating_sub(1);
        if mail.unread {
            page.unread = page.unread.saturating_sub(1);
            if mail.folder.eq_ignore_ascii_case("INBOX") {
                let n = page.inbox_unread.entry(mail.account_id).or_default();
                *n = n.saturating_sub(1);
            }
        }
    }
}
fn insert_row(page: &mut MailPage, mail: Mail, position: Option<usize>) {
    if page.rows.iter().any(|m| m.id == mail.id) {
        return;
    }
    page.total += 1;
    if mail.unread {
        page.unread += 1;
        if mail.folder.eq_ignore_ascii_case("INBOX") {
            *page
                .inbox_unread
                .entry(mail.account_id.clone())
                .or_default() += 1;
        }
    }
    page.rows
        .insert(position.unwrap_or(0).min(page.rows.len()), mail);
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture;
    use super::*;

    fn tokens(app: &App) -> Vec<u64> {
        app.action_toasts.current.as_ref().unwrap().undo_tokens()
    }
    fn receipt(original: &Mail, folder: &str, account: &str) -> Arc<MoveReceipt> {
        Arc::new(MoveReceipt::server(
            original,
            account,
            folder,
            Some("42.99".into()),
            crate::mail_actions::Fingerprint::of(b"fictional mail"),
        ))
    }
    #[tokio::test]
    async fn undo_before_forward_ack_restores_immediately_then_uses_the_receipt_and_new_uid() {
        let (mut app, mut commands, detail) = fixture().await;
        let original = detail.summary.clone();
        app.move_mail(original.clone(), "Archive".into());
        let Command::Move(request, mail, folder) = commands.try_recv().unwrap() else {
            panic!()
        };
        let tokens = tokens(&app);
        app.undo_actions(tokens.clone());
        assert_eq!(app.page.total, 1);
        assert_eq!(app.page.rows[0].id, original.id);
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Restored 1 message"
        );
        assert!(
            commands.try_recv().is_err(),
            "Never reuse the old UID while MOVE is pending"
        );
        app.toggle_mail_flag(original.clone(), false);
        assert!(
            commands.try_recv().is_err(),
            "Placeholder actions cannot write an obsolete UID"
        );
        app.undo_actions(tokens);
        assert_eq!(app.mail_actions.pending(), 1);
        let moved = receipt(&mail, &folder, &mail.account_id);
        let _ = app.move_receipt(request, mail, folder, Ok(moved.clone()));
        let Command::UndoMove(reverse, source, acknowledged) = commands.try_recv().unwrap() else {
            panic!()
        };
        assert_eq!(source.id, original.id);
        assert_eq!(
            acknowledged.current.as_ref().unwrap().id,
            moved.current.as_ref().unwrap().id
        );
        assert_eq!(app.page.total, 1);
        assert_eq!(app.mail_actions.pending(), 1);
        // Late page snapshots cannot hide the user's latest restore intent.
        app.set_mail_page(Arc::new(MailPage::default()));
        assert_eq!(app.page.total, 1);
        let restored = receipt(&source, "INBOX", &source.account_id);
        let restored_id = restored.current.as_ref().unwrap().id.clone();
        let _ = app.undo_finished(reverse, source, Ok(restored));
        assert_eq!(app.page.rows[0].id, restored_id);
        assert_ne!(restored_id, original.id);
        assert_eq!(app.mail_actions.pending(), 0);
        assert!(commands.try_recv().is_err());
    }
    #[tokio::test]
    async fn undo_cancels_unsent_move_while_preserving_accepted_flag_changes() {
        let (mut app, mut commands, detail) = fixture().await;
        let mail = detail.summary.clone();
        app.toggle_mail_flag(mail.clone(), false);
        let Command::Flags(request, sent, _) = commands.try_recv().unwrap() else {
            panic!()
        };
        app.move_mail(mail, "Trash".into());
        app.undo_actions(tokens(&app));
        assert_eq!(app.page.total, 1);
        assert!(app.page.rows[0].starred);
        assert_eq!(app.mail_actions.pending(), 1);
        let _ = app.flags_finished(request, sent, Ok(()));
        assert!(commands.try_recv().is_err());
        assert_eq!(app.mail_actions.pending(), 0);
        assert!(app.mail_actions.undo.is_empty());
    }
    #[tokio::test]
    async fn undo_failure_retains_retry_and_uses_the_same_receipt_without_reviving_dismissed_feedback()
     {
        let (mut app, mut commands, detail) = fixture().await;
        let original = detail.summary.clone();
        app.move_mail(original.clone(), "Trash".into());
        let Command::Move(request, mail, folder) = commands.try_recv().unwrap() else {
            panic!()
        };
        let _ = app.move_receipt(
            request,
            mail.clone(),
            folder.clone(),
            Ok(receipt(&mail, &folder, &mail.account_id)),
        );
        app.undo_actions(tokens(&app));
        let Command::UndoMove(request, original, receipt) = commands.try_recv().unwrap() else {
            panic!()
        };
        let _ = app.undo_finished(request, original.clone(), Err("Rejected reversal".into()));
        assert_eq!(app.page.total, 0);
        assert_eq!(app.mail_actions.pending(), 0);
        assert!(app.action_toasts.current.is_none());
        let failed = app.mail_actions.undo_failures();
        assert_eq!(failed.len(), 1);
        app.action_toasts
            .expire(Instant::now() + std::time::Duration::from_secs(60));
        app.prune_undos();
        assert_eq!(app.mail_actions.undo_failures(), failed);
        app.undo_actions(failed);
        assert_eq!(app.page.total, 1);
        let Command::UndoMove(retry, source, retry_receipt) = commands.try_recv().unwrap() else {
            panic!()
        };
        assert!(Arc::ptr_eq(&receipt, &retry_receipt));
        let _ = app.undo_finished(request, original, Err("Stale failure".into()));
        assert_eq!(app.mail_actions.pending(), 1);
        app.action_toasts.current = None;
        let _ = app.undo_finished(
            retry,
            source.clone(),
            Ok(Arc::new(MoveReceipt::local(&source, "INBOX"))),
        );
        assert!(app.action_toasts.current.is_none());
        assert_eq!(app.page.total, 1);
        assert_eq!(app.mail_actions.pending(), 0);
    }
    #[tokio::test]
    async fn full_queue_keeps_undo_intent_and_dispatches_once_capacity_returns() {
        let (mut app, mut commands, detail) = fixture().await;
        app.move_mail(detail.summary.clone(), "Archive".into());
        let Command::Move(request, mail, folder) = commands.try_recv().unwrap() else {
            panic!()
        };
        let _ = app.move_receipt(
            request,
            mail.clone(),
            folder.clone(),
            Ok(receipt(&mail, &folder, &mail.account_id)),
        );
        for _ in 0..32 {
            app.tx
                .as_ref()
                .unwrap()
                .try_send(Command::LoadImages(vec![]))
                .unwrap();
        }
        app.undo_actions(tokens(&app));
        assert_eq!(app.page.total, 1);
        assert_eq!(app.mail_actions.pending(), 1);
        assert!(app.mail_actions.undo_failures().is_empty());
        for _ in 0..32 {
            commands.try_recv().unwrap();
        }
        app.dispatch_undos();
        assert!(matches!(commands.try_recv(), Ok(Command::UndoMove(..))));
        app.dispatch_undos();
        assert!(commands.try_recv().is_err());
    }
    #[tokio::test]
    async fn grouped_undo_preserves_each_account_folder_and_does_not_steal_navigation() {
        let (mut app, mut commands, detail) = fixture().await;
        let mut second = detail.summary.clone();
        second.id = "personal-original".into();
        second.account_id = "personal".into();
        second.folder = "Plans".into();
        app.move_mail(detail.summary.clone(), "Archive".into());
        app.move_mail(second, "Archive".into());
        let group = tokens(&app);
        assert_eq!(group.len(), 2);
        for _ in 0..2 {
            let Command::Move(request, mail, folder) = commands.try_recv().unwrap() else {
                panic!()
            };
            let _ = app.move_receipt(
                request,
                mail.clone(),
                folder.clone(),
                Ok(receipt(&mail, &folder, &mail.account_id)),
            );
        }
        app.query.folder = "Unrelated".into();
        app.set_mail_page(Arc::new(MailPage::default()));
        app.selected = Some("still-reading".into());
        app.undo_actions(group);
        assert_eq!(app.page.total, 0);
        assert_eq!(app.selected.as_deref(), Some("still-reading"));
        let Command::UndoMove(_, first, _) = commands.try_recv().unwrap() else {
            panic!()
        };
        let Command::UndoMove(_, second, _) = commands.try_recv().unwrap() else {
            panic!()
        };
        assert_eq!(first.folder, "INBOX");
        assert_eq!(second.folder, "Plans");
        assert_ne!(first.account_id, second.account_id);
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Restored 2 messages"
        );
    }
    #[tokio::test]
    async fn undo_in_destination_drops_obsolete_reader_and_ignores_its_late_result() {
        let (mut app, mut commands, detail) = fixture().await;
        app.move_mail(detail.summary.clone(), "Archive".into());
        let Command::Move(request, mail, folder) = commands.try_recv().unwrap() else {
            panic!()
        };
        let receipt = receipt(&mail, &folder, &mail.account_id);
        let current = receipt.current.as_ref().unwrap().clone();
        let _ = app.move_receipt(request, mail, folder, Ok(receipt));
        app.query.folder = "Archive".into();
        app.set_mail_page(Arc::new(MailPage {
            rows: vec![current.clone()],
            total: 1,
            ..Default::default()
        }));
        app.selected = Some(current.id.clone());
        let generation = app.conversation.generation;
        app.undo_actions(tokens(&app));
        assert!(app.selected.is_none());
        assert_eq!(app.page.total, 0);
        app.notice = None;
        let _ = app.conversation_result(
            generation,
            current.id,
            Err("Old UID no longer exists".into()),
        );
        assert!(app.notice.is_none());
    }
    #[tokio::test]
    async fn failed_forward_after_undo_keeps_restored_feedback_and_never_sends_reverse() {
        let (mut app, mut commands, detail) = fixture().await;
        app.move_mail(detail.summary.clone(), "Archive".into());
        let Command::Move(request, mail, folder) = commands.try_recv().unwrap() else {
            panic!()
        };
        app.undo_actions(tokens(&app));
        let _ = app.move_receipt(request, mail, folder, Err("Forward rejected".into()));
        assert_eq!(app.page.total, 1);
        assert_eq!(app.mail_actions.pending(), 0);
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Restored 1 message"
        );
        assert!(commands.try_recv().is_err());
        assert!(app.mail_actions.undo.is_empty());
    }
}
