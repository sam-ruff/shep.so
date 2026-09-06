use super::*;

impl Engine {
    pub(super) async fn run_restore(
        &self,
        target: BackupTarget,
        id: String,
        passphrase: SecretString,
        output: &mut Output,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(!self.demo, "Restore is disabled in preview.");
        // Local archives may also contain Google sources. Serialize against
        // login/calendar discovery while preserving this device's Google token.
        let _google = self.google_connection_lock.read().await;
        let prefs = self.store.get("preferences").await?;
        Self::check_backup_target(&target, &prefs)?;
        let bytes = self.backup_provider(&prefs).await?.download(&id).await?;
        let snapshot =
            tokio::task::spawn_blocking(move || backup::decrypt(&bytes, &passphrase)).await??;
        self.import_snapshot(snapshot, output).await
    }

    pub(super) async fn import_snapshot(
        &self,
        snapshot: Snapshot,
        output: &mut Output,
    ) -> anyhow::Result<()> {
        let snapshot = tokio::task::spawn_blocking(move || {
            snapshot.validate()?;
            Ok::<_, anyhow::Error>(snapshot)
        })
        .await??;
        let _lifecycle = self.connection_lifecycle_lock.lock().await;
        // Match transfer's sorted account-lock order. Syncs, flags, moves and
        // connection edits cannot race the import or its missing-password pass.
        let mut accounts: Vec<_> = snapshot
            .accounts
            .iter()
            .map(|a| a.id.as_str())
            .chain(
                snapshot
                    .messages
                    .iter()
                    .map(|m| m.summary.account_id.as_str()),
            )
            .collect();
        accounts.sort_unstable();
        accounts.dedup();
        let mut guards = Vec::new();
        for id in accounts {
            guards.push(self.account_lock(id).await);
        }
        let mut calendars: Vec<_> = snapshot.calendars.iter().map(|c| c.id.as_str()).collect();
        calendars.sort_unstable();
        for id in calendars {
            guards.push(self.calendar_lock(id).await);
        }
        let restored = self.store.restore_snapshot(snapshot).await?;
        // The SQLite commit is complete even if a keychain prompt fails next.
        // Never compensate by deleting restored mail or overwriting passwords.
        self.workspace(output).await.context(
            "Mail restored, but the workspace could not refresh. Reopen Shep to reload it.",
        )?;
        output.send(Event::Changed).await?;
        let mut failed = 0;
        for (id, secret) in restored.credentials {
            if self
                .restore_credentials
                .restore_missing(&id, secret)
                .await
                .is_err()
            {
                failed += 1;
            }
        }
        let mut notice = format!(
            "Backup restored: {} emails added. Existing mail and connection settings were kept.",
            restored.messages
        );
        if restored.kept_connections > 0 {
            notice.push_str(" Passwords for connections whose settings have changed were skipped.");
        }
        if failed > 0 {
            output.send(Event::Error(format!("{notice} {failed} passwords could not be saved to the OS keychain. Unlock it and restore this copy again, or enter the missing passwords in account settings."))).await?;
        } else {
            notice.push_str(" Accounts without a saved password need it entered before syncing.");
            output.send(Event::Notice(notice)).await?;
        }
        Ok(())
    }
}
