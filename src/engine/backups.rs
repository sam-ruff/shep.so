use super::*;

impl Engine {
    pub(super) async fn backup_connection_guard(
        &self,
        target: &BackupTarget,
    ) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        match target {
            BackupTarget::GoogleDrive { .. } => {
                Some(self.google_connection_lock.clone().lock_owned().await)
            }
            BackupTarget::Local(_) => None,
        }
    }

    pub(super) fn check_backup_target(
        target: &BackupTarget,
        prefs: &Preferences,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            *target == BackupTarget::from_preferences(prefs),
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
        let prefs: Preferences = self.store.get("preferences").await?;
        Self::check_backup_target(&target, &prefs)?;
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
                            if BackupTarget::from_preferences(current) == changed {
                                current.backup_ready = false;
                            }
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
                    providers::read_secret(&id)
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
        let bytes =
            tokio::task::spawn_blocking(move || backup::encrypt(&snapshot, &secret)).await??;
        let now = chrono::Utc::now();
        let name = format!(
            "shep-{}-{}.shepbackup",
            now.format("%Y%m%dT%H%M%SZ"),
            uuid::Uuid::new_v4()
        );
        let id = provider.upload(&name, bytes).await?;
        self.finish_backup(
            provider.as_ref(),
            &prefs,
            target,
            BackupCopy {
                id,
                name,
                created_at: now.to_rfc3339(),
            },
            passphrase,
            output,
        )
        .await
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
    ) -> anyhow::Result<()> {
        output
            .send(Event::BackupSaved(target.clone(), copy.clone()))
            .await?;
        let mut warnings = Vec::new();
        let time = chrono::Utc::now().timestamp();
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
            .filter(|p| BackupTarget::from_preferences(p) == target)
            .map_or(prefs.backup_copies, |p| p.backup_copies);
        if let Err(error) = backup::retain(provider, keep, &copy.id).await {
            warnings.push(format!("Older-copy cleanup did not finish: {error}."));
        }
        if let Err(error) = self.workspace(output).await {
            warnings.push(format!("Could not refresh local backup settings: {error}."));
        }
        output.send(Event::BackupFinished(target)).await?;
        output
            .send(if warnings.is_empty() {
                Event::Notice("Encrypted backup saved.".into())
            } else {
                Event::Error(format!("Encrypted backup saved. {}", warnings.join(" ")))
            })
            .await?;
        Ok(())
    }
}
