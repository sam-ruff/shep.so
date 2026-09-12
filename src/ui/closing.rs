//! Close intent survives independent saves. Resume only once every required
//! acknowledgment is present; read-only refreshes never hold the window open.
use super::*;

impl App {
    pub(super) fn schedule_pending_move_recovery(&mut self) {
        if self.pending_close.is_some()
            || self.tray.exiting
            || self.profiles.changing()
            || self.database_import.pending()
        {
            return;
        }
        self.try_command(Command::RecoverPendingMoves);
    }

    pub(super) fn has_required_close_work(&self) -> bool {
        self.profile_sync.pending()
            || self.profiles.changing()
            || self.database_import.pending()
            || self.database_transfer.pending.is_some()
            || self.folder_staging()
            || self.folder_creation.busy
            || self.bulk.staging.is_some()
            || self.mail_actions.pending() > 0
            || !self.move_recovery.pending.is_empty()
            || self.removal.removing.is_some()
            || self.calendar_setup.saving.is_some()
            || self.composer.discard_pending
            || self.composer.forward_pending.is_some()
            || self.composer.io.is_some()
            || self.composer.pending()
            || self.preference_sync.dirty()
            || self.busy.iter().any(|key| self.required_busy(key))
    }

    /// Upload journals and credential cleanup resume on the next launch, so an
    /// explicit Quit may leave them; every other busy write must acknowledge.
    fn journaled_busy(key: &str) -> bool {
        key.starts_with("backup:") || key == "credential-cleanup"
    }

    pub(super) fn required_busy(&self, key: &str) -> bool {
        if Self::journaled_busy(key) {
            return !self.tray.insisted;
        }
        matches!(
            key,
            "google-disconnect" | "google" | "pending-move-recovery"
        ) || key.starts_with("outgoing:")
            || key.starts_with("send:")
            || key.starts_with("event:")
            || key.starts_with("account:")
    }

    pub(super) fn continue_pending_close(&mut self) -> Task<Message> {
        let Some(window) = self.pending_close else {
            return Task::none();
        };
        if self.has_required_close_work() || (self.tx.is_some() && !self.bulk.stopped) {
            return Task::none();
        }
        self.pending_close = None;
        self.handle(Message::WindowClose(window))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_move_recovery_is_coalesced_and_close_waits_from_admission() {
        let (mut app, _) = App::new();
        let (sender, _selection, mut network) = engine::CommandSender::close_test_channels();
        app.tx = Some(sender);
        app.bulk.stopped = true;
        app.schedule_pending_move_recovery();
        app.schedule_pending_move_recovery();
        assert!(matches!(
            network.try_recv().expect("admitted"),
            Command::RecoverPendingMoves
        ));
        assert!(network.try_recv().is_err(), "duplicate is coalesced");
        let window = iced::window::Id::unique();
        let _ = app.handle(Message::WindowClose(window));
        assert_eq!(app.pending_close, Some(window));
        app.schedule_pending_move_recovery();
        assert!(
            network.try_recv().is_err(),
            "closing cannot admit another write"
        );
        let _ = app.update(Message::Backend(Event::Busy(
            "pending-move-recovery".into(),
            false,
        )));
        assert!(app.pending_close.is_none());
    }

    #[test]
    fn every_durable_busy_path_keeps_close_intent_and_resumes_after_ack() {
        for key in [
            "credential-cleanup",
            "google-disconnect",
            "google",
            "pending-move-recovery",
            "outgoing:one",
            "send:one",
            "event:one",
            "account:one",
            "backup:one",
        ] {
            let (mut app, _) = App::new();
            let window = iced::window::Id::unique();
            app.busy.insert(key.into());
            let _ = app.handle(Message::WindowClose(window));
            assert_eq!(app.pending_close, Some(window), "{key}");
            let _ = app.continue_pending_close();
            assert_eq!(app.pending_close, Some(window), "still pending: {key}");
            let _ = app.update(Message::Backend(Event::Busy(key.into(), false)));
            assert!(app.pending_close.is_none(), "did not continue: {key}");
        }
    }

    #[test]
    fn queued_account_save_holds_close_before_worker_busy_and_rejects_duplicate_admission() {
        let (mut app, _) = App::new();
        let (sender, _selection, mut network) = engine::CommandSender::close_test_channels();
        app.tx = Some(sender);
        app.bulk.stopped = true;
        let account: Account = serde_json::from_value(serde_json::json!({
            "id":"queued", "name":"Fixture", "email":"fixture@example.test",
            "protocol":"Imap", "host":"imap.example.test", "port":993,
            "username":"fixture", "smtp_host":"smtp.example.test", "smtp_port":465
        }))
        .unwrap();
        assert!(app.try_command(Command::SaveAccount(account.clone(), "".into(), "".into())));
        assert!(!app.try_command(Command::SaveAccount(account, "".into(), "".into())));
        assert!(matches!(
            network.try_recv().unwrap(),
            Command::SaveAccount(..)
        ));
        assert!(network.try_recv().is_err());
        let window = iced::window::Id::unique();
        let _ = app.handle(Message::WindowClose(window));
        assert_eq!(app.pending_close, Some(window));
        let _ = app.update(Message::Backend(Event::Busy(
            "account:queued".into(),
            false,
        )));
        assert!(app.pending_close.is_none());
    }

    #[tokio::test]
    async fn close_continues_through_calendar_save_and_bulk_ack_in_either_order() {
        for save_first in [true, false] {
            let (mut app, _) = App::new();
            let (sender, mut commands, _network) = engine::CommandSender::close_test_channels();
            app.tx = Some(sender);
            let window = iced::window::Id::unique();
            app.calendar_setup.saving = Some(7);
            app.calendar_setup.generation = 7;
            let _ = app.update(Message::WindowClose(window));
            assert_eq!(app.pending_close, Some(window));
            assert!(matches!(commands.try_recv().unwrap(), Command::BulkStop));
            if save_first {
                let _ = app.update(Message::Backend(Event::CalendarsConnected(7, Ok(()))));
                assert_eq!(app.pending_close, Some(window));
                let _ = app.update(Message::Backend(Event::BulkStopped));
            } else {
                let _ = app.update(Message::Backend(Event::BulkStopped));
                assert_eq!(app.pending_close, Some(window));
                let _ = app.update(Message::Backend(Event::CalendarsConnected(7, Ok(()))));
            }
            assert!(app.pending_close.is_none());
            assert!(
                commands.try_recv().is_err(),
                "Stop must not be queued twice"
            );
        }
    }

    #[tokio::test]
    async fn failed_calendar_form_save_cancels_close_before_late_stop_ack() {
        let (mut app, _) = App::new();
        let (sender, mut commands, _network) = engine::CommandSender::close_test_channels();
        app.tx = Some(sender);
        app.dialog = Some(Dialog::Calendar);
        app.calendar_setup.generation = 4;
        app.calendar_setup.saving = Some(4);
        let _ = app.update(Message::WindowClose(iced::window::Id::unique()));
        assert!(matches!(commands.try_recv().unwrap(), Command::BulkStop));
        let _ = app.update(Message::Backend(Event::CalendarsConnected(
            4,
            Err("Credentials unavailable".into()),
        )));
        assert!(app.pending_close.is_none());
        assert_eq!(
            app.calendar_setup.error.as_deref(),
            Some("Credentials unavailable")
        );
        let _ = app.update(Message::Backend(Event::BulkStopped));
        assert!(app.pending_close.is_none());
        assert_eq!(app.dialog, Some(Dialog::Calendar));
    }

    #[test]
    fn close_waits_for_all_dependencies_and_preserves_new_errors() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.busy.extend(["send:one".into(), "event:two".into()]);
        let _ = app.handle(Message::WindowClose(window));
        let _ = app.update(Message::Backend(Event::Busy("send:one".into(), false)));
        assert_eq!(app.pending_close, Some(window));
        let _ = app.update(Message::Backend(Event::Error(
            "Calendar save failed; retry it.".into(),
        )));
        assert!(app.pending_close.is_none());
        let _ = app.update(Message::Backend(Event::Busy("event:two".into(), false)));
        assert!(app.pending_close.is_none());
        assert!(app.notice.as_ref().unwrap().1);
    }

    #[test]
    fn insisted_quit_keeps_local_saves_but_releases_journaled_uploads() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.busy
            .extend(["backup:one".into(), "credential-cleanup".into()]);
        app.composer.io = Some("draft".into());
        let _ = app.handle(Message::WindowClose(window));
        assert_eq!(app.pending_close, Some(window));
        let _ = app.quit_now(window);
        assert_eq!(
            app.pending_close,
            Some(window),
            "an attachment write is not journaled"
        );
        assert!(app.notice.as_ref().unwrap().0.contains("attachments"));
        app.composer.io = None;
        let _ = app.continue_pending_close();
        assert!(
            app.pending_close.is_none(),
            "journaled uploads no longer hold exit"
        );
        assert!(app.tray.exiting);
        assert!(
            app.busy.contains("backup:one"),
            "abandonment is not a fake ack"
        );
    }

    #[test]
    fn a_new_close_after_cancelled_quit_waits_for_journaled_uploads_again() {
        let (mut app, _) = App::new();
        let window = iced::window::Id::unique();
        app.busy.insert("backup:one".into());
        let _ = app.handle(Message::WindowClose(window));
        let _ = app.update(Message::Backend(Event::Error("Upload failed".into())));
        assert!(app.pending_close.is_none());
        let _ = app.quit_now(window);
        assert!(app.tray.exiting, "an explicit Quit still leaves");

        let (mut app, _) = App::new();
        app.busy.insert("backup:one".into());
        let _ = app.quit_now(window);
        assert!(
            app.tray.exiting,
            "Quit with no prior close still leaves journaled work"
        );

        let (mut app, _) = App::new();
        app.busy.insert("send:one".into());
        let _ = app.quit_now(window);
        assert!(!app.tray.exiting);
        assert_eq!(app.pending_close, Some(window));
        let _ = app.update(Message::Backend(Event::Error("Send failed".into())));
        assert!(app.pending_close.is_none());
        app.busy.insert("backup:two".into());
        let _ = app.handle(Message::WindowClose(window));
        assert_eq!(
            app.pending_close,
            Some(window),
            "an ordinary close waits for the upload again"
        );
    }

    #[test]
    fn optional_refresh_does_not_hold_close_and_attachment_completion_resumes_it() {
        let (mut app, _) = App::new();
        app.busy.extend(["sync".into(), "calendar".into()]);
        let window = iced::window::Id::unique();
        app.composer.io = Some("draft".into());
        let _ = app.handle(Message::WindowClose(window));
        assert_eq!(app.pending_close, Some(window));
        app.composer.io = None;
        let _ = app.continue_pending_close();
        assert!(app.pending_close.is_none());
        assert!(app.busy.contains("sync"));
    }
}
