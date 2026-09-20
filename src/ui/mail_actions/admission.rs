use super::*;
use crate::bulk::{Action, Job};

pub(super) struct Admission {
    pub id: String,
    pub original: Mail,
    pub action: Action,
    pub revision: Option<u64>,
    pub origin: MailQuery,
    position: Option<usize>,
}

impl App {
    pub(in crate::ui) fn admit_mail_move(
        &mut self,
        mail: Mail,
        account: Option<String>,
        folder: String,
        lineage: Option<String>,
    ) {
        if self.move_is_blocked(&mail.id)
            || self.mail_actions.restoring(&mail.id)
            || (account.as_ref().is_none_or(|a| a == &mail.account_id) && mail.folder == folder)
        {
            return;
        }
        let neighbours = self.removal_neighbors(&mail.id);
        if self
            .mail_actions
            .read_candidate
            .as_ref()
            .is_some_and(|m| m.id == mail.id)
        {
            self.finish_read();
        }
        let id = mail.id.clone();
        if !self.admit_mail_action_with_lineage(mail, Action::Move { account, folder }, lineage) {
            return;
        }
        self.dialog = None;
        self.focused_input = None;
        self.pending_focus = None;
        self.select_after_removal(&id, neighbours);
        self.refill_short_page();
    }

    pub(in crate::ui) fn mail_input_lineage(&self, mail: &Mail) -> Option<String> {
        if matches!(self.dialog, Some(Dialog::Move | Dialog::MoveConfirm))
            && let Some((original, lineage)) = &self.mail_actions.move_review
            && original.id == mail.id
        {
            return lineage.clone();
        }
        if let Some(detail) = self
            .detail
            .as_ref()
            .filter(|detail| detail.summary.id == mail.id)
        {
            return detail.lineage.clone();
        }
        if self
            .conversation
            .page
            .rows
            .iter()
            .any(|row| row.id == mail.id)
        {
            return self.conversation.page.lineages.get(&mail.id).cloned();
        }
        self.page.lineages.get(&mail.id).cloned()
    }

    pub(in crate::ui) fn admit_mail_action_with_lineage(
        &mut self,
        mail: Mail,
        action: Action,
        lineage: Option<String>,
    ) -> bool {
        if self.mail_actions.admissions.len() >= CHANNEL_CAPACITY
            || self.mail_actions.journal_jobs.len() >= CHANNEL_CAPACITY
        {
            self.notice(
                "The local action queue is full. Please retry shortly.",
                true,
            );
            return false;
        }
        if self.bulk.stopped || self.bulk.stop_requested {
            if !self.try_command(Command::BulkResume(String::new())) {
                return false;
            }
            self.bulk.stopped = false;
            self.bulk.stop_requested = false;
        }
        let id = uuid::Uuid::new_v4().to_string();
        if !self.try_command(Command::AdmitMail(
            id.clone(),
            mail.clone(),
            action.clone(),
            lineage,
        )) {
            return false;
        }
        self.remember_individual_job(&id, &mail, &action);
        self.mail_actions.journal_jobs.insert(id.clone());
        let position = self
            .page
            .rows
            .iter()
            .position(|row| row.id == mail.id)
            .or_else(|| {
                self.mail_actions
                    .admissions
                    .iter()
                    .find(|entry| entry.original.id == mail.id)
                    .and_then(|entry| entry.position)
            });
        self.mail_actions.admissions.push_back(Admission {
            id,
            original: mail,
            action,
            revision: None,
            origin: self.query.clone(),
            position,
        });
        self.invalidate_action_snapshot();
        self.project_mail_flags();
        true
    }

    pub(in crate::ui) fn mail_admitted(&mut self, id: String, result: Result<Arc<Job>, String>) {
        if result.is_err()
            && (!self.mail_actions.journal_jobs.contains(&id)
                || self
                    .mail_actions
                    .admissions
                    .iter()
                    .any(|entry| entry.id == id && entry.revision.is_some())
                || self.mail_actions.base_page.bulk_observed.contains_key(&id))
        {
            return;
        }
        let previous_error = self.notice.clone().filter(|(_, error, _)| *error);
        let mut failure = None;
        match result {
            Ok(job) => {
                if let Some(entry) = self.mail_actions.admissions.iter_mut().find(|e| e.id == id) {
                    entry.revision = Some(job.revision);
                }
                self.individual_job_admitted(job);
                self.send(Command::BulkRun(id));
            }
            Err(error) => {
                self.mail_actions.journal_jobs.remove(&id);
                self.mail_actions.admissions.retain(|e| e.id != id);
                self.individual_job_rejected(&id);
                self.pending_close = None;
                self.project_mail_flags();
                failure = Some(format!("This change was not saved. {error}"));
            }
        }
        self.request_page();
        if let Some(failure) = failure {
            self.notice(failure, true);
        } else if let Some(previous_error) = previous_error {
            self.notice = Some(previous_error);
        }
    }

    pub(in crate::ui) fn retire_individual_projection(&mut self, id: &str) {
        self.mail_actions.admissions.retain(|entry| entry.id != id);
    }

    pub(in crate::ui) fn observe_individual_job(&mut self, job: &Job) {
        if job.remaining == 0 && job.running == 0 {
            self.mail_actions.journal_jobs.remove(&job.id);
            if job.failed > 0 && job.uncertain == 0 {
                self.retire_individual_projection(&job.id);
                self.project_mail_flags();
            }
        }
    }

    pub(in crate::ui) fn individual_pending(&self) -> usize {
        self.mail_actions.pending()
            + self
                .mail_actions
                .journal_jobs
                .iter()
                .filter(|id| {
                    !self
                        .mail_actions
                        .admissions
                        .iter()
                        .any(|entry| &entry.id == *id && entry.revision.is_none())
                })
                .count()
    }

    pub(in crate::ui) fn individual_observed_jobs(&self) -> impl Iterator<Item = &String> {
        self.mail_actions.admissions.iter().map(|entry| &entry.id)
    }

    pub(in crate::ui) fn project_mail_admissions(&mut self) {
        let mut page = (*self.page).clone();
        self.mail_actions.admissions.retain(|entry| {
            !page.bulk_observed.contains_key(&entry.id)
                && entry
                    .revision
                    .is_none_or(|revision| page.bulk_revision < revision)
        });
        let mut current = HashMap::<String, Mail>::new();
        for entry in &self.mail_actions.admissions {
            let original = &entry.original;
            let before = current
                .entry(original.id.clone())
                .or_insert_with(|| {
                    let mut mail = page
                        .rows
                        .iter()
                        .find(|m| m.id == original.id)
                        .cloned()
                        .unwrap_or_else(|| original.clone());
                    if let Some(Some(state)) = page.observed.get(&original.id) {
                        mail.account_id.clone_from(&state.account);
                        mail.folder.clone_from(&state.folder);
                        mail.unread = state.unread;
                        mail.starred = state.starred;
                    }
                    mail
                })
                .clone();
            let mut after = before.clone();
            entry.action.apply(&mut after);
            let visible = |mail: &Mail| {
                self.bulk_scope_contains(&self.query, &mail.account_id, &mail.folder)
                    && !(self.query.unread_only && !mail.unread
                        || self.query.read_only && mail.unread
                        || self.query.starred_only && !mail.starred)
            };
            let same_search = self.query.search == entry.origin.search;
            let before_visible = same_search && visible(&before);
            let after_visible = same_search && visible(&after);
            adjust_count(&mut page.total, before_visible, after_visible);
            adjust_count(
                &mut page.unread,
                before_visible && before.unread,
                after_visible && after.unread,
            );
            counts::adjust(
                &mut page.inbox_unread,
                Some(&MailMembership::from(&before)),
                Some(&MailMembership::from(&after)),
            );
            page.rows
                .retain(|mail| mail.id != original.id || after_visible);
            if let Some(row) = page.rows.iter_mut().find(|mail| mail.id == original.id) {
                *row = after.clone();
            } else if after_visible
                && before_visible != after_visible
                && page.rows.len() < PAGE_SIZE
            {
                let position = entry
                    .position
                    .unwrap_or(page.rows.len())
                    .min(page.rows.len());
                page.rows.insert(position, after.clone());
            }
            if matches!(entry.action, Action::Move { .. }) {
                page.bulk_placeholders.insert(original.id.clone());
            }
            page.observed
                .insert(original.id.clone(), Some(MailMembership::from(&after)));
            current.insert(original.id.clone(), after);
        }
        self.page = Arc::new(page);
    }
}

fn adjust_count(count: &mut usize, before: bool, after: bool) {
    if before && !after {
        *count = count.saturating_sub(1);
    } else if !before && after {
        *count = count.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    async fn fixture() -> (App, Store, tokio::sync::mpsc::Receiver<Command>, Mail) {
        let store = Store::memory().expect("store");
        let mail = parse_mail(
            "work",
            "1",
            "INBOX",
            b"From: fixture@example.test\r\nSubject: One\r\n\r\nBody".to_vec(),
            true,
            false,
        )
        .expect("mail");
        store.upsert(vec![mail]).await.expect("saved");
        let (sender, receiver) = engine::CommandSender::calendar_test_channel();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        app.query.folder = "INBOX".into();
        app.set_mail_page(Arc::new(
            store.query(app.query.clone()).await.expect("page"),
        ));
        let original = app.page.rows[0].clone();
        (app, store, receiver, original)
    }

    fn admission(receiver: &mut tokio::sync::mpsc::Receiver<Command>) -> (String, Mail, Action) {
        match receiver.try_recv().expect("admission command") {
            Command::AdmitMail(id, original, action, _) => (id, original, action),
            other => panic!("Expected admission, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn every_field_edit_and_move_enters_fifo_before_earlier_acknowledgement() {
        let (mut app, _, mut receiver, original) = fixture().await;
        app.toggle_mail_flag(original.clone(), true);
        app.toggle_mail_flag(original.clone(), false);
        app.move_mail(original.clone(), "Archive".into());
        assert_eq!((app.page.total, app.page.unread), (0, 0));
        assert_eq!(app.page.inbox_unread.get("work").copied().unwrap_or(0), 0);
        assert_eq!(app.mail_actions.pending(), 3);
        let (read, _, read_action) = admission(&mut receiver);
        let (flag, _, flag_action) = admission(&mut receiver);
        let (moved, source, move_action) = admission(&mut receiver);
        assert_ne!(read, flag);
        assert_ne!(flag, moved);
        assert_eq!(
            read_action,
            Action::Flags(Flags {
                unread: Some(false),
                starred: None
            })
        );
        assert_eq!(
            flag_action,
            Action::Flags(Flags {
                unread: None,
                starred: Some(true)
            })
        );
        assert_eq!(
            move_action,
            Action::Move {
                account: None,
                folder: "Archive".into()
            }
        );
        assert_eq!(source.remote_id, original.remote_id);
    }

    #[tokio::test]
    async fn older_admission_failure_preserves_newer_same_field_and_other_field() {
        let (mut app, _, mut receiver, original) = fixture().await;
        app.toggle_mail_flag(original.clone(), false);
        let (older, _, _) = admission(&mut receiver);
        app.toggle_mail_flag(original.clone(), false);
        let (newer, _, _) = admission(&mut receiver);
        app.toggle_mail_flag(original.clone(), true);
        app.mail_admitted(older, Err("Fixture admission failed".into()));
        assert!(!app.page.rows[0].starred);
        assert!(!app.page.rows[0].unread);
        assert_eq!(app.page.inbox_unread.get("work").copied().unwrap_or(0), 0);
        assert!(
            app.mail_actions
                .admissions
                .iter()
                .any(|entry| entry.id == newer)
        );
        assert!(
            app.notice
                .as_ref()
                .expect("failure")
                .0
                .contains("Fixture admission failed")
        );
    }

    #[tokio::test]
    async fn saved_projection_retires_overlay_without_count_drift() {
        let (mut app, store, mut receiver, original) = fixture().await;
        app.toggle_mail_flag(original, true);
        let (id, original, action) = admission(&mut receiver);
        let job = store
            .start_individual_mail_action(id.clone(), original, action)
            .await
            .expect("admitted");
        app.mail_admitted(id, Ok(Arc::new(job)));
        assert_eq!(app.page.unread, 0);
        let mut query = app.query.clone();
        query.observe_bulk = app.bulk_observed_ids();
        app.set_mail_page(Arc::new(store.query(query).await.expect("projected")));
        assert_eq!(app.page.unread, 0);
        assert_eq!(app.page.inbox_unread.get("work").copied().unwrap_or(0), 0);
        assert!(app.mail_actions.admissions.is_empty());
        app.toggle_mail_flag(app.page.rows[0].clone(), true);
        assert!(
            app.page.rows[0].unread,
            "Durable pending flags must allow a newer field edit"
        );
    }

    #[tokio::test]
    async fn undo_before_individual_admission_reply_is_durable_and_immediate() {
        let (mut app, store, mut receiver, original) = fixture().await;
        app.move_mail(original, "Archive".into());
        let token = app
            .action_toasts
            .current
            .as_ref()
            .expect("toast")
            .undo_tokens()[0];
        app.undo_combined_actions(vec![token]);
        assert_eq!((app.page.total, app.page.unread), (1, 1));
        let (id, original, action) = admission(&mut receiver);
        let Command::BulkUndo(undo) = receiver.try_recv().expect("ordered Undo") else {
            panic!("Expected journal Undo")
        };
        assert_eq!(id, undo);
        store
            .start_individual_mail_action(id.clone(), original, action)
            .await
            .expect("forward admission");
        let job = store.request_bulk_undo(id).await.expect("durable Undo");
        assert_eq!(job.cancelled, 1);
        assert_eq!(job.running, 0);
    }

    #[tokio::test]
    async fn reader_and_row_controls_keep_their_own_identity_observation() {
        let (mut app, store, mut receiver, original) = fixture().await;
        let older = Arc::new(
            store
                .detail(original.id.clone())
                .await
                .expect("original detail"),
        );
        let old_lineage = older.lineage.clone();
        let replaced = original.id.clone();
        store
            .run(move |connection| {
                connection.execute(
                    "UPDATE messages SET raw=? WHERE id=?",
                    rusqlite::params![
                        b"From: fixture@example.test\r\nSubject: One\r\n\r\nReplacement body"
                            .as_slice(),
                        replaced
                    ],
                )?;
                Ok(())
            })
            .await
            .expect("replaced");
        app.set_mail_page(Arc::new(
            store.query(app.query.clone()).await.expect("new page"),
        ));
        app.selected = Some(original.id.clone());
        app.detail = Some(older);
        let _ = app.handle(Message::ToggleStar);
        let Command::AdmitMail(id, mail, action, lineage) =
            receiver.try_recv().expect("reader intent")
        else {
            panic!("reader admission")
        };
        assert_eq!(lineage, old_lineage);
        let result = store
            .start_observed_mail_action(id.clone(), mail, action, lineage.expect("old proof"))
            .await;
        assert!(
            result.is_err(),
            "A replacement must reject the old reader proof"
        );
        app.mail_admitted(id, result.map(Arc::new).map_err(|error| error.to_string()));
        assert!(!app.page.rows[0].starred);
        let _ = app.handle(Message::FlagRow(original.id.clone()));
        let Command::AdmitMail(id, mail, action, lineage) =
            receiver.try_recv().expect("row intent")
        else {
            panic!("row admission")
        };
        assert_ne!(lineage, old_lineage);
        store
            .start_observed_mail_action(id, mail, action, lineage.expect("row proof"))
            .await
            .expect("row owns replacement observation");
    }

    #[tokio::test]
    async fn read_navigation_keeps_proof_captured_when_the_message_was_read() {
        let (mut app, _, mut receiver, original) = fixture().await;
        let lineage = app.page.lineages[&original.id].clone();
        app.select_for_read(original.id.clone());
        Arc::make_mut(&mut app.page)
            .lineages
            .insert(original.id.clone(), "replacement".into());
        app.finish_read();
        let Command::AdmitMail(_, _, _, proof) = receiver.try_recv().expect("read intent") else {
            panic!("read admission")
        };
        assert_eq!(proof, Some(lineage));
    }
}
