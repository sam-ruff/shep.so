use super::*;

impl App {
    /// Only deliberate list navigation arms a read. Startup selection, cached
    /// refreshes and neighbor preloading must not acknowledge unseen messages.
    pub(super) fn select_for_read(&mut self, id: String) {
        let Some(mail) = self.page.rows.iter().find(|mail| mail.id == id).cloned() else {
            return;
        };
        if self
            .mail_actions
            .read_candidate
            .as_ref()
            .is_none_or(|current| current.id != id)
        {
            self.finish_read();
            if !self.mail_actions.restoring(&mail.id)
                && !self.page.is_placeholder(&mail.id)
                && self.mail_actions.effective(&mail).unread
            {
                self.mail_actions.read_candidate = Some(mail);
            }
        }
        self.select(id);
    }

    pub(super) fn finish_read(&mut self) -> bool {
        let Some(candidate) = self.mail_actions.read_candidate.take() else {
            return true;
        };
        let mail = self
            .page
            .rows
            .iter()
            .find(|mail| mail.id == candidate.id)
            .or_else(|| {
                self.detail
                    .as_ref()
                    .map(|detail| &detail.summary)
                    .filter(|mail| mail.id == candidate.id)
            })
            .cloned()
            .unwrap_or(candidate);
        if self.mail_actions.effective(&mail).unread {
            // Uses the same immediate projection, coalescing and failure rollback
            // as the explicit read control. No body load is required.
            self.toggle_mail_flag(mail.clone(), true);
            return !self.mail_actions.effective(&mail).unread;
        }
        true
    }

    pub(super) fn read_navigation(&mut self, message: &Message) -> bool {
        let Some(candidate) = &self.mail_actions.read_candidate else {
            return true;
        };
        let leaves = matches!(
            message,
            Message::Tab(_)
                | Message::SettingsTab(_)
                | Message::Folder(_)
                | Message::SentFolder
                | Message::Account(_)
                | Message::AccountFolder(..)
                | Message::AccountFolderUnified
                | Message::Starred
                | Message::Filter(_)
                | Message::Sort(_)
                | Message::Query(_)
                | Message::NextPage(_)
                | Message::Draft(_)
                | Message::NewMessage
                | Message::Reply
                | Message::ReplyAll
                | Message::ClosePreview
                | Message::WindowClose(_)
                | Message::WindowUnfocused
                | Message::Focus("search", 0)
        ) || matches!(message, Message::MailContext(id, _) if id != &candidate.id);
        if leaves {
            let accepted = self.finish_read();
            if matches!(message, Message::WindowClose(_)) && !accepted {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_actions::Flags;

    async fn fixture() -> (App, tokio::sync::mpsc::Receiver<Command>, Vec<Mail>) {
        let store = crate::store::Store::memory().unwrap();
        let messages: Vec<_> = (1..=3)
            .map(|id| {
                parse_mail(
                    "work",
                    &id.to_string(),
                    "INBOX",
                    format!(
                        "From: friend@example.test\r\nSubject: Message {id}\r\n\r\nRead me {id}"
                    )
                    .into_bytes(),
                    true,
                    id == 1,
                )
                .unwrap()
            })
            .collect();
        store.upsert(messages).await.unwrap();
        let (sender, receiver) = engine::CommandSender::network_test_channel();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        app.query.folder = "INBOX".into();
        app.query.account = Some("work".into());
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        let messages = app.page.rows.clone();
        app.select(messages[0].id.clone());
        (app, receiver, messages)
    }
    fn flags(receiver: &mut tokio::sync::mpsc::Receiver<Command>) -> (u64, Mail, Flags) {
        match receiver.try_recv().unwrap() {
            Command::Flags(request, mail, flags) => (request, mail, flags),
            other => panic!("Expected read write, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn deliberate_selection_then_navigation_reads_only_the_previous_message() {
        let (mut app, mut receiver, mails) = fixture().await;
        let _ = app.handle(Message::Hover(mails[1].id.clone()));
        let _ = app.handle(Message::Tab(Tab::Calendar));
        assert!(
            receiver.try_recv().is_err(),
            "Startup selection and hover are not reading"
        );
        app.tab = Tab::Mail;
        let _ = app.handle(Message::Select(mails[0].id.clone()));
        assert!(
            receiver.try_recv().is_err(),
            "Wait until leaving the message"
        );
        let _ = app.handle(Message::Backend(Event::Changed));
        assert!(
            receiver.try_recv().is_err(),
            "Background refresh does not end reading"
        );
        let _ = app.handle(Message::Select(mails[1].id.clone()));
        let (request, mail, changes) = flags(&mut receiver);
        assert_eq!(mail.id, mails[0].id);
        assert_eq!(
            changes,
            Flags {
                unread: Some(false),
                starred: None
            }
        );
        assert!(
            !app.page.rows[0].unread,
            "Visual change precedes acknowledgment"
        );
        assert!(app.page.rows[1].unread);
        assert_eq!(app.page.unread, 2);
        assert_eq!(app.page.rows[0].starred, mails[0].starred);
        let _ = app.flags_finished(request, mail, Ok(()));
        assert!(receiver.try_recv().is_err());
        let _ = app.handle(Message::WindowUnfocused);
        let (_, mail, changes) = flags(&mut receiver);
        assert_eq!(mail.id, mails[1].id);
        assert_eq!(changes.unread, Some(false));
        assert_eq!(app.page.unread, 1);
    }

    #[tokio::test]
    async fn explicit_mark_unread_survives_leaving_and_old_acknowledgments() {
        let (mut app, mut receiver, mails) = fixture().await;
        app.select_for_read(mails[0].id.clone());
        app.toggle_mail_flag(mails[0].clone(), true);
        let (request, sent, _) = flags(&mut receiver);
        app.toggle_mail_flag(mails[0].clone(), true);
        assert!(app.mail_actions.read_candidate.is_none());
        app.select_for_read(mails[1].id.clone());
        let _ = app.flags_finished(request, sent, Ok(()));
        let (_, sent, change) = flags(&mut receiver);
        assert_eq!(sent.id, mails[0].id);
        assert_eq!(change.unread, Some(true));
        assert!(app.page.rows[0].unread);
    }

    #[tokio::test]
    async fn failed_read_rolls_back_without_changing_the_new_selection() {
        let (mut app, mut receiver, mails) = fixture().await;
        app.select_for_read(mails[0].id.clone());
        app.select_for_read(mails[1].id.clone());
        let (request, sent, _) = flags(&mut receiver);
        let _ = app.flags_finished(request, sent, Err("Fixture save failed".into()));
        assert!(app.page.rows[0].unread);
        assert_eq!(app.page.unread, 3);
        assert_eq!(app.selected.as_deref(), Some(mails[1].id.as_str()));
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .0
                .contains("Fixture save failed")
        );
    }

    #[tokio::test]
    async fn move_waits_for_read_and_uses_its_confirmed_flags_even_after_read_failure() {
        for success in [true, false] {
            let (mut app, mut receiver, mails) = fixture().await;
            app.select_for_read(mails[0].id.clone());
            app.move_mail(mails[0].clone(), "Archive".into());
            let (request, sent, _) = flags(&mut receiver);
            assert!(!app.page.rows.iter().any(|m| m.id == mails[0].id));
            assert!(
                receiver.try_recv().is_err(),
                "Read must finish before the UID moves"
            );
            let _ = app.flags_finished(
                request,
                sent,
                if success {
                    Ok(())
                } else {
                    Err("Read rejected".into())
                },
            );
            let Command::Move(_, mail, folder) = receiver.try_recv().unwrap() else {
                panic!("Move not dispatched")
            };
            assert_eq!(mail.id, mails[0].id);
            assert_eq!(folder, "Archive");
            assert_eq!(mail.unread, !success);
        }
    }

    #[tokio::test]
    async fn cross_account_move_waits_for_read_without_reusing_the_old_uid_after_transfer() {
        for success in [true, false] {
            let (mut app, mut receiver, mails) = fixture().await;
            app.select_for_read(mails[0].id.clone());
            app.transfer_mail(mails[0].clone(), "personal".into(), "INBOX".into());
            let (request, sent, _) = flags(&mut receiver);
            assert!(app.mail_actions.read_candidate.is_none());
            assert_ne!(app.selected.as_ref(), Some(&mails[0].id));
            assert!(receiver.try_recv().is_err());
            let _ = app.flags_finished(
                request,
                sent,
                if success {
                    Ok(())
                } else {
                    Err("Read rejected".into())
                },
            );
            let Command::Transfer(_, mail, account, folder) = receiver.try_recv().unwrap() else {
                panic!("Expected transfer")
            };
            assert_eq!(account, "personal");
            assert_eq!(folder, "INBOX");
            assert_eq!(mail.id, mails[0].id);
            assert_eq!(mail.unread, !success);
            assert!(receiver.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn exhausted_queue_keeps_read_unchanged_and_cancels_window_close() {
        let (mut app, mut receiver, mails) = fixture().await;
        for _ in 0..32 {
            app.tx
                .as_ref()
                .unwrap()
                .try_send(Command::LoadImages(vec![]))
                .unwrap();
        }
        app.select_for_read(mails[0].id.clone());
        assert!(!app.read_navigation(&Message::WindowClose(iced::window::Id::unique())));
        assert!(app.page.rows[0].unread);
        assert_eq!(app.mail_actions.pending(), 0);
        for _ in 0..32 {
            receiver.try_recv().unwrap();
        }
        assert!(receiver.try_recv().is_err());
    }
    #[tokio::test]
    async fn completing_read_preserves_a_flag_committed_since_selection() {
        let (mut app, mut receiver, mails) = fixture().await;
        app.select_for_read(mails[0].id.clone());
        app.toggle_mail_flag(mails[0].clone(), false);
        let (request, sent, _) = flags(&mut receiver);
        let _ = app.flags_finished(request, sent, Ok(()));
        let latest = app.page.rows[0].clone();
        assert_ne!(latest.starred, mails[0].starred);
        app.mail_actions.observe_detail(&latest);
        app.select_for_read(mails[1].id.clone());
        let (_, sent, changes) = flags(&mut receiver);
        assert_eq!(
            changes,
            Flags {
                unread: Some(false),
                starred: None
            }
        );
        assert_eq!(sent.starred, latest.starred);
        assert_eq!(app.page.rows[0].starred, latest.starred);
    }
}
