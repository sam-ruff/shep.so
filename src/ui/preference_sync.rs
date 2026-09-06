use crate::{model::Preferences, store::PreferenceSnapshot};

/// Tracks local edits independently from the monotonically versioned store.
#[derive(Default)]
pub(super) struct PreferenceSync {
    local: u64,
    acknowledged: u64,
    pub saved: PreferenceSnapshot,
}

impl PreferenceSync {
    pub fn new(saved: PreferenceSnapshot) -> Self {
        Self {
            saved,
            ..Default::default()
        }
    }

    pub fn changed(&mut self) -> u64 {
        self.local += 1;
        self.local
    }

    pub fn dirty(&self) -> bool {
        self.local != self.acknowledged
    }

    pub fn observe(&mut self, snapshot: PreferenceSnapshot, live: &mut Preferences) {
        if snapshot.revision >= self.saved.revision {
            self.saved = snapshot;
        }
        if self.dirty() {
            live.last_backup = self.saved.value.last_backup;
        } else {
            *live = self.saved.value.clone();
        }
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
}
