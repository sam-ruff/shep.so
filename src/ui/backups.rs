use super::*;
use secrecy::{ExposeSecret, SecretString};

pub(super) type ConnectionCheck = (u64, BackupTarget, Option<Result<(), String>>);

pub(super) enum BackupAction {
    Save(SecretString),
    ConnectS3(Option<(SecretString, SecretString)>),
    List,
    Restore(String, SecretString),
}

pub(super) struct PendingBackup {
    request: u64,
    target: BackupTarget,
    action: BackupAction,
}
impl App {
    pub(super) fn backup_validation_error(&mut self, error: impl Into<String>) {
        self.notice(error, true);
        self.preference_notice = self.notice.as_ref().map(|notice| notice.2);
    }

    pub(super) fn change_backup_destination(&mut self, id: Option<String>) {
        if let Err(error) = self.read_preferences() {
            self.backup_validation_error(error.to_string());
            return;
        }
        let result = match id {
            Some(id) => crate::backup::config::select(&mut self.preferences, &id),
            None => crate::backup::config::add(&mut self.preferences),
        };
        match result {
            Ok(()) => {
                self.settings_fields();
                self.fields.remove("passphrase");
                self.fields.remove("s3_access_secret");
                self.fields.remove("s3_key_secret");
                self.s3_connection = None;
                self.backups_generation += 1;
                self.save_preferences();
            }
            Err(error) => self.notice(error.to_string(), true),
        }
    }

    pub(super) fn backup_busy(&self) -> bool {
        self.busy
            .contains(&self.configured_backup_target().work_key())
    }

    pub(super) fn s3_form_settings(&self) -> crate::backup::s3::Settings {
        let mut settings = self.preferences.backup_s3.clone();
        if self.fields.contains_key("s3_endpoint") {
            settings.endpoint = self.field("s3_endpoint").trim().into();
            settings.region = self.field("s3_region").trim().into();
            settings.bucket = self.field("s3_bucket").trim().into();
            settings.prefix = self.field("s3_prefix").trim().into();
        }
        settings
    }

    pub(super) fn configured_backup_target(&self) -> BackupTarget {
        if self.preferences.backup_destination == BackupDestination::S3
            && self.tab == Tab::Preferences
        {
            return BackupTarget::S3(self.s3_form_settings().identity());
        }
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
            self.backup_validation_error("Use a backup passphrase of at least 12 characters.");
            return;
        }
        if let Err(error) = self.read_preferences() {
            self.backup_validation_error(error.to_string());
            return;
        }
        let target = self.configured_backup_target();
        if matches!(&target, BackupTarget::S3(_))
            && let Err(error) = self.preferences.backup_s3.validate()
        {
            self.backup_validation_error(error.to_string());
            return;
        }
        if matches!(target, BackupTarget::GoogleDrive { .. })
            && (self.preferences.google_lifecycle.disconnected
                || !self.preferences.google_grant.access.drive_allowed())
        {
            self.backup_validation_error(
                "Reconnect Google and approve Drive backup access before accessing copies.",
            );
            return;
        }
        if let BackupTarget::Local(path) = &target
            && !std::path::Path::new(path).is_absolute()
        {
            self.backup_validation_error("Choose an absolute backup folder path in Preferences.");
            return;
        }
        let request = self.preference_sync.changed();
        if self
            .preference_notice
            .take()
            .is_some_and(|at| self.notice.as_ref().is_some_and(|notice| notice.2 == at))
        {
            self.notice = None;
        }
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
            BackupAction::ConnectS3(secret) => {
                self.s3_connection = Some((request, pending.target.clone(), None));
                if !self.try_command(Command::ConnectS3(request, pending.target, secret)) {
                    self.s3_connection = None;
                }
            }
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

    #[tokio::test]
    async fn s3_setup_save_preserves_shared_settings_and_a_concurrent_backup_receipt() {
        let store = crate::store::Store::memory().unwrap();
        let (mut app, _) = App::new();
        app.tab = Tab::Preferences;
        app.preferences.backup_destination = BackupDestination::S3;
        app.preferences.backup_s3.bucket = "fixture-backups".into();
        let initial = store
            .save_preferences(app.preferences.clone())
            .await
            .unwrap();
        app.preference_sync = super::preference_sync::PreferenceSync::new(initial);
        app.settings_fields();
        let target = app.configured_backup_target();
        let (sender, mut saves) = engine::CommandSender::persistence_test_channel();
        app.tx = Some(sender);
        app.fields.insert("copies", "13".into());
        app.begin_backup_request(BackupAction::ConnectS3(None));
        let Command::SavePreferences(_, write) = saves.try_recv().unwrap() else {
            panic!("S3 setup must queue its preferences before testing the connection");
        };
        assert!(write.portable.tooltips.is_none());
        store
            .update_preferences(|p| p.tooltips = false)
            .await
            .unwrap();
        store
            .record_backup(target.clone(), 4567, true)
            .await
            .unwrap();
        let saved = store.save_preferences(write).await.unwrap().value;
        assert!(!saved.tooltips);
        assert_eq!(saved.backup_copies, 13);
        assert_eq!(saved.last_backup, Some(4567));
        assert!(saved.backup_ready);
        assert_eq!(BackupTarget::from_preferences(&saved), target);
    }

    #[test]
    fn s3_endpoint_changes_clear_unsaved_keys_and_old_connection_results() {
        let (mut app, _) = App::new();
        app.tab = Tab::Preferences;
        app.preferences.backup_destination = BackupDestination::S3;
        app.preferences.backup_s3.bucket = "fixture-backups".into();
        app.settings_fields();
        let target = app.configured_backup_target();
        app.fields.insert("s3_access_secret", "old-key".into());
        app.fields.insert("s3_key_secret", "old-secret".into());
        app.s3_connection = Some((1, target.clone(), None));
        let _ = app.handle(Message::Field(
            "s3_endpoint",
            "https://another.example.test".into(),
        ));
        assert!(app.field("s3_access_secret").is_empty() && app.field("s3_key_secret").is_empty());
        assert!(app.s3_connection.is_none());
        app.fields.insert("s3_key_secret", "new-secret".into());
        let _ = app.handle(Message::Backend(Event::S3Connection(1, target, Ok(()))));
        assert!(app.s3_connection.is_none());
        assert_eq!(app.field("s3_key_secret"), "new-secret");
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
