use crate::{model::Preferences, store::PreferenceSnapshot};

/// Tracks local edits independently from the monotonically versioned store.
pub(super) struct PreferenceSync {
    local: u64,
    acknowledged: u64,
    pub saved: PreferenceSnapshot,
    portable: crate::preference_edits::Tracker,
}

impl Default for PreferenceSync {
    fn default() -> Self {
        Self::new(PreferenceSnapshot::default())
    }
}

impl PreferenceSync {
    pub fn new(saved: PreferenceSnapshot) -> Self {
        Self {
            portable: crate::preference_edits::Tracker::new(&saved.value),
            saved,
            local: 0,
            acknowledged: 0,
        }
    }

    pub fn changed(&mut self) -> u64 {
        self.local += 1;
        self.local
    }

    pub fn generation(&self) -> u64 {
        self.local
    }

    pub fn dirty(&self) -> bool {
        self.local != self.acknowledged
    }

    pub fn write(&mut self, value: Preferences) -> crate::preference_edits::Write {
        self.portable.capture(&value, self.local);
        crate::preference_edits::Write {
            portable: self.portable.edits(&value, self.acknowledged),
            value,
        }
    }

    pub fn observe(&mut self, snapshot: PreferenceSnapshot, live: &mut Preferences) {
        self.portable.capture(live, self.local);
        if snapshot.revision >= self.saved.revision {
            self.saved = snapshot;
        }
        if self.dirty() {
            live.google_connection_id = self.saved.value.google_connection_id.clone();
            live.google_lifecycle = self.saved.value.google_lifecycle;
            live.google_grant = self.saved.value.google_grant.clone();
            if (live.google_lifecycle.disconnected || !live.google_grant.access.drive_allowed())
                && live.backup_destination == crate::model::BackupDestination::GoogleDrive
            {
                live.auto_backup = false;
            }
            let same_target = crate::backup::BackupTarget::from_preferences(live)
                == crate::backup::BackupTarget::from_preferences(&self.saved.value);
            live.last_backup = if same_target {
                self.saved.value.last_backup
            } else {
                None
            };
            live.backup_ready = same_target
                && self.saved.value.backup_ready
                && live.backup_format == self.saved.value.backup_format;
            crate::backup::config::preserve_metadata(&self.saved.value, live);
        } else {
            *live = self.saved.value.clone();
        }
        self.portable
            .observe(&self.saved.value, live, self.acknowledged);
    }

    pub fn acknowledge(
        &mut self,
        request: u64,
        snapshot: PreferenceSnapshot,
        live: &mut Preferences,
    ) {
        if request > self.local {
            return;
        }
        self.acknowledged = self.acknowledged.max(request);
        self.observe(snapshot, live);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Appearance;

    fn snapshot(revision: u64, value: &Preferences) -> PreferenceSnapshot {
        PreferenceSnapshot {
            revision,
            value: value.clone(),
        }
    }

    #[tokio::test]
    async fn palette_edits_preserve_each_other_role_through_queued_saves_and_reversions() {
        use crate::appearance::Rgb;
        let store = crate::store::Store::memory().unwrap();
        let original = Preferences::default();
        let mut live = original.clone();
        let mut sync = PreferenceSync::new(snapshot(0, &original));
        live.palettes.light.primary = Rgb::parse("#007F73").unwrap();
        let first = sync.changed();
        let first_write = sync.write(live.clone());
        let remote = store
            .update_preferences(|p| {
                p.appearance = Appearance::Dark;
                p.palettes.dark.background = Rgb::parse("#10221A").unwrap();
                p.palettes.light.flag = Rgb::parse("#FF0000").unwrap();
            })
            .await
            .unwrap();
        let expected_dark = remote.value.palettes.dark;
        let expected_flag = remote.value.palettes.light.flag;
        sync.observe(remote, &mut live);
        assert_eq!(live.palettes.light.primary.to_string(), "#007F73");
        assert_eq!(live.palettes.dark, expected_dark);
        assert_eq!(live.palettes.light.flag, expected_flag);
        assert_eq!(live.appearance, Appearance::Dark);
        live.palettes.light.primary = original.palettes.light.primary;
        let second = sync.changed();
        let second_write = sync.write(live.clone());
        let first_saved = store.save_preferences(first_write).await.unwrap();
        assert_eq!(first_saved.value.palettes.dark, expected_dark);
        sync.acknowledge(first, first_saved.clone(), &mut live);
        assert_eq!(live.palettes.light.primary, original.palettes.light.primary);
        let second_saved = store.save_preferences(second_write).await.unwrap();
        sync.acknowledge(second, second_saved.clone(), &mut live);
        sync.acknowledge(first, first_saved, &mut live);
        assert_eq!(live, second_saved.value);
        assert_eq!(live.palettes.dark, expected_dark);
        assert_eq!(live.palettes.light.flag, expected_flag);
        assert!(!sync.dirty());
    }

    #[tokio::test]
    async fn palette_changes_survive_an_unrelated_stale_native_save_retry() {
        let store = crate::store::Store::memory().unwrap();
        let original = Preferences::default();
        let mut live = original.clone();
        let mut sync = PreferenceSync::new(snapshot(0, &original));
        live.reader_split = 0.61;
        sync.changed();
        let write = sync.write(live.clone());
        store.save_preferences(write.clone()).await.unwrap();
        let current = store
            .update_preferences(|p| {
                p.palettes.light.primary = crate::appearance::Rgb::parse("#007F73").unwrap();
                p.palettes.dark.accent = crate::appearance::Rgb::parse("#33DDAA").unwrap();
            })
            .await
            .unwrap();
        sync.observe(current.clone(), &mut live);
        let saved = store.save_preferences(write).await.unwrap();
        assert_eq!(saved.value.palettes, current.value.palettes);
        assert_eq!(saved.value.reader_split, 0.61);
        assert_eq!(live.palettes, current.value.palettes);
    }

    #[tokio::test]
    async fn queued_native_save_merges_remote_settings_and_retains_explicit_reversion() {
        let store = crate::store::Store::memory().unwrap();
        let original = Preferences::default();
        let mut live = original.clone();
        let mut sync = PreferenceSync::new(snapshot(0, &original));
        live.appearance = Appearance::Dark;
        let first = sync.changed();
        let first_write = sync.write(live.clone());
        // Another device changes an unrelated field after this request queued.
        let remote = store
            .update_preferences(|p| p.unified_inbox = !p.unified_inbox)
            .await
            .unwrap();
        sync.observe(remote, &mut live);
        assert_eq!(live.unified_inbox, !original.unified_inbox);
        assert_eq!(live.appearance, Appearance::Dark);
        // Reverting is a new intent, even though the old cache value is System.
        live.appearance = original.appearance;
        let second = sync.changed();
        let second_write = sync.write(live.clone());
        let saved = store.save_preferences(first_write).await.unwrap();
        assert_eq!(saved.value.unified_inbox, !original.unified_inbox);
        sync.acknowledge(first, saved, &mut live);
        assert_eq!(live.appearance, original.appearance);
        let saved = store.save_preferences(second_write).await.unwrap();
        sync.acknowledge(second, saved.clone(), &mut live);
        assert_eq!(saved.value.appearance, original.appearance);
        assert_eq!(saved.value.unified_inbox, !original.unified_inbox);
        assert_eq!(live, saved.value);
        assert!(!sync.dirty());
    }

    #[tokio::test]
    async fn retrying_an_admitted_write_does_not_revert_untouched_remote_settings() {
        let store = crate::store::Store::memory().unwrap();
        let original = Preferences::default();
        let mut live = original.clone();
        let mut sync = PreferenceSync::new(snapshot(0, &original));
        live.reader_split = 0.61;
        sync.changed();
        let write = sync.write(live.clone());
        store.save_preferences(write.clone()).await.unwrap();
        let remote = store
            .update_preferences(|p| p.appearance = Appearance::Dark)
            .await
            .unwrap();
        sync.observe(remote, &mut live);
        assert_eq!(live.appearance, Appearance::Dark);
        let saved = store.save_preferences(write).await.unwrap();
        assert_eq!(saved.value.reader_split, 0.61);
        assert_eq!(saved.value.appearance, Appearance::Dark);
    }

    #[test]
    fn old_acknowledgments_and_workspaces_cannot_undo_newer_preferences() {
        let original = Preferences::default();
        let mut live = original.clone();
        let mut sync = PreferenceSync::new(snapshot(1, &original));
        live.appearance = Appearance::Dark;
        let first = sync.changed();
        let first_saved = snapshot(2, &live);
        live.reader_font_size = 22;
        let second = sync.changed();
        let second_saved = snapshot(3, &live);
        sync.acknowledge(first, first_saved.clone(), &mut live);
        assert_eq!(live.reader_font_size, 22);
        assert!(sync.dirty());
        sync.acknowledge(second, second_saved, &mut live);
        assert!(!sync.dirty());
        sync.observe(snapshot(1, &original), &mut live);
        sync.acknowledge(first, first_saved, &mut live);
        assert_eq!(live.appearance, Appearance::Dark);
        assert_eq!(live.reader_font_size, 22);
        assert_eq!(sync.saved.revision, 3);
    }

    #[test]
    fn dragging_between_save_and_ack_keeps_the_latest_unsaved_split() {
        let mut live = Preferences::default();
        let mut sync = PreferenceSync::default();
        live.reader_split = 0.4;
        let sent = sync.changed();
        let saved = snapshot(1, &live);
        live.reader_split = 0.6;
        sync.changed();
        sync.acknowledge(sent, saved, &mut live);
        assert!(sync.dirty());
        assert_eq!(live.reader_split, 0.6);
        assert_eq!(sync.saved.value.reader_split, 0.4);
        let latest = sync.changed();
        sync.acknowledge(latest, snapshot(2, &live), &mut live);
        assert!(!sync.dirty());
        assert_eq!(sync.saved.value.reader_split, 0.6);
    }

    #[test]
    fn backup_metadata_survives_late_save_ack_without_losing_local_changes() {
        let mut live = Preferences::default();
        let mut sync = PreferenceSync::default();
        live.reader_font_size = 20;
        let request = sync.changed();
        let saved = snapshot(1, &live);
        let mut newer = live.clone();
        newer.last_backup = Some(42);
        sync.observe(snapshot(2, &newer), &mut live);
        assert!(sync.dirty());
        assert_eq!(live.reader_font_size, 20);
        sync.acknowledge(request, saved, &mut live);
        assert!(!sync.dirty());
        assert_eq!(live.last_backup, Some(42));
        assert_eq!(live.reader_font_size, 20);
    }

    #[test]
    fn backup_metadata_from_the_saved_destination_does_not_appear_on_an_unsaved_one() {
        let original = Preferences {
            backup_folder: "/first".into(),
            last_backup: Some(42),
            backup_ready: true,
            ..Default::default()
        };
        let mut live = original.clone();
        let mut sync = PreferenceSync::new(snapshot(1, &original));
        live.backup_folder = "/second".into();
        sync.changed();
        sync.observe(snapshot(2, &original), &mut live);
        assert_eq!(live.backup_folder, "/second");
        assert!(live.last_backup.is_none() && !live.backup_ready);
    }
}
