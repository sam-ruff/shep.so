use super::*;
use secrecy::{ExposeSecret, SecretString};

pub(super) type ConnectionCheck = (u64, BackupTarget, Option<Result<(), String>>);

pub(super) struct HostKeyReview {
    pub request: u64,
    pub settings: crate::backup::sftp::Settings,
    pub result: Option<Result<String, String>>,
    pub verified: bool,
}

pub(super) enum BackupAction {
    Save(SecretString),
    ResumeHistory(String, SecretString),
    ConnectS3(Option<(SecretString, SecretString)>),
    ConnectSftp(Option<SecretString>),
    ConnectFtp(Option<SecretString>),
    List,
    Restore(String, SecretString),
}

#[derive(Debug, Clone, serde::Serialize)]
pub(super) struct RunRow {
    pub id: String,
    pub name: String,
    pub target: BackupTarget,
    pub request: u64,
    pub status: crate::backup::run::Status,
}

#[derive(Default)]
pub(super) struct Activity {
    pub open: bool,
    pub generation: u64,
    pub target: Option<BackupTarget>,
    pub entries: Arc<Vec<crate::backup::history::Entry>>,
    pub loading: bool,
    pub error: Option<String>,
}

pub(super) struct PendingBackup {
    request: u64,
    target: BackupTarget,
    action: BackupAction,
}
impl App {
    pub(super) fn refresh_backup_history(&mut self) {
        let target = self.configured_backup_target();
        self.backup_activity.generation += 1;
        self.backup_activity.loading = true;
        self.backup_activity.error = None;
        if self.backup_activity.target.as_ref() != Some(&target) {
            self.backup_activity.entries = Arc::new(Vec::new());
        }
        self.backup_activity.target = Some(target.clone());
        if !self.try_command(Command::BackupHistory(
            self.backup_activity.generation,
            target,
        )) {
            self.backup_activity.loading = false;
            self.backup_activity.error =
                Some("Activity could not be loaded. Try Refresh activity.".into());
        }
    }
    pub(super) fn retry_backup_history(&mut self, id: String) {
        let target = self.configured_backup_target();
        if self.backup_activity.entries.first().is_none_or(|entry| {
            entry.id != id || entry.target != target || !entry.outcome.attention()
        }) {
            self.backup_validation_error("Backup activity changed. Refresh it before retrying.");
            return;
        }
        if self.backup_busy() || self.pending_backup.is_some() {
            return;
        }
        self.begin_backup_request(BackupAction::ResumeHistory(
            id,
            self.field("passphrase").to_owned().into(),
        ));
    }

    pub(super) fn backup_validation_error(&mut self, error: impl Into<String>) {
        self.notice(error, true);
        self.preference_notice = self.notice.as_ref().map(|notice| notice.2);
    }

    fn clear_backup_validation(&mut self) {
        if self
            .preference_notice
            .take()
            .is_some_and(|at| self.notice.as_ref().is_some_and(|notice| notice.2 == at))
        {
            self.notice = None;
        }
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
                self.sftp_connection = None;
                self.ftp_connection = None;
                self.fields.remove("ftp_password_secret");
                self.sftp_host_key = None;
                self.fields.remove("sftp_password_secret");
                self.backups_generation += 1;
                self.save_preferences();
                self.refresh_backup_history();
            }
            Err(error) => self.notice(error.to_string(), true),
        }
    }

    pub(super) fn backup_busy(&self) -> bool {
        self.backup_run
            .iter()
            .any(|row| row.target == self.configured_backup_target() && row.status.pending())
            || self
                .busy
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

    pub(super) fn ftp_form_settings(&self) -> crate::backup::ftp::Settings {
        let mut settings = self.preferences.backup_ftp.clone();
        if self.fields.contains_key("ftp_host") {
            settings.host = self.field("ftp_host").trim().into();
            settings.port = self.field("ftp_port").trim().parse().unwrap_or(0);
            settings.username = self.field("ftp_username").trim().into();
            settings.directory = self.field("ftp_directory").trim().into();
        }
        settings
    }
    pub(super) fn sftp_form_settings(&self) -> crate::backup::sftp::Settings {
        let mut settings = self.preferences.backup_sftp.clone();
        if self.fields.contains_key("sftp_host") {
            settings.host = self.field("sftp_host").trim().into();
            settings.port = self.field("sftp_port").trim().parse().unwrap_or(0);
            settings.username = self.field("sftp_username").trim().into();
            settings.directory = self.field("sftp_directory").trim().into();
            settings.fingerprint = self.field("sftp_fingerprint").trim().into();
        }
        settings
    }
    pub(super) fn probe_sftp_fingerprint(&mut self) {
        let settings = self.sftp_form_settings();
        if let Err(error) = settings.server() {
            self.backup_validation_error(error.to_string());
            return;
        }
        self.clear_backup_validation();
        self.backups_generation += 1;
        let request = self.backups_generation;
        self.sftp_host_key = Some(HostKeyReview {
            request,
            settings: settings.clone(),
            result: None,
            verified: false,
        });
        if !self.try_command(Command::ProbeSftp(request, settings)) {
            self.sftp_host_key = None;
        }
    }
    pub(super) fn accept_sftp_fingerprint(&mut self) {
        let settings = self.sftp_form_settings();
        let Some(review) = &self.sftp_host_key else {
            return;
        };
        let Some(Ok(fingerprint)) = &review.result else {
            return;
        };
        if !review.verified
            || settings.host != review.settings.host
            || settings.port != review.settings.port
        {
            return;
        }
        self.fields.insert("sftp_fingerprint", fingerprint.clone());
        self.fields.remove("sftp_password_secret");
        self.sftp_host_key = None;
        self.sftp_connection = None;
        match self.read_preferences() {
            Ok(()) => self.save_preferences(),
            Err(error) => self.backup_validation_error(error.to_string()),
        }
    }

    pub(super) fn configured_backup_target(&self) -> BackupTarget {
        if self.preferences.backup_destination == BackupDestination::Ftp
            && self.tab == Tab::Preferences
        {
            return BackupTarget::Ftp(self.ftp_form_settings().identity());
        }
        if self.preferences.backup_destination == BackupDestination::Sftp
            && self.tab == Tab::Preferences
        {
            return BackupTarget::Sftp(self.sftp_form_settings().identity());
        }
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
            && self.preferences.backup_format.encrypted()
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
        if matches!(&target, BackupTarget::Sftp(_))
            && let Err(error) = self.preferences.backup_sftp.validate()
        {
            self.backup_validation_error(error.to_string());
            return;
        }
        if matches!(&target, BackupTarget::Ftp(_))
            && let Err(error) = self.preferences.backup_ftp.validate()
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
        self.clear_backup_validation();
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
        if self.pending_backup_all == Some(request) {
            self.pending_backup_all = None;
            for row in &mut self.backup_run {
                if row.status == crate::backup::run::Status::SavingPreferences {
                    row.status = crate::backup::run::Status::Failed(
                        "Settings could not be saved. Retry after correcting the error.".into(),
                    );
                }
            }
        }
        if self
            .pending_backup
            .as_ref()
            .is_some_and(|p| p.request == request)
        {
            self.pending_backup = None;
        }
    }

    pub(super) fn continue_backup_request(&mut self, request: u64) {
        if self.pending_backup_all == Some(request) {
            self.pending_backup_all = None;
            self.dispatch_backup_run();
        }
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
            BackupAction::ConnectFtp(secret) => {
                self.ftp_connection = Some((request, pending.target.clone(), None));
                if !self.try_command(Command::ConnectFtp(request, pending.target, secret)) {
                    self.ftp_connection = None;
                }
            }
            BackupAction::ConnectSftp(secret) => {
                self.sftp_connection = Some((request, pending.target.clone(), None));
                if !self.try_command(Command::ConnectSftp(request, pending.target, secret)) {
                    self.sftp_connection = None;
                }
            }
            BackupAction::ConnectS3(secret) => {
                self.s3_connection = Some((request, pending.target.clone(), None));
                if !self.try_command(Command::ConnectS3(request, pending.target, secret)) {
                    self.s3_connection = None;
                }
            }
            BackupAction::Save(secret) => self.send(Command::Backup(pending.target, secret)),
            BackupAction::ResumeHistory(id, secret) => {
                self.send(Command::RetryBackupHistory(id, pending.target, secret))
            }
            BackupAction::List => self.request_backup_copies(pending.target),
            BackupAction::Restore(id, secret) => {
                self.send(Command::Restore(pending.target, id, secret))
            }
        }
    }

    pub(super) fn include_backup(&mut self, id: String, included: bool) {
        if let Err(error) = self.read_preferences() {
            self.backup_validation_error(error.to_string());
            return;
        }
        if let Some(destination) = self
            .preferences
            .backup_destinations
            .iter_mut()
            .find(|d| d.id == id)
        {
            destination.included = included;
            self.save_preferences();
        }
    }

    pub(super) fn begin_backup_all(&mut self, retry: Option<String>) {
        use crate::backup::run::Status;
        if self.pending_backup_all.is_some()
            || self.pending_backup.is_some()
            || (retry.is_none() && self.backup_run.iter().any(|row| row.status.pending()))
        {
            return;
        }
        if retry.as_ref().is_some_and(|id| {
            !self
                .backup_run
                .iter()
                .any(|row| &row.id == id && matches!(row.status, Status::Failed(_)))
        }) {
            return;
        }
        if let Err(error) = self.read_preferences() {
            self.backup_validation_error(error.to_string());
            return;
        }
        let destinations: Vec<_> = self
            .preferences
            .backup_destinations
            .iter()
            .filter(|d| d.included && retry.as_ref().is_none_or(|id| id == &d.id))
            .cloned()
            .collect();
        if destinations.is_empty() {
            self.backup_validation_error("Include at least one destination in Back up all.");
            return;
        }
        if retry.is_none() {
            self.backup_run.clear();
        }
        for destination in destinations {
            if self
                .backup_run
                .iter()
                .any(|row| row.id == destination.id && row.status.pending())
            {
                continue;
            }
            self.backup_run_generation += 1;
            let target = destination.target(&self.preferences);
            let ready = crate::backup::config::resolve(&self.workspace.preferences, &target)
                .is_ok_and(|p| p.backup_ready && p.backup_format == destination.format);
            let status = if ready {
                Status::SavingPreferences
            } else {
                Status::NeedsSetup(
                    "Save the first copy with the chosen options in this destination's setup."
                        .into(),
                )
            };
            let row = RunRow {
                id: destination.id,
                name: destination.name,
                target,
                request: self.backup_run_generation,
                status,
            };
            if let Some(index) = self.backup_run.iter().position(|old| old.id == row.id) {
                self.backup_run[index] = row;
            } else {
                self.backup_run.push(row);
            }
        }
        self.clear_backup_validation();
        if self
            .backup_run
            .iter()
            .any(|row| row.status == Status::SavingPreferences)
        {
            let request = self.preference_sync.changed();
            self.pending_backup_all = Some(request);
            if !self.queue_preference_write(request, self.preferences.clone()) {
                self.cancel_backup_save(request);
            }
        }
    }

    fn dispatch_backup_run(&mut self) {
        use crate::backup::run::Status;
        for index in 0..self.backup_run.len() {
            if self.backup_run[index].status != Status::SavingPreferences {
                continue;
            }
            let row = self.backup_run[index].clone();
            let current = self
                .preferences
                .backup_destinations
                .iter()
                .find(|d| d.id == row.id);
            let matches_current = current
                .is_some_and(|d| d.included && d.target(&self.preferences) == row.target)
                && (self.preferences.backup_selected.as_ref() != Some(&row.id)
                    || self.configured_backup_target() == row.target);
            if !matches_current {
                self.backup_run[index].status = Status::Failed("This destination changed or was excluded while settings were saving. Review it before retrying.".into());
            } else if self.busy.contains(&row.target.work_key()) {
                self.backup_run[index].status = Status::Failed(
                    "This destination is already working. Wait for it to finish, then retry."
                        .into(),
                );
            } else if !self.try_command(Command::BackupIncluded(row.request, row.id, row.target)) {
                self.backup_run[index].status = Status::Failed(
                    "The work queue is full. Retry this destination in a moment.".into(),
                );
            } else {
                self.backup_run[index].status = Status::Queued;
            }
        }
    }

    pub(super) fn observe_backup_run(
        &mut self,
        request: u64,
        target: BackupTarget,
        status: crate::backup::run::Status,
    ) {
        if let Some(row) = self
            .backup_run
            .iter_mut()
            .find(|row| row.request == request && row.target == target)
        {
            if let crate::backup::run::Status::Failed(error) = &status {
                self.pending_close = None;
                self.notice = Some((format!("{}: {error}", row.name), true, Instant::now()));
            }
            row.status = status;
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

    fn group_app() -> App {
        let (mut app, _) = App::new();
        app.preferences.backup_folder = "/first".into();
        crate::backup::config::add(&mut app.preferences).unwrap();
        app.preferences.backup_folder = "/second".into();
        app.preferences.backup_ready = true;
        crate::backup::config::capture_editor(&mut app.preferences);
        app.preferences.backup_destinations[0].ready = true;
        Arc::make_mut(&mut app.workspace).preferences = app.preferences.clone();
        app.settings_fields();
        app
    }

    #[test]
    fn backup_all_waits_for_save_admits_once_and_retries_only_failed_rows() {
        use crate::backup::run::Status;
        let mut app = group_app();
        let (sender, mut saves, mut commands) = engine::CommandSender::backup_test_channels();
        app.tx = Some(sender);
        app.begin_backup_all(None);
        let Command::SavePreferences(request, _) = saves.try_recv().unwrap() else {
            panic!("save first")
        };
        assert!(commands.try_recv().is_err());
        app.begin_backup_all(None);
        assert!(saves.try_recv().is_err());
        app.continue_backup_request(request + 1);
        assert!(commands.try_recv().is_err());
        app.continue_backup_request(request);
        let first = app.backup_run[0].clone();
        let second = app.backup_run[1].clone();
        for row in [&first, &second] {
            assert!(
                matches!(commands.try_recv(), Ok(Command::BackupIncluded(id, _, target)) if id == row.request && target == row.target)
            );
            assert!(
                app.busy.contains(&row.target.work_key()),
                "queued copies must block exit before backend Busy arrives"
            );
        }
        app.observe_backup_run(
            second.request,
            second.target.clone(),
            Status::Failed("offline".into()),
        );
        app.busy.remove(&second.target.work_key());
        app.begin_backup_all(Some(second.id));
        let Command::SavePreferences(retry, _) = saves.try_recv().unwrap() else {
            panic!("save retry")
        };
        app.continue_backup_request(retry);
        assert!(
            matches!(commands.try_recv(), Ok(Command::BackupIncluded(_, _, target)) if target == second.target)
        );
        assert!(
            commands.try_recv().is_err(),
            "first queued copy must not be repeated"
        );
        app.observe_backup_run(second.request, second.target, Status::Saved);
        assert_eq!(
            app.backup_run[1].status,
            Status::Queued,
            "old attempt cannot finish a retry"
        );
        app.observe_backup_run(first.request, first.target, Status::Saved);
        assert_eq!(app.backup_run[0].status, Status::Saved);
    }

    #[test]
    fn backup_all_backpressure_and_save_failure_remain_recoverable_per_destination() {
        use crate::backup::run::Status;
        let mut app = group_app();
        let (sender, mut saves, mut commands) = engine::CommandSender::backup_test_channels();
        for _ in 0..32 {
            sender.try_send(Command::LoadImages(vec![])).unwrap();
        }
        app.tx = Some(sender);
        app.begin_backup_all(None);
        let Command::SavePreferences(request, _) = saves.try_recv().unwrap() else {
            panic!("save")
        };
        app.continue_backup_request(request);
        assert!(
            app.backup_run
                .iter()
                .all(|row| matches!(&row.status, Status::Failed(error) if error.contains("queue")))
        );
        while commands.try_recv().is_ok() {}
        app.begin_backup_all(None);
        let Command::SavePreferences(request, _) = saves.try_recv().unwrap() else {
            panic!("save")
        };
        app.cancel_backup_save(request);
        app.continue_backup_request(request);
        assert!(commands.try_recv().is_err());
        assert!(
            app.backup_run.iter().all(
                |row| matches!(&row.status, Status::Failed(error) if error.contains("Settings"))
            )
        );
    }

    #[test]
    fn backup_all_latest_exclusion_wins_before_its_first_settings_acknowledgment() {
        let mut app = group_app();
        let (sender, mut saves, mut commands) = engine::CommandSender::backup_test_channels();
        app.tx = Some(sender);
        app.begin_backup_all(None);
        let Command::SavePreferences(request, _) = saves.try_recv().unwrap() else {
            panic!("save")
        };
        let id = app.preferences.backup_destinations[1].id.clone();
        app.include_backup(id, false);
        app.continue_backup_request(request);
        assert!(
            matches!(commands.try_recv(), Ok(Command::BackupIncluded(_, _, target)) if target == app.backup_run[0].target)
        );
        assert!(commands.try_recv().is_err());
        assert!(
            matches!(&app.backup_run[1].status, crate::backup::run::Status::Failed(error) if error.contains("excluded"))
        );
    }

    #[test]
    fn backup_all_include_saves_other_form_edits_in_the_same_write() {
        let mut app = group_app();
        let (sender, mut saves, _commands) = engine::CommandSender::backup_test_channels();
        app.tx = Some(sender);
        app.fields.insert("copies", "17".into());
        let id = app.preferences.backup_destinations[1].id.clone();
        app.include_backup(id, false);
        let Command::SavePreferences(_, saved) = saves.try_recv().unwrap() else {
            panic!("save")
        };
        assert_eq!(saved.backup_copies, 17);
        assert_eq!(saved.backup_destinations[1].copies, 17);
        assert!(!saved.backup_destinations[1].included);
    }

    #[test]
    fn backup_all_exclusion_survives_editor_capture_and_first_copy_stays_explicit() {
        use crate::backup::run::Status;
        let mut app = group_app();
        app.preferences.backup_destinations[1].included = false;
        app.preferences.backup_destinations[0].ready = false;
        Arc::make_mut(&mut app.workspace).preferences = app.preferences.clone();
        app.begin_backup_all(None);
        assert_eq!(app.backup_run.len(), 1);
        assert!(matches!(app.backup_run[0].status, Status::NeedsSetup(_)));
        assert!(!app.preferences.backup_destinations[1].included);
        let mut legacy = serde_json::to_value(&app.preferences.backup_destinations[1]).unwrap();
        legacy.as_object_mut().unwrap().remove("included");
        let restored: crate::backup::config::Destination = serde_json::from_value(legacy).unwrap();
        assert!(restored.included);
    }

    fn copy(id: &str) -> BackupCopy {
        BackupCopy {
            id: id.into(),
            name: id.into(),
            created_at: String::new(),
        }
    }

    #[test]
    fn ftp_security_switch_preserves_custom_ports_and_clears_stale_password_results() {
        use crate::backup::ftp::Security;
        let (mut app, _) = App::new();
        app.tab = Tab::Preferences;
        app.preferences.backup_destination = BackupDestination::Ftp;
        app.settings_fields();
        app.fields
            .insert("ftp_password_secret", "fixture-password".into());
        let old = app.configured_backup_target();
        app.ftp_connection = Some((9, old.clone(), None));
        let _ = app.handle(Message::FtpSecurity(Security::ImplicitTls));
        assert_eq!(app.field("ftp_port"), "990");
        assert!(app.field("ftp_password_secret").is_empty());
        assert!(app.ftp_connection.is_none());
        let _ = app.handle(Message::Backend(Event::FtpConnection(9, old, Ok(()))));
        assert!(app.ftp_connection.is_none());
        let _ = app.handle(Message::Field("ftp_port", "2121".into()));
        let _ = app.handle(Message::FtpSecurity(Security::ExplicitTls));
        assert_eq!(app.field("ftp_port"), "2121");
        let _ = app.handle(Message::FtpSecurity(Security::Plain));
        assert_eq!(app.field("ftp_port"), "2121");
    }

    #[test]
    fn sftp_fingerprint_review_needs_explicit_verification_and_rejects_a_changed_server() {
        use base64::Engine as _;
        let fingerprint = |byte| {
            format!(
                "SHA256:{}",
                base64::engine::general_purpose::STANDARD_NO_PAD.encode([byte; 32])
            )
        };
        let (mut app, _) = App::new();
        app.tab = Tab::Preferences;
        app.preferences.backup_destination = BackupDestination::Sftp;
        app.preferences.backup_sftp = crate::backup::sftp::Settings {
            host: "backup.example.test".into(),
            username: "fixture-user".into(),
            directory: "/archive".into(),
            fingerprint: fingerprint(1),
            ..Default::default()
        };
        app.settings_fields();
        let settings = app.sftp_form_settings();
        app.sftp_host_key = Some(HostKeyReview {
            request: 1,
            settings: settings.clone(),
            result: Some(Ok(fingerprint(2))),
            verified: false,
        });
        let _ = app.handle(Message::AcceptSftpFingerprint);
        assert_eq!(app.field("sftp_fingerprint"), fingerprint(1));
        let _ = app.handle(Message::VerifySftpFingerprint(true));
        let _ = app.handle(Message::Field("sftp_host", "different.example.test".into()));
        let _ = app.handle(Message::AcceptSftpFingerprint);
        assert_eq!(app.field("sftp_fingerprint"), fingerprint(1));
        assert!(app.sftp_host_key.is_none());
        let _ = app.handle(Message::Backend(Event::SftpFingerprint(
            1,
            settings,
            Ok(fingerprint(2)),
        )));
        assert!(app.sftp_host_key.is_none());
        let (sender, mut saves) = engine::CommandSender::persistence_test_channel();
        app.tx = Some(sender);
        app.sftp_host_key = Some(HostKeyReview {
            request: 2,
            settings: app.sftp_form_settings(),
            result: Some(Ok(fingerprint(3))),
            verified: true,
        });
        app.fields
            .insert("sftp_password_secret", "old-password".into());
        let _ = app.handle(Message::AcceptSftpFingerprint);
        let Command::SavePreferences(_, saved) = saves.try_recv().unwrap() else {
            panic!("verified fingerprint must persist");
        };
        assert_eq!(saved.backup_sftp.fingerprint, fingerprint(3));
        assert_eq!(saved.backup_sftp.host, "different.example.test");
        assert!(app.field("sftp_password_secret").is_empty());
        assert!(app.sftp_host_key.is_none());
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

    #[test]
    fn backup_history_stale_result_and_changed_destination_cannot_retry_an_old_copy() {
        let (mut app, _) = App::new();
        app.tab = Tab::Preferences;
        app.preferences.backup_folder = std::env::temp_dir()
            .join("history-first")
            .to_string_lossy()
            .into();
        app.settings_fields();
        let target = app.configured_backup_target();
        let mut row =
            crate::backup::history::Entry::new(target.clone(), "First".into(), Default::default());
        row.outcome = crate::backup::history::Outcome::NeedsReview;
        row.copy = Some("reserved-copy".into());
        app.backup_activity.generation = 2;
        let _ = app.handle(Message::Backend(Event::BackupHistory(
            1,
            target.clone(),
            Ok(Arc::new(vec![row.clone()])),
        )));
        assert!(app.backup_activity.entries.is_empty());
        let _ = app.handle(Message::Backend(Event::BackupHistory(
            2,
            target,
            Ok(Arc::new(vec![row.clone()])),
        )));
        let (sender, mut saves, mut network) = engine::CommandSender::backup_test_channels();
        app.tx = Some(sender);
        app.retry_backup_history(row.id);
        let Command::SavePreferences(request, _) = saves.try_recv().unwrap() else {
            panic!("Retry must save settings first");
        };
        app.fields.insert(
            "backup_folder",
            std::env::temp_dir()
                .join("history-second")
                .to_string_lossy()
                .into(),
        );
        app.continue_backup_request(request);
        assert!(network.try_recv().is_err());
        assert!(app.notice.unwrap().0.contains("destination changed"));
    }

    #[test]
    fn backup_history_read_queue_backpressure_keeps_refresh_available() {
        let (mut app, _) = App::new();
        let target = app.configured_backup_target();
        let (sender, _reads) = engine::CommandSender::foreground_test_channel();
        for request in 0..32 {
            sender
                .try_send(Command::BackupHistory(request, target.clone()))
                .unwrap();
        }
        app.tx = Some(sender);
        app.refresh_backup_history();
        assert!(!app.backup_activity.loading);
        assert!(
            app.backup_activity
                .error
                .as_ref()
                .unwrap()
                .contains("Refresh activity")
        );
    }
}
