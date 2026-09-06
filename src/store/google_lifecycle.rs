use super::*;
use std::collections::HashSet;

impl Store {
    /// Disable service access before attempting keychain cleanup. Retain cached
    /// calendars and backup identity so reconnect can recover existing copies.
    pub async fn disconnect_google(&self, revision: u64) -> anyhow::Result<GoogleLifecycle> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut prefs: Preferences = get(&tx, "preferences")?;
            anyhow::ensure!(
                prefs.google_lifecycle.revision == revision,
                "The Google connection changed. Review it again before disconnecting."
            );
            prefs.google_lifecycle = GoogleLifecycle {
                revision: revision
                    .checked_add(1)
                    .context("Google connection revision overflow")?,
                disconnected: true,
                cleanup_pending: true,
            };
            if prefs.backup_destination == BackupDestination::GoogleDrive {
                prefs.auto_backup = false;
                prefs.backup_ready = false;
            }
            let mut sources: Vec<CalendarSource> = get(&tx, "calendars")?;
            let mut archived: HashSet<String> = get(&tx, "google_archived")?;
            for source in &mut sources {
                if source.kind == CalendarKind::Google {
                    source.access = CalendarAccess::READ_ONLY;
                    archived.insert(source.id.clone());
                }
            }
            put(&tx, "preferences", &prefs)?;
            put(&tx, "calendars", &sources)?;
            put(&tx, "google_archived", &archived)?;
            connections::changed(&tx)?;
            tx.commit()?;
            Ok(prefs.google_lifecycle)
        })
        .await
    }

    pub async fn finish_google_cleanup(&self, revision: u64) -> anyhow::Result<()> {
        self.update_preferences_checked(move |prefs| {
            anyhow::ensure!(
                prefs.google_lifecycle.revision == revision && prefs.google_lifecycle.disconnected,
                "Google changed while credential cleanup was running."
            );
            prefs.google_lifecycle.cleanup_pending = false;
            Ok(())
        })
        .await?;
        Ok(())
    }
}
