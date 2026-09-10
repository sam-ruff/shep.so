use super::*;

struct FinishReport {
    clean: bool,
    detail: String,
}

impl Engine {
    pub(super) async fn connect_ftp(
        &self,
        target: &BackupTarget,
        supplied: Option<SecretString>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.demo, "FTP connections are disabled in preview.");
        let saved: Preferences = self.store.get("preferences").await?;
        let configured = backup::config::resolve(&saved, target)?;
        anyhow::ensure!(
            configured.backup_destination == BackupDestination::Ftp,
            "Choose an FTP destination first."
        );
        let id = configured.backup_ftp.secret_id();
        let secret = match supplied {
            Some(secret) => secret,
            None => self
                .credentials
                .read(&id)
                .await
                .map_err(|_| anyhow::anyhow!("Enter the FTP password for this server."))?,
        };
        backup::ftp::FtpBackup::new(&configured.backup_ftp, secret.clone())?
            .test_connection()
            .await?;
        let current: Preferences = self.store.get("preferences").await?;
        let current = backup::config::resolve(&current, target)?;
        anyhow::ensure!(
            current.backup_ftp.secret_id() == id,
            "The FTP connection settings changed. Test the current settings again."
        );
        self.credentials.write(&id, secret).await?;
        Ok(())
    }

    pub(super) async fn connect_sftp(
        &self,
        target: &BackupTarget,
        supplied: Option<SecretString>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.demo, "SFTP connections are disabled in preview.");
        let saved: Preferences = self.store.get("preferences").await?;
        let configured = backup::config::resolve(&saved, target)?;
        anyhow::ensure!(
            configured.backup_destination == BackupDestination::Sftp,
            "Choose an SFTP destination first."
        );
        let id = configured.backup_sftp.secret_id();
        let secret = match supplied {
            Some(secret) => secret,
            None => self.credentials.read(&id).await.map_err(|_| {
                anyhow::anyhow!("Enter the SFTP password for this verified server.")
            })?,
        };
        backup::sftp::SftpBackup::new(&configured.backup_sftp, secret.clone())?
            .test_connection()
            .await?;
        let current: Preferences = self.store.get("preferences").await?;
        let current = backup::config::resolve(&current, target)?;
        anyhow::ensure!(
            current.backup_sftp.secret_id() == id,
            "The SFTP connection settings changed. Test the current settings again."
        );
        self.credentials.write(&id, secret).await?;
        Ok(())
    }

    pub(super) async fn connect_s3(
        &self,
        target: &BackupTarget,
        supplied: Option<(SecretString, SecretString)>,
        factory: impl FnOnce(
            &backup::s3::Settings,
            &SecretString,
        ) -> anyhow::Result<backup::s3::S3Backup>
        + Send,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.demo, "S3 connections are disabled in preview.");
        let saved: Preferences = self.store.get("preferences").await?;
        let configured = backup::config::resolve(&saved, target)?;
        anyhow::ensure!(
            configured.backup_destination == BackupDestination::S3,
            "Choose an S3 destination first."
        );
        let id = configured.backup_s3.identity().secret_id();
        let secret = match supplied {
            Some((key, secret)) => backup::s3::access_secret(
                key.expose_secret().into(),
                secret.expose_secret().into(),
            )?,
            None => self.credentials.read(&id).await.map_err(|_| {
                anyhow::anyhow!(
                    "Enter the S3 access key and secret key to set up this destination."
                )
            })?,
        };
        factory(&configured.backup_s3, &secret)?
            .test_connection()
            .await?;
        let current: Preferences = self.store.get("preferences").await?;
        backup::config::resolve(&current, target)?;
        self.credentials.write(&id, secret).await?;
        Ok(())
    }

    pub(super) async fn backup_journal(&self) -> anyhow::Result<backup::journal::Journal> {
        self.backup_uploads
            .get_or_try_init(|| async {
                let path = self
                    .store
                    .run(|connection| {
                        Ok(connection
                            .path()
                            .filter(|path| !path.is_empty())
                            .map(|path| {
                                std::path::PathBuf::from(path)
                                    .with_file_name("backup-uploads.sqlite")
                            }))
                    })
                    .await?;
                let key = self.store.connection_key();
                tokio::task::spawn_blocking(move || match (path.as_deref(), key) {
                    (Some(path), Some(key)) => backup::journal::Journal::open_encrypted(path, &key),
                    (path, None) => backup::journal::Journal::open(path),
                    (None, Some(_)) => {
                        anyhow::bail!("The encrypted backup journal needs a saved workspace.")
                    }
                })
                .await?
            })
            .await
            .cloned()
    }

    pub(super) async fn backup_connection_guard(
        &self,
        target: &BackupTarget,
    ) -> Option<tokio::sync::OwnedRwLockReadGuard<()>> {
        match target {
            BackupTarget::GoogleDrive { .. } => {
                Some(self.google_connection_lock.clone().read_owned().await)
            }
            BackupTarget::Local(_)
            | BackupTarget::S3(_)
            | BackupTarget::Sftp(_)
            | BackupTarget::Ftp(_) => None,
        }
    }

    pub(super) fn check_backup_target(
        target: &BackupTarget,
        prefs: &Preferences,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            !matches!(target, BackupTarget::GoogleDrive { .. })
                || (!prefs.google_lifecycle.disconnected
                    && prefs.google_grant.access.drive_allowed()),
            "Reconnect Google and approve Drive backup access before accessing copies."
        );
        anyhow::ensure!(
            backup::config::resolve(prefs, target).is_ok(),
            "The backup destination changed. Refresh copies or start the backup again with the current settings."
        );
        Ok(())
    }

    pub(super) fn allow_backup(&self) -> anyhow::Result<()> {
        #[cfg(feature = "test-support")]
        if self.demo && crate::test_support::backups::active() {
            return Ok(());
        }
        anyhow::ensure!(!self.demo, "Backup is disabled in preview.");
        Ok(())
    }

    pub(super) async fn run_backup(
        &self,
        target: BackupTarget,
        supplied: Option<SecretString>,
        output: &mut Output,
    ) -> anyhow::Result<()> {
        self.run_backup_with_history(target, supplied, output, None)
            .await
    }

    async fn backup_progress(
        output: &mut Output,
        request: Option<u64>,
        target: &BackupTarget,
        status: backup::run::Status,
    ) -> anyhow::Result<()> {
        if let Some(request) = request {
            output
                .send(Event::BackupRun(request, target.clone(), status))
                .await?;
        }
        Ok(())
    }

    async fn run_backup_with_history(
        &self,
        target: BackupTarget,
        supplied: Option<SecretString>,
        output: &mut Output,
        included: Option<(u64, &str)>,
    ) -> anyhow::Result<()> {
        use backup::history::{Entry, Outcome};
        self.allow_backup()?;
        let saved: Preferences = self.store.get("preferences").await?;
        let prefs = backup::config::resolve(&saved, &target)?;
        if supplied.is_none() && included.is_none() && (!prefs.auto_backup || !prefs.backup_ready) {
            return Ok(());
        }
        let name = prefs
            .backup_destinations
            .iter()
            .find(|d| d.target(&prefs) == target)
            .map(|d| d.name.clone())
            .unwrap_or_else(|| prefs.backup_destination.to_string());
        let mut history = Entry::new(target.clone(), name, prefs.backup_format);
        self.store
            .write_backup_history(history.clone())
            .await
            .context("Could not save backup history; the copy was not started")?;
        let result = self
            .run_backup_observed(target.clone(), supplied, output, included, &mut history)
            .await;
        history.finished = Some(chrono::Utc::now().timestamp_millis());
        if let Err(error) = &result {
            history.detail = format!("{error:#}");
            history.outcome =
                if matches!(history.outcome, Outcome::Saved | Outcome::SavedWithWarning) {
                    Outcome::SavedWithWarning
                } else if history.copy.is_some() {
                    Outcome::NeedsReview
                } else {
                    Outcome::Failed
                };
        }
        let confirmed = matches!(history.outcome, Outcome::Saved | Outcome::SavedWithWarning);
        if let Err(error) = self.store.write_backup_history(history).await {
            // The upload journal and receipt remain authoritative, even if this
            // separate activity view cannot record its final result.
            output.send(Event::Error(format!("Could not update backup activity: {error}. The upload receipt was kept; refresh activity after reopening Shep."))).await?;
        }
        output
            .send(Event::BackupHistoryChanged(target.clone()))
            .await?;
        if confirmed && let Err(error) = &result {
            let detail = format!("Copy saved, but follow-up work failed: {error:#}");
            output.send(Event::Error(detail.clone())).await?;
            Self::backup_progress(
                output,
                included.map(|(request, _)| request),
                &target,
                backup::run::Status::SavedWithWarning(detail),
            )
            .await?;
            return Ok(());
        }
        result
    }

    async fn run_backup_observed(
        &self,
        target: BackupTarget,
        supplied: Option<SecretString>,
        output: &mut Output,
        included: Option<(u64, &str)>,
        history: &mut backup::history::Entry,
    ) -> anyhow::Result<()> {
        let request = included.map(|(request, _)| request);
        self.allow_backup()?;
        let _guard = self.backup_connection_guard(&target).await;
        let saved: Preferences = self.store.get("preferences").await?;
        Self::check_backup_target(&target, &saved)?;
        let prefs = backup::config::resolve(&saved, &target)?;
        prefs.validate()?;
        let automatic = supplied.is_none() && included.is_none();
        if automatic && (!prefs.auto_backup || !prefs.backup_ready) {
            return Ok(());
        }
        let provider = self.backup_provider(&prefs).await?;
        let journal = self.backup_journal().await?;
        let mut pending = journal.pending(&target).await?;
        if let Some(previous) = &pending {
            history.copy = Some(previous.upload.id.clone());
            history.format = backup::format::options(&previous.data)?;
            if previous.committed {
                history.outcome = backup::history::Outcome::SavedWithWarning;
            }
        }
        if let Some(previous) = &pending
            && previous.committed
            && !provider.verify_upload(&previous.upload).await?
        {
            // A known successful copy can have been removed later. This is not
            // an ambiguous upload, so a new snapshot may safely replace it.
            journal.remove(&target, &previous.upload.id).await?;
            pending = None;
            history.copy = None;
            history.outcome = backup::history::Outcome::Unfinished;
        }
        // A retained upload owns its format and passphrase even after the form changes.
        let archive_format = pending
            .as_ref()
            .map(|p| backup::format::options(&p.data))
            .transpose()?
            .unwrap_or(prefs.backup_format);
        let passphrase = if archive_format.encrypted() {
            if let Some(secret) = supplied.filter(|secret| !secret.expose_secret().is_empty()) {
                anyhow::ensure!(
                    secret.expose_secret().chars().count() >= 12,
                    "Use a backup passphrase of at least 12 characters."
                );
                Some(secret)
            } else {
                match self.passphrases.read(&target).await {
                    Ok(secret) => Some(secret),
                    Err(_) => {
                        if automatic {
                            let changed = target.clone();
                            self.store
                                .update_preferences(move |current| {
                                    backup::config::pause(current, &changed)
                                })
                                .await?;
                            self.workspace(output).await?;
                            anyhow::bail!(
                                "Automatic backups are paused because the saved passphrase is unavailable. Unlock your OS keychain, or enter the original passphrase in Backups and choose Back up now."
                            );
                        }
                        anyhow::bail!(
                            "Unlock your OS keychain, or turn on encryption and enter this destination's original passphrase in Backups to resume its pending copy."
                        );
                    }
                }
            }
        } else {
            None
        };
        if let Some((_, id)) = included {
            let current: Preferences = self.store.get("preferences").await?;
            Self::check_included(&current, id, &target)?;
        }
        let pending = match pending {
            Some(pending) => pending,
            None => {
                Self::backup_progress(output, request, &target, backup::run::Status::Preparing)
                    .await?;
                let bytes = self.prepare_snapshot(&prefs, passphrase.as_ref()).await?;
                let name = format!(
                    "shep-{}-{}.shepbackup",
                    chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
                    uuid::Uuid::new_v4()
                );
                let upload = provider.reserve(&name, &bytes).await?;
                journal.prepare(&target, upload, bytes).await?
            }
        };
        history.copy = Some(pending.upload.id.clone());
        history.format = archive_format;
        self.store
            .write_backup_history(history.clone())
            .await
            .context("Could not save the reserved copy in backup activity; retry to continue")?;
        // Never associate a new passphrase with a previously staged archive.
        let secret = passphrase.clone();
        let mut pending = tokio::task::spawn_blocking(move || {
            backup::format::verify(&pending.data, secret.as_ref())
                .context("Enter the original passphrase to resume this pending backup")?;
            Ok::<_, anyhow::Error>(pending)
        })
        .await??;
        if !pending.committed {
            Self::backup_progress(output, request, &target, backup::run::Status::Uploading).await?;
            let checkpoint = backup::journal::Checkpoint {
                journal: journal.clone(),
                target: target.clone(),
            };
            provider.upload_prepared(&mut pending.upload, &pending.data, &checkpoint).await
                .context("The pending copy was kept. Retry to resume it; use Setup if it needs its original passphrase")?;
        }
        // Acknowledged server acceptance stays distinct from later local/keychain
        // or retention failures, including a closed observation channel.
        history.outcome = backup::history::Outcome::Saved;
        self.store
            .write_backup_history(history.clone())
            .await
            .context("Copy uploaded, but its activity receipt could not be saved")?;
        Self::backup_progress(output, request, &target, backup::run::Status::Finishing).await?;
        let upload = pending.upload;
        let marked = journal.committed(&target, &upload.id).await;
        let created_at = upload
            .name
            .strip_prefix("shep-")
            .and_then(|name| name.get(..16))
            .and_then(|time| chrono::NaiveDateTime::parse_from_str(time, "%Y%m%dT%H%M%SZ").ok())
            .map(|time| time.and_utc().to_rfc3339())
            .unwrap_or_default();
        let mut completed_preferences = prefs.clone();
        completed_preferences.backup_format = archive_format;
        let report = self
            .finish_backup_archive(
                provider.as_ref(),
                &completed_preferences,
                target.clone(),
                BackupCopy {
                    id: upload.id.clone(),
                    name: upload.name,
                    created_at,
                },
                passphrase,
                output,
            )
            .await?;
        history.detail = report.detail;
        let mut fully_finished = report.clean && marked.is_ok();
        match marked {
            Ok(()) if report.clean => {
                if let Err(error) = journal.remove(&target, &upload.id).await {
                    fully_finished = false;
                    output
                        .send(Event::Error(format!(
                            "Backup saved. Could not clear its completed upload record: {error}"
                        )))
                        .await?;
                }
            }
            Err(error) => {
                output
                    .send(Event::Error(format!(
                        "Backup saved. Could not record its upload acknowledgment: {error}"
                    )))
                    .await?;
            }
            _ => {}
        }
        if !fully_finished {
            history.outcome = backup::history::Outcome::SavedWithWarning;
            if history.detail.is_empty() {
                history.detail = "The copy was saved, but its local upload receipt needs attention. Retry to finish the retained receipt.".into();
            }
        }
        Self::backup_progress(output, request, &target, if fully_finished {
            backup::run::Status::Saved
        } else {
            backup::run::Status::SavedWithWarning("Copy saved; cleanup or keychain setup needs attention. Open this destination to review the error before retrying.".into())
        }).await?;
        Ok(())
    }

    pub(super) async fn backup_included(
        &self,
        request: u64,
        id: String,
        target: BackupTarget,
        output: &mut Output,
    ) -> anyhow::Result<()> {
        Self::backup_progress(
            output,
            Some(request),
            &target,
            backup::run::Status::Preparing,
        )
        .await?;
        let result: anyhow::Result<()> = async {
            self.allow_backup()?;
            let current: Preferences = self.store.get("preferences").await?;
            Self::check_included(&current, &id, &target)?;
            self.run_backup_with_history(target.clone(), None, output, Some((request, &id)))
                .await
        }
        .await;
        if let Err(error) = result {
            output
                .send(Event::Error(format!(
                    "Backup destination needs attention: {error:#}"
                )))
                .await?;
            Self::backup_progress(
                output,
                Some(request),
                &target,
                backup::run::Status::Failed(format!("{error:#}")),
            )
            .await?;
        }
        Ok(())
    }

    pub(super) fn check_included(
        prefs: &Preferences,
        id: &str,
        target: &BackupTarget,
    ) -> anyhow::Result<()> {
        let destination = prefs
            .backup_destinations
            .iter()
            .find(|d| d.id == id)
            .context("This destination was removed. Review the current backup destinations.")?;
        anyhow::ensure!(
            destination.included && destination.target(prefs) == *target,
            "This destination changed or was excluded. Review its settings before retrying."
        );
        anyhow::ensure!(
            destination.ready,
            "Finish setup: open this destination and save the first copy with its chosen options using Back up now."
        );
        Self::check_backup_target(target, prefs)
    }

    async fn prepare_snapshot(
        &self,
        prefs: &Preferences,
        passphrase: Option<&SecretString>,
    ) -> anyhow::Result<Vec<u8>> {
        let accounts: Vec<Account> = self.store.get("accounts").await?;
        let calendars: Vec<CalendarSource> = self.store.get("calendars").await?;
        let mut credentials = Vec::new();
        anyhow::ensure!(
            prefs.backup_format.encrypted() || !prefs.backup_accounts,
            "Account passwords require an encrypted backup."
        );
        if prefs.backup_accounts {
            for id in accounts
                .iter()
                .map(|a| a.id.clone())
                .chain(
                    calendars
                        .iter()
                        .filter(|c| c.kind == CalendarKind::CalDav)
                        .map(|c| c.id.clone()),
                )
                .chain(
                    accounts
                        .iter()
                        .filter(|a| a.smtp_separate_password)
                        .map(|a| format!("{}:smtp", a.id)),
                )
            {
                credentials.push((
                    id.clone(),
                    self.credentials
                        .read(&id)
                        .await?
                        .expose_secret()
                        .to_string(),
                ));
            }
        }
        let snapshot = Snapshot {
            version: 1,
            created_at: chrono::Utc::now().timestamp(),
            messages: self.store.export().await?,
            accounts,
            calendars,
            preferences: prefs.clone(),
            credentials,
        };
        let secret = passphrase.cloned();
        let format = prefs.backup_format;
        tokio::task::spawn_blocking(move || {
            backup::format::encode(&snapshot, format, secret.as_ref())
        })
        .await?
    }

    #[cfg(test)]
    pub(super) async fn finish_backup(
        &self,
        provider: &dyn BackupProvider,
        prefs: &Preferences,
        target: BackupTarget,
        copy: BackupCopy,
        passphrase: SecretString,
        output: &mut Output,
    ) -> anyhow::Result<bool> {
        self.finish_backup_archive(provider, prefs, target, copy, Some(passphrase), output)
            .await
            .map(|report| report.clean)
    }

    /// Upload acknowledgement is final even if cleanup or local metadata fails.
    async fn finish_backup_archive(
        &self,
        provider: &dyn BackupProvider,
        prefs: &Preferences,
        target: BackupTarget,
        copy: BackupCopy,
        passphrase: Option<SecretString>,
        output: &mut Output,
    ) -> anyhow::Result<FinishReport> {
        output
            .send(Event::BackupSaved(target.clone(), copy.clone()))
            .await?;
        let mut warnings = Vec::new();
        let time = chrono::DateTime::parse_from_rfc3339(&copy.created_at)
            .map(|time| time.timestamp())
            .unwrap_or_else(|_| chrono::Utc::now().timestamp());
        // Record the commit before keychain/retention work, so a later failure
        // does not schedule another upload on the next timer tick.
        if let Err(error) = self
            .store
            .record_backup_format(target.clone(), prefs.backup_format, time, false)
            .await
        {
            warnings.push(format!("Could not record backup history: {error}."));
        }
        // Remember manual copies too: enabling the schedule later must work.
        let saved_passphrase = if let Some(passphrase) = passphrase {
            self.passphrases.write(&target, passphrase).await
        } else {
            Ok(())
        };
        let ready = match saved_passphrase {
            Ok(()) => true,
            Err(_) => {
                warnings.push("Automatic backups need setup: unlock your OS keychain, enter the passphrase in Backups and choose Back up now.".into());
                false
            }
        };
        if let Err(error) = self
            .store
            .record_backup_format(target.clone(), prefs.backup_format, time, ready)
            .await
        {
            warnings.push(format!("Could not update the backup schedule: {error}."));
        }
        let keep = self
            .store
            .get::<Preferences>("preferences")
            .await
            .ok()
            .and_then(|p| backup::config::resolve(&p, &target).ok())
            .map_or(prefs.backup_copies, |p| p.backup_copies);
        if let Err(error) = backup::retain(provider, keep, &copy.id).await {
            warnings.push(format!("Older-copy cleanup did not finish: {error}."));
        }
        if let Err(error) = self.workspace(output).await {
            warnings.push(format!("Could not refresh local backup settings: {error}."));
        }
        output.send(Event::BackupFinished(target.clone())).await?;
        let clean = warnings.is_empty();
        let saved_label = if prefs.backup_format.encrypted() {
            "Encrypted backup saved."
        } else {
            "Unencrypted backup saved."
        };
        let saved_label = prefs
            .backup_destinations
            .iter()
            .find(|destination| destination.target(prefs) == target)
            .map_or_else(
                || saved_label.to_owned(),
                |destination| format!("{}: {saved_label}", destination.name),
            );
        output
            .send(if clean {
                Event::Notice(saved_label)
            } else {
                Event::Error(format!("{saved_label} {}", warnings.join(" ")))
            })
            .await?;
        Ok(FinishReport {
            clean,
            detail: warnings.join(" "),
        })
    }
}
