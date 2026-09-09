use super::*;

impl Engine {
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
                tokio::task::spawn_blocking(move || backup::journal::Journal::open(path.as_deref()))
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
            BackupTarget::Local(_) | BackupTarget::S3(_) | BackupTarget::Sftp(_) => None,
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

    pub(super) async fn run_backup(
        &self,
        target: BackupTarget,
        supplied: Option<SecretString>,
        output: &mut Output,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.demo, "Backup is disabled in preview.");
        let _guard = self.backup_connection_guard(&target).await;
        let saved: Preferences = self.store.get("preferences").await?;
        Self::check_backup_target(&target, &saved)?;
        let prefs = backup::config::resolve(&saved, &target)?;
        prefs.validate()?;
        let passphrase = if let Some(secret) = supplied {
            secret
        } else {
            // Recheck after queueing: the user may have disabled the schedule.
            if !prefs.auto_backup || !prefs.backup_ready {
                return Ok(());
            }
            match self.passphrases.read(&target).await {
                Ok(secret) => secret,
                Err(_) => {
                    let changed = target.clone();
                    self.store
                        .update_preferences(move |current| {
                            backup::config::pause(current, &changed);
                        })
                        .await?;
                    self.workspace(output).await?;
                    anyhow::bail!(
                        "Automatic backups are paused because the saved passphrase is unavailable. Enter it in Backups and choose Back up now to resume."
                    );
                }
            }
        };
        anyhow::ensure!(
            passphrase.expose_secret().chars().count() >= 12,
            "Use a backup passphrase of at least 12 characters."
        );
        let provider = self.backup_provider(&prefs).await?;
        let journal = self.backup_journal().await?;
        let mut pending = journal.pending(&target).await?;
        if let Some(previous) = &pending
            && previous.committed
            && !provider.verify_upload(&previous.upload).await?
        {
            // A known successful copy can have been removed later. This is not
            // an ambiguous upload, so a new snapshot may safely replace it.
            journal.remove(&target, &previous.upload.id).await?;
            pending = None;
        }
        let pending = match pending {
            Some(pending) => pending,
            None => {
                let bytes = self.encrypted_snapshot(&prefs, &passphrase).await?;
                let name = format!(
                    "shep-{}-{}.shepbackup",
                    chrono::Utc::now().format("%Y%m%dT%H%M%SZ"),
                    uuid::Uuid::new_v4()
                );
                let upload = provider.reserve(&name, &bytes).await?;
                journal.prepare(&target, upload, bytes).await?
            }
        };
        // Never associate a new passphrase with a previously staged archive.
        let secret = passphrase.clone();
        let mut pending = tokio::task::spawn_blocking(move || {
            backup::verify_passphrase(&pending.data, &secret)
                .context("Enter the original passphrase to resume this pending backup")?;
            Ok::<_, anyhow::Error>(pending)
        })
        .await??;
        if !pending.committed {
            let checkpoint = backup::journal::Checkpoint {
                journal: journal.clone(),
                target: target.clone(),
            };
            provider.upload_prepared(&mut pending.upload, &pending.data, &checkpoint).await
                .context("The pending encrypted copy was kept. Retry Back up now with its original passphrase to resume")?;
        }
        let upload = pending.upload;
        let marked = journal.committed(&target, &upload.id).await;
        let created_at = upload
            .name
            .strip_prefix("shep-")
            .and_then(|name| name.get(..16))
            .and_then(|time| chrono::NaiveDateTime::parse_from_str(time, "%Y%m%dT%H%M%SZ").ok())
            .map(|time| time.and_utc().to_rfc3339())
            .unwrap_or_default();
        let clean = self
            .finish_backup(
                provider.as_ref(),
                &prefs,
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
        match marked {
            Ok(()) if clean => {
                if let Err(error) = journal.remove(&target, &upload.id).await {
                    output.send(Event::Error(format!("Encrypted backup saved. Could not clear its completed upload record: {error}"))).await?;
                }
            }
            Err(error) => {
                output.send(Event::Error(format!("Encrypted backup saved. Could not record its upload acknowledgment: {error}"))).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn encrypted_snapshot(
        &self,
        prefs: &Preferences,
        passphrase: &SecretString,
    ) -> anyhow::Result<Vec<u8>> {
        let accounts: Vec<Account> = self.store.get("accounts").await?;
        let calendars: Vec<CalendarSource> = self.store.get("calendars").await?;
        let mut credentials = Vec::new();
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
        let secret = passphrase.clone();
        tokio::task::spawn_blocking(move || backup::encrypt(&snapshot, &secret)).await?
    }

    /// Upload acknowledgement is final even if cleanup or local metadata fails.
    pub(super) async fn finish_backup(
        &self,
        provider: &dyn BackupProvider,
        prefs: &Preferences,
        target: BackupTarget,
        copy: BackupCopy,
        passphrase: SecretString,
        output: &mut Output,
    ) -> anyhow::Result<bool> {
        output
            .send(Event::BackupSaved(target.clone(), copy.clone()))
            .await?;
        let mut warnings = Vec::new();
        let time = chrono::DateTime::parse_from_rfc3339(&copy.created_at)
            .map(|time| time.timestamp())
            .unwrap_or_else(|_| chrono::Utc::now().timestamp());
        // Record the commit before keychain/retention work, so a later failure
        // does not schedule another upload on the next timer tick.
        if let Err(error) = self.store.record_backup(target.clone(), time, false).await {
            warnings.push(format!("Could not record backup history: {error}."));
        }
        // Remember manual copies too: enabling the schedule later must work.
        let ready = match self.passphrases.write(&target, passphrase).await {
            Ok(()) => true,
            Err(_) => {
                warnings.push("Automatic backups need setup: unlock your OS keychain, enter the passphrase in Backups and choose Back up now.".into());
                false
            }
        };
        if let Err(error) = self.store.record_backup(target.clone(), time, ready).await {
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
        output.send(Event::BackupFinished(target)).await?;
        let clean = warnings.is_empty();
        output
            .send(if clean {
                Event::Notice("Encrypted backup saved.".into())
            } else {
                Event::Error(format!("Encrypted backup saved. {}", warnings.join(" ")))
            })
            .await?;
        Ok(clean)
    }
}
