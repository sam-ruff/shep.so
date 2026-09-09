use super::*;
use secrecy::{ExposeSecret, SecretString};

pub(super) enum BackupAction {
    Save(SecretString),
    List,
    Restore(String, SecretString),
}

pub(super) struct PendingBackup {
    request: u64,
    target: BackupTarget,
    action: BackupAction,
}
impl App {
    pub(super) fn configured_backup_target(&self) -> BackupTarget {
        if self.preferences.backup_destination == BackupDestination::Local
            && self.tab == Tab::Preferences
            && self.fields.contains_key("backup_folder")
        {
            return BackupTarget::Local(self.field("backup_folder").into());
        }
        BackupTarget::from_preferences(&self.preferences)
    }

    pub(super) fn visible_backups(&self) -> &[BackupCopy] {
        if self.preferences.backup_destination == BackupDestination::GoogleDrive
            && (self.preferences.google_lifecycle.disconnected
                || !self.preferences.google_grant.access.drive_allowed())
        {
            return &[];
        }
        if self.backups_target.as_ref() == Some(&self.configured_backup_target()) {
            &self.backups
        } else {
            &[]
        }
    }

    pub(super) fn begin_backup_request(&mut self, action: BackupAction) {
        if let BackupAction::Save(secret) = &action
            && secret.expose_secret().chars().count() < 12
        {
            self.notice("Use a backup passphrase of at least 12 characters.", true);
            return;
        }
        if let Err(error) = self.read_preferences() {
            self.notice(error.to_string(), true);
            return;
        }
        let target = self.configured_backup_target();
        if matches!(target, BackupTarget::GoogleDrive { .. })
            && (self.preferences.google_lifecycle.disconnected
                || !self.preferences.google_grant.access.drive_allowed())
        {
            self.notice(
                "Reconnect Google and approve Drive backup access before accessing copies.",
                true,
            );
            return;
        }
        if let BackupTarget::Local(path) = &target
            && !std::path::Path::new(path).is_absolute()
        {
            self.notice(
                "Choose an absolute backup folder path in Preferences.",
                true,
            );
            return;
        }
        let request = self.preference_sync.changed();
        self.pending_backup = Some(PendingBackup {
            request,
            target,
            action,
        });
        if !self.queue_preference_write(request, self.preferences.clone()) {
            self.pending_backup = None;
        }
    }

    pub(super) fn cancel_backup_save(&mut self, request: u64) {
        if self
            .pending_backup
            .as_ref()
            .is_some_and(|p| p.request == request)
        {
            self.pending_backup = None;
        }
    }

    pub(super) fn continue_backup_request(&mut self, request: u64) {
        if self
            .pending_backup
            .as_ref()
            .is_none_or(|pending| pending.request != request)
        {
            return;
        }
        let Some(pending) = self.pending_backup.take() else {
            return;
        };
        if pending.target != self.configured_backup_target() {
            self.notice(
                "The backup destination changed. Start the action again with the current settings.",
                true,
            );
            return;
        }
        match pending.action {
            BackupAction::Save(secret) => self.send(Command::Backup(pending.target, secret)),
            BackupAction::List => self.request_backup_copies(pending.target),
            BackupAction::Restore(id, secret) => {
                self.send(Command::Restore(pending.target, id, secret))
            }
        }
    }

    pub(super) fn request_backup_copies(&mut self, target: BackupTarget) {
        self.backups_generation += 1;
        self.send(Command::ListBackups(self.backups_generation, target));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copy(id: &str) -> BackupCopy {
        BackupCopy {
            id: id.into(),
            name: id.into(),
            created_at: String::new(),
        }
    }

    #[test]
    fn stale_lists_and_commits_cannot_populate_another_destination() {
        let (mut app, _) = App::new();
        app.tab = Tab::Preferences;
        app.preferences.backup_folder = "/second".into();
        app.settings_fields();
        app.backups_generation = 2;
        let current = app.configured_backup_target();
        let _ = app.handle(Message::Backend(Event::Backups(
            2,
            current.clone(),
            Ok(Arc::new(vec![copy("current")])),
        )));
        for event in [
            Event::Backups(1, current.clone(), Ok(Arc::new(vec![copy("older-list")]))),
            Event::Backups(
                2,
                BackupTarget::Local("/first".into()),
                Ok(Arc::new(vec![copy("other-folder")])),
            ),
            Event::BackupSaved(BackupTarget::Local("/first".into()), copy("other-copy")),
            Event::Backups(1, current.clone(), Err("old listing failed".into())),
        ] {
            let _ = app.handle(Message::Backend(event));
        }
        assert_eq!(app.visible_backups()[0].id, "current");
        assert!(app.notice.is_none());
        let _ = app.handle(Message::Field("backup_folder", "/third".into()));
        assert!(app.visible_backups().is_empty());
        let _ = app.handle(Message::Restore("current".into()));
        assert_eq!(app.dialog, None);
        assert!(app.notice.as_ref().unwrap().0.contains("Refresh"));
    }

    #[test]
    fn a_committed_copy_invalidates_an_older_inflight_listing() {
        let (mut app, _) = App::new();
        let target = app.configured_backup_target();
        app.backups_generation = 1;
        let _ = app.handle(Message::Backend(Event::BackupSaved(
            target.clone(),
            copy("saved"),
        )));
        let _ = app.handle(Message::Backend(Event::Backups(
            1,
            target,
            Ok(Arc::new(Vec::new())),
        )));
        assert_eq!(app.visible_backups()[0].id, "saved");
    }

    #[test]
    fn backup_action_waits_for_its_save_and_cancels_when_the_destination_changes() {
        let (mut app, _) = App::new();
        let (sender, _receiver) = engine::CommandSender::persistence_test_channel();
        app.tx = Some(sender);
        app.tab = Tab::Preferences;
        app.settings_fields();
        let first = std::env::temp_dir()
            .join("shep-first")
            .to_string_lossy()
            .to_string();
        app.fields.insert("backup_folder", first.clone());
        app.begin_backup_request(BackupAction::Save(SecretString::from("a saved passphrase")));
        let request = app.pending_backup.as_ref().unwrap().request;
        app.continue_backup_request(request + 1);
        assert!(app.pending_backup.is_some());
        app.fields.insert("backup_folder", format!("{first}-other"));
        app.continue_backup_request(request);
        assert!(app.pending_backup.is_none());
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .0
                .contains("destination changed")
        );
    }
}
