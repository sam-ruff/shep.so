use super::*;
use std::collections::HashSet;

impl Store {
    /// The credential vault already contains the old and candidate grants. This
    /// transaction is the sole activation point for credentials and source access.
    pub async fn activate_google(
        &self,
        expected: Preferences,
        grant: GoogleGrant,
        drive_identity: Option<String>,
        sources: Vec<CalendarSource>,
    ) -> anyhow::Result<PreferenceSnapshot> {
        anyhow::ensure!(
            !grant.id.is_empty()
                && grant.client_id == expected.google_client_id
                && grant.access.known
                && (grant.access.drive || grant.access.calendar_read),
            "Invalid Google grant."
        );
        anyhow::ensure!(
            grant.access.drive == drive_identity.is_some(),
            "Google Drive identity has not been verified."
        );
        anyhow::ensure!(
            sources.iter().all(|s| s.kind == CalendarKind::Google)
                && (grant.access.calendar_read || sources.is_empty()),
            "Invalid Google calendar list."
        );
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut value: Preferences = get(&tx, "preferences")?;
            anyhow::ensure!(
                value.google_lifecycle.revision == expected.google_lifecycle.revision
                    && value.google_client_id == expected.google_client_id
                    && value.google_client_secret == expected.google_client_secret,
                "Google settings changed during sign-in. Reconnect with the current settings."
            );
            anyhow::ensure!(
                !value.google_lifecycle.cleanup_pending,
                "Finish Google cleanup before reconnecting."
            );
            let previous_backups = value.clone();
            if let Some(identity) = drive_identity {
                anyhow::ensure!(
                    identity.starts_with("drive:") && identity.len() > 6,
                    "Invalid Google Drive identity."
                );
                if value.google_connection_id != identity
                    && value.backup_destination == BackupDestination::GoogleDrive
                {
                    value.last_backup = None;
                    value.backup_ready = false;
                }
                value.google_connection_id = identity;
            }
            if !grant.access.drive && value.backup_destination == BackupDestination::GoogleDrive {
                value.auto_backup = false;
                value.backup_ready = false;
            }
            let old_target = crate::backup::BackupTarget::from_preferences(&value);
            value.google_grant = grant;
            if old_target != crate::backup::BackupTarget::from_preferences(&value)
                && value.backup_destination == BackupDestination::GoogleDrive
            {
                value.last_backup = None;
                value.backup_ready = false;
            }
            value.google_lifecycle.revision = value
                .google_lifecycle
                .revision
                .checked_add(1)
                .context("Google connection revision overflow")?;
            value.google_lifecycle.disconnected = false;
            crate::backup::config::preserve_metadata(&previous_backups, &mut value);
            put(&tx, "preferences", &value)?;
            refresh_sources(&tx, sources)?;
            let revision = get(&tx, "preferences_revision")?;
            tx.commit()?;
            Ok(PreferenceSnapshot { revision, value })
        })
        .await
    }

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
            let previous_backups = prefs.clone();
            crate::backup::config::preserve_metadata(&previous_backups, &mut prefs);
            profile_sync::pause(&tx)?;
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

// Used inside both refresh and connection activation transactions.
pub(super) fn refresh_sources(
    c: &rusqlite::Connection,
    sources: Vec<CalendarSource>,
) -> anyhow::Result<()> {
    let mut archived: std::collections::HashSet<String> = get(c, "google_archived")?;
    let mut current: Vec<CalendarSource> = get(c, "calendars")?;
    // Keep cached events when access disappears, but never preserve a
    // stale grant to edit a calendar absent from a complete listing.
    for source in &mut current {
        if source.kind == CalendarKind::Google {
            source.access = CalendarAccess::READ_ONLY;
            archived.insert(source.id.clone());
        }
    }
    let prefs: Preferences = get(c, "preferences")?;
    for mut source in sources {
        if !prefs.google_grant.access.calendar_write_allowed() {
            source.access = CalendarAccess::READ_ONLY;
        }
        if connections::removed(c, ConnectionKind::Calendar, &source.id)?.is_some() {
            continue;
        }
        current.retain(|s| s.id != source.id);
        archived.remove(&source.id);
        current.push(source);
    }
    put(c, "calendars", &current)?;
    put(c, "google_archived", &archived)?;
    connections::changed(c)?;
    Ok(())
}
