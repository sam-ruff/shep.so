//! Track actual portable preference intent, independently of whole-window saves.
use crate::{model::Preferences, profiles::preferences::export};
use serde_json::Value;
use shep_profile_core::SettingKey;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub(super) struct Edits {
    seen: Option<BTreeMap<SettingKey, Value>>,
    pub versions: BTreeMap<SettingKey, u64>,
    sent: BTreeMap<SettingKey, u64>,
    serial: u64,
}
impl Edits {
    pub fn prepare(
        &mut self,
        requested: &Preferences,
        saved: &Preferences,
    ) -> BTreeSet<SettingKey> {
        let current = export(requested).expect("typed portable preferences");
        let seen = self
            .seen
            .get_or_insert_with(|| export(saved).expect("typed saved preferences"));
        for (key, value) in &current {
            if seen.get(key) != Some(value) {
                self.serial += 1;
                self.versions.insert(*key, self.serial);
            }
        }
        *seen = current;
        self.versions
            .iter()
            .filter_map(|(key, version)| {
                (*version > self.sent.get(key).copied().unwrap_or(0)).then_some(*key)
            })
            .collect()
    }
    pub fn accepted(&mut self) {
        self.sent = self.versions.clone();
    }
    pub fn observed(&mut self, live: &Preferences) {
        self.seen = Some(export(live).expect("typed preferences"));
    }
    pub fn remote(&mut self, key: SettingKey, value: Value) {
        if let Some(seen) = &mut self.seen {
            seen.insert(key, value);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Appearance;
    #[test]
    fn rejected_and_reverted_edits_remain_explicit_and_unrelated_saves_have_no_portable_patch() {
        let saved = Preferences::default();
        let mut local = saved.clone();
        let mut edits = Edits::default();
        assert!(edits.prepare(&local, &saved).is_empty());
        edits.accepted();
        local.appearance = Appearance::Dark;
        assert_eq!(
            edits.prepare(&local, &saved),
            BTreeSet::from([SettingKey::Appearance])
        );
        let first = edits.versions[&SettingKey::Appearance];
        // The queue rejects this save, then the user changes back.
        local.appearance = saved.appearance;
        assert_eq!(
            edits.prepare(&local, &saved),
            BTreeSet::from([SettingKey::Appearance])
        );
        assert!(edits.versions[&SettingKey::Appearance] > first);
        edits.accepted();
        local.reader_font_size = 22;
        assert!(edits.prepare(&local, &saved).is_empty());
        local.appearance = Appearance::Light;
        edits.remote(SettingKey::Appearance, serde_json::json!("Light"));
        assert!(edits.prepare(&local, &saved).is_empty());
    }
}

// Effects belong to accepting the canonical values, whichever queue supplied them.
#[derive(Clone, Copy)]
pub(super) struct Effects {
    unified: bool,
    grouped: bool,
    replies: crate::model::ReplyDisplay,
    images: crate::model::ImagePolicy,
}
impl From<&Preferences> for Effects {
    fn from(p: &Preferences) -> Self {
        Self {
            unified: p.unified_inbox,
            grouped: p.group_conversations,
            replies: p.reply_display,
            images: p.image_policy,
        }
    }
}
impl super::App {
    pub(super) fn apply_profile_preference_effects(&mut self, previous: Effects) {
        if previous.unified != self.preferences.unified_inbox {
            self.query.folders = None;
            self.query.account = if self.preferences.unified_inbox {
                None
            } else {
                self.workspace.accounts.first().map(|a| a.id.clone())
            };
            self.request_page();
        }
        if previous.grouped != self.preferences.group_conversations {
            self.conversation.page = Default::default();
            if let Some(id) = self.selected.clone() {
                self.focus_conversation_message(id);
            }
            self.request_conversation(None);
        }
        if previous.replies != self.preferences.reply_display {
            self.expanded_replies.clear();
        }
        if previous.images != self.preferences.image_policy {
            self.load_remote_images();
        }
    }
}
