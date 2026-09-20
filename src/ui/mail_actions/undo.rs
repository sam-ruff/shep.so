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
    pub fn undo_available(&self, token: u64) -> bool {
        self.undo
            .get(&token)
            .is_some_and(|r| !r.requested || r.error.is_some())
    }
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
    #[cfg(test)]
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
            let original = record.original.clone();
            let id = original.id.clone();
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
                self.reconcile_move_row(&original, None, true);
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
    use super::super::tests::{fixture_store, next};
    use super::*;

    async fn admit(
        app: &mut App,
        store: &crate::store::Store,
        commands: &mut tokio::sync::mpsc::Receiver<Command>,
    ) -> String {
        let Command::AdmitMail(id, mail, action, proof) = next(commands).expect("admission") else {
            panic!("Expected durable admission")
        };
        let job = store
            .start_observed_mail_action(id.clone(), mail, action, proof.expect("observed source"))
            .await
            .expect("saved intent");
        app.mail_admitted(id.clone(), Ok(Arc::new(job)));
        id
    }

    async fn page(app: &mut App, store: &crate::store::Store) {
        let mut query = app.query.clone();
        query.observe = app.mail_actions.observed_ids();
        query.observe_bulk = app.bulk_observed_ids();
        if let Some(id) = &app.selected {
            query.observe.push(id.clone());
        }
        app.set_mail_page(Arc::new(store.query(query).await.expect("projected page")));
    }

    async fn undo(
        app: &mut App,
        store: &crate::store::Store,
        commands: &mut tokio::sync::mpsc::Receiver<Command>,
    ) -> String {
        let Command::BulkUndo(id) = next(commands).expect("Undo admission") else {
            panic!("Expected durable Undo")
        };
        let job = store
            .request_bulk_undo(id.clone())
            .await
            .expect("saved Undo");
        app.bulk_event(Event::BulkUpdate(Arc::new(job)));
        page(app, store).await;
        id
    }

    async fn moved(
        app: &mut App,
        store: &crate::store::Store,
        item: crate::bulk::Item,
        receipt: Arc<MoveReceipt>,
    ) {
        if let Some(current) = &receipt.current {
            let source = if item.undo {
                let Some(crate::bulk::Receipt::Move(forward)) = &item.receipt else {
                    panic!("forward receipt")
                };
                forward.current.as_ref().expect("actual destination")
            } else {
                item.original.as_ref().expect("original")
            };
            store
                .relocate_mail(source.clone(), current.clone())
                .await
                .expect("provider cache receipt");
            app.bulk_event(Event::BulkIdentity(
                item.job.clone(),
                item.id.clone(),
                Some(current.id.clone()),
            ));
        }
        let job = store
            .finish_bulk_item(
                item,
                Ok(crate::bulk::Receipt::Move(Box::new((*receipt).clone()))),
            )
            .await
            .expect("finish receipt");
        app.bulk_event(Event::BulkUpdate(Arc::new(job)));
        page(app, store).await;
    }

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
        let (mut app, mut commands, detail, store) = fixture_store().await;
        let original = detail.summary.clone();
        app.move_mail(original.clone(), "Archive".into());
        let id = admit(&mut app, &store, &mut commands).await;
        let item = store.claim_bulk_item(id.clone()).await.unwrap().unwrap();
        let tokens = tokens(&app);
        let previous_generation = app.generation;
        app.undo_combined_actions(tokens.clone());
        assert_eq!(app.page.total, 1);
        assert_eq!(app.page.rows[0].id, original.id);
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Restored 1 message"
        );
        assert_eq!(undo(&mut app, &store, &mut commands).await, id);
        assert!(
            store.claim_bulk_item(id.clone()).await.unwrap().is_none(),
            "Never reverse while the forward provider call is pending"
        );
        app.toggle_mail_flag(original.clone(), false);
        assert!(
            next(&mut commands).is_err(),
            "Placeholder actions cannot write an obsolete UID"
        );
        app.undo_combined_actions(tokens);
        let acknowledged = receipt(&original, "Archive", &original.account_id);
        moved(&mut app, &store, item, acknowledged.clone()).await;
        let reverse = store.claim_bulk_item(id.clone()).await.unwrap().unwrap();
        let source = reverse.original.as_ref().unwrap().clone();
        let Some(crate::bulk::Receipt::Move(forward)) = &reverse.receipt else {
            panic!("saved forward receipt")
        };
        assert_eq!(source.id, original.id);
        assert_eq!(
            forward.current.as_ref().unwrap().id,
            acknowledged.current.as_ref().unwrap().id
        );
        assert_eq!(app.page.total, 1);
        // Late page snapshots cannot hide the user's latest restore intent.
        let _ = app.handle(Message::Backend(Event::Page(
            previous_generation,
            Arc::new(MailPage::default()),
            false,
        )));
        assert_eq!(app.page.total, 1);
        let restored = receipt(&source, "INBOX", &source.account_id);
        let restored_id = restored.current.as_ref().unwrap().id.clone();
        moved(&mut app, &store, reverse, restored).await;
        assert_eq!(app.page.rows[0].id, restored_id);
        assert_ne!(restored_id, original.id);
        assert_eq!(app.mail_actions.pending(), 0);
        assert!(next(&mut commands).is_err());
    }
    #[tokio::test]
    async fn undo_cancels_unsent_move_while_preserving_accepted_flag_changes() {
        let (mut app, mut commands, detail, store) = fixture_store().await;
        let mail = detail.summary.clone();
        app.toggle_mail_flag(mail.clone(), false);
        let flags = admit(&mut app, &store, &mut commands).await;
        app.move_mail(mail, "Trash".into());
        let movement = admit(&mut app, &store, &mut commands).await;
        app.undo_combined_actions(tokens(&app));
        assert_eq!(app.page.total, 1);
        assert!(app.page.rows[0].starred);
        assert_eq!(undo(&mut app, &store, &mut commands).await, movement);
        assert!(store.claim_bulk_item(movement).await.unwrap().is_none());
        let item = store.claim_bulk_item(flags).await.unwrap().unwrap();
        store
            .acknowledge_bulk_flags(
                item.clone(),
                crate::bulk::Receipt::Flags {
                    before: Flags {
                        unread: None,
                        starred: Some(false),
                    },
                    after: Flags {
                        unread: None,
                        starred: Some(true),
                    },
                },
            )
            .await
            .unwrap();
        let job = store
            .finish_bulk_item(item, Ok(crate::bulk::Receipt::Unchanged))
            .await
            .unwrap();
        app.bulk_event(Event::BulkUpdate(Arc::new(job)));
        page(&mut app, &store).await;
        assert!(app.page.rows[0].starred);
        assert!(next(&mut commands).is_err());
        assert_eq!(app.mail_actions.pending(), 0);
        assert!(app.mail_actions.undo.is_empty());
    }
    #[tokio::test]
    async fn undo_failure_retains_retry_and_uses_the_same_receipt_without_reviving_dismissed_feedback()
     {
        let (mut app, mut commands, detail, store) = fixture_store().await;
        let original = detail.summary.clone();
        app.move_mail(original.clone(), "Trash".into());
        let id = admit(&mut app, &store, &mut commands).await;
        let item = store.claim_bulk_item(id.clone()).await.unwrap().unwrap();
        let forward = receipt(&original, "Trash", &original.account_id);
        moved(&mut app, &store, item, forward.clone()).await;
        app.undo_combined_actions(tokens(&app));
        undo(&mut app, &store, &mut commands).await;
        let inverse = store.claim_bulk_item(id.clone()).await.unwrap().unwrap();
        let failed = store
            .finish_bulk_item(inverse.clone(), Err(("Rejected reversal".into(), false)))
            .await
            .unwrap();
        app.bulk_event(Event::BulkUpdate(Arc::new(failed.clone())));
        page(&mut app, &store).await;
        assert_eq!(app.page.total, 0);
        assert_eq!(app.mail_actions.pending(), 0);
        assert!(app.action_toasts.current.is_none());
        assert_eq!(
            app.bulk
                .jobs
                .iter()
                .find(|job| job.id == id)
                .unwrap()
                .failed,
            1
        );
        app.action_toasts
            .expire(Instant::now() + std::time::Duration::from_secs(60));
        app.prune_undos();
        let retried = store.request_bulk_undo(id.clone()).await.unwrap();
        app.bulk_event(Event::BulkUpdate(Arc::new(retried)));
        page(&mut app, &store).await;
        assert_eq!(app.page.total, 1);
        let retry = store.claim_bulk_item(id.clone()).await.unwrap().unwrap();
        let Some(crate::bulk::Receipt::Move(saved)) = &retry.receipt else {
            panic!("retained receipt")
        };
        assert_eq!(
            saved
                .current
                .as_ref()
                .map(|mail| (&mail.id, &mail.remote_id)),
            forward
                .current
                .as_ref()
                .map(|mail| (&mail.id, &mail.remote_id))
        );
        app.bulk_event(Event::BulkUpdate(Arc::new(failed.clone())));
        app.notice = None;
        app.bulk_event(Event::BulkFinished(id.clone(), Ok(Arc::new(failed))));
        assert!(
            app.notice.is_none(),
            "A stale terminal result cannot override a newer retry"
        );
        assert_eq!(
            app.bulk
                .jobs
                .iter()
                .find(|job| job.id == id)
                .unwrap()
                .failed,
            0
        );
        app.action_toasts.current = None;
        let restored = receipt(&original, "INBOX", &original.account_id);
        moved(&mut app, &store, retry, restored).await;
        assert!(app.action_toasts.current.is_none());
        assert_eq!(app.page.total, 1);
        assert_eq!(app.mail_actions.pending(), 0);
    }
    #[tokio::test]
    async fn full_queue_rejects_undo_visibly_and_retry_dispatches_once_capacity_returns() {
        let (mut app, mut commands, detail, store) = fixture_store().await;
        app.move_mail(detail.summary.clone(), "Archive".into());
        let id = admit(&mut app, &store, &mut commands).await;
        let item = store.claim_bulk_item(id.clone()).await.unwrap().unwrap();
        moved(
            &mut app,
            &store,
            item,
            receipt(&detail.summary, "Archive", "fixture"),
        )
        .await;
        assert!(next(&mut commands).is_err());
        for _ in 0..32 {
            app.tx
                .as_ref()
                .unwrap()
                .try_send(Command::LoadImages(vec![]))
                .unwrap();
        }
        let restore = tokens(&app);
        app.undo_combined_actions(restore.clone());
        assert_eq!(app.page.total, 0);
        assert!(app.notice.as_ref().is_some_and(|(_, error, _)| *error));
        for _ in 0..32 {
            commands.try_recv().unwrap();
        }
        app.undo_combined_actions(restore.clone());
        assert_eq!(app.page.total, 1);
        assert_eq!(undo(&mut app, &store, &mut commands).await, id);
        app.undo_combined_actions(restore);
        assert!(next(&mut commands).is_err());
    }
    #[tokio::test]
    async fn grouped_undo_preserves_each_account_folder_and_does_not_steal_navigation() {
        let (mut app, mut commands, detail, store) = fixture_store().await;
        let parsed = parse_mail(
            "personal",
            "42.8",
            "Plans",
            b"From: personal@example.test\r\nSubject: Second\r\n\r\nBody".to_vec(),
            true,
            false,
        )
        .unwrap();
        let second = parsed.summary.clone();
        store.upsert(vec![parsed]).await.unwrap();
        let second_proof = store.detail(second.id.clone()).await.unwrap().lineage;
        app.move_mail(detail.summary.clone(), "Archive".into());
        app.admit_mail_move(second, None, "Archive".into(), second_proof);
        let group = tokens(&app);
        assert_eq!(group.len(), 2);
        for _ in 0..2 {
            let id = admit(&mut app, &store, &mut commands).await;
            let item = store.claim_bulk_item(id).await.unwrap().unwrap();
            let original = item.original.as_ref().unwrap();
            let forward = receipt(original, "Archive", &original.account_id);
            moved(&mut app, &store, item, forward).await;
        }
        app.query.folder = "Unrelated".into();
        app.set_mail_page(Arc::new(MailPage::default()));
        app.selected = Some("still-reading".into());
        app.undo_combined_actions(group);
        assert_eq!(app.page.total, 0);
        assert_eq!(app.selected.as_deref(), Some("still-reading"));
        let first_id = undo(&mut app, &store, &mut commands).await;
        let second_id = undo(&mut app, &store, &mut commands).await;
        let first = store
            .claim_bulk_item(first_id)
            .await
            .unwrap()
            .unwrap()
            .original
            .unwrap();
        let second = store
            .claim_bulk_item(second_id)
            .await
            .unwrap()
            .unwrap()
            .original
            .unwrap();
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
        let (mut app, mut commands, detail, store) = fixture_store().await;
        app.move_mail(detail.summary.clone(), "Archive".into());
        let id = admit(&mut app, &store, &mut commands).await;
        let item = store.claim_bulk_item(id).await.unwrap().unwrap();
        let receipt = receipt(&detail.summary, "Archive", "fixture");
        let current = receipt.current.as_ref().unwrap().clone();
        moved(&mut app, &store, item, receipt).await;
        app.query.folder = "Archive".into();
        page(&mut app, &store).await;
        app.selected = Some(current.id.clone());
        let generation = app.conversation.generation;
        app.undo_combined_actions(tokens(&app));
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
        let (mut app, mut commands, detail, store) = fixture_store().await;
        app.move_mail(detail.summary.clone(), "Archive".into());
        let id = admit(&mut app, &store, &mut commands).await;
        let item = store.claim_bulk_item(id.clone()).await.unwrap().unwrap();
        app.undo_combined_actions(tokens(&app));
        undo(&mut app, &store, &mut commands).await;
        let job = store
            .finish_bulk_item(item, Err(("Forward rejected".into(), false)))
            .await
            .unwrap();
        app.bulk_event(Event::BulkUpdate(Arc::new(job)));
        page(&mut app, &store).await;
        assert_eq!(app.page.total, 1);
        assert_eq!(app.mail_actions.pending(), 0);
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Restored 1 message"
        );
        assert!(store.claim_bulk_item(id).await.unwrap().is_none());
        assert!(next(&mut commands).is_err());
        assert!(app.mail_actions.undo.is_empty());
    }
}
