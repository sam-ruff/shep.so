use iced_cube::rich_text_editor::{Shortcut, Shortcuts};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Action {
    Move,
    Compose,
    Reply,
    Archive,
    Star,
    Search,
    Sync,
    Next,
    Previous,
    Mail,
    Calendar,
    Settings,
    OpenMessage,
    ClosePreview,
    ReplyAll,
    SelectAll,
    Delete,
    Inbox,
    Find,
    Forward,
    Print,
    Activity,
    Bold,
    Italic,
    Underline,
    Strikethrough,
    BulletList,
    NumberedList,
    Link,
    ClearFormatting,
}
impl Action {
    /// Formatting actions and the editor shortcut each one binds.
    pub const FORMATTING: [(Self, Shortcut); 8] = [
        (Self::Bold, Shortcut::Bold),
        (Self::Italic, Shortcut::Italic),
        (Self::Underline, Shortcut::Underline),
        (Self::Strikethrough, Shortcut::Strikethrough),
        (Self::BulletList, Shortcut::BulletedList),
        (Self::NumberedList, Shortcut::NumberedList),
        (Self::Link, Shortcut::Link),
        (Self::ClearFormatting, Shortcut::ClearFormatting),
    ];

    /// Settings list order. Formatting sits between Reply all and Select all,
    /// rows no native test clicks, so saved coordinates measured from the top
    /// (to Sync) and from the scrolled bottom (from Select all) still hold.
    pub const ALL: [Self; 30] = [
        Self::Move,
        Self::Compose,
        Self::Reply,
        Self::Archive,
        Self::Star,
        Self::Search,
        Self::Sync,
        Self::Next,
        Self::Previous,
        Self::Mail,
        Self::Calendar,
        Self::Settings,
        Self::OpenMessage,
        Self::ClosePreview,
        Self::ReplyAll,
        Self::Bold,
        Self::Italic,
        Self::Underline,
        Self::Strikethrough,
        Self::BulletList,
        Self::NumberedList,
        Self::Link,
        Self::ClearFormatting,
        Self::SelectAll,
        Self::Delete,
        Self::Inbox,
        Self::Find,
        Self::Forward,
        Self::Print,
        Self::Activity,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Move => "Move to folder",
            Self::Compose => "Compose a message",
            Self::Reply => "Reply",
            Self::Archive => "Archive",
            Self::Star => "Toggle flag",
            Self::Search => "Search mail",
            Self::Sync => "Sync accounts",
            Self::Next => "Next message",
            Self::Previous => "Previous message",
            Self::Mail => "Open mail",
            Self::Calendar => "Open calendar",
            Self::Settings => "Open settings",
            Self::OpenMessage => "Open full-window reader",
            Self::ClosePreview => "Close full-window reader",
            Self::ReplyAll => "Reply to all",
            Self::SelectAll => "Select all messages (list)",
            Self::Delete => "Move to Trash",
            Self::Inbox => "Go to Inbox (sidebar)",
            Self::Find => "Find in message",
            Self::Forward => "Forward message",
            Self::Print => "Print message",
            Self::Activity => "Open Activity",
            Self::Bold => "Bold",
            Self::Italic => "Italic",
            Self::Underline => "Underline",
            Self::Strikethrough => "Strikethrough",
            Self::BulletList => "Bulleted list",
            Self::NumberedList => "Numbered list",
            Self::Link => "Insert link",
            Self::ClearFormatting => "Clear formatting",
        }
    }

    /// Formatting shortcuts act only while the message body has focus.
    pub fn formatting(self) -> bool {
        Self::FORMATTING.iter().any(|(action, _)| *action == self)
    }
}

/// Shown above the formatting rows in the shortcut settings.
pub const FORMATTING_HEADING: &str = "Formatting, while writing a message";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Primary,
    Secondary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap(pub BTreeMap<Action, String>, pub BTreeMap<Action, String>);
#[derive(Serialize, Deserialize)]
struct SavedKeys {
    version: u8,
    primary: BTreeMap<Action, String>,
    secondary: BTreeMap<Action, String>,
}
impl Serialize for Keymap {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        SavedKeys {
            version: 2,
            primary: self.0.clone(),
            secondary: self.1.clone(),
        }
        .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for Keymap {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Saved {
            Current(SavedKeys),
            Legacy(BTreeMap<Action, String>),
        }
        match Saved::deserialize(deserializer)? {
            Saved::Current(saved) => {
                if saved.version != 2 {
                    return Err(serde::de::Error::custom(
                        "Unsupported shortcut settings version",
                    ));
                }
                let mut keys = Self(saved.primary, saved.secondary);
                for (action, key) in Self::default().0 {
                    if !keys.0.contains_key(&action) {
                        let key = if keys.resolve(&key).is_none() {
                            key
                        } else {
                            String::new()
                        };
                        keys.0.insert(action, key);
                    }
                }
                keys.add_refresh_secondary();
                Ok(keys)
            }
            Saved::Legacy(mut primary) => {
                // Migrate the old Archive default, without taking a custom key
                // from any other action. Non-default mappings remain intact.
                let unused = |keys: &BTreeMap<Action, String>, key: &str| {
                    !keys.values().any(|k| k.eq_ignore_ascii_case(key))
                };
                if primary.get(&Action::Archive).is_none_or(|k| k == "E")
                    && unused(&primary, "Backspace")
                {
                    primary.insert(Action::Archive, "Backspace".into());
                }
                for (action, key) in Self::default().0 {
                    if !primary.contains_key(&action) {
                        let key = if unused(&primary, &key) {
                            key
                        } else {
                            String::new()
                        };
                        primary.insert(action, key);
                    }
                }
                let secondary = if unused(&primary, "Delete") {
                    BTreeMap::from([(Action::Archive, "Delete".into())])
                } else {
                    BTreeMap::new()
                };
                let mut keys = Self(primary, secondary);
                keys.add_refresh_secondary();
                Ok(keys)
            }
        }
    }
}
impl Default for Keymap {
    fn default() -> Self {
        Self(
            Action::ALL
                .into_iter()
                .zip([
                    "M",
                    "C",
                    "R",
                    "Backspace",
                    "S",
                    "Mod+K",
                    "Mod+R",
                    "J",
                    "K",
                    "Mod+1",
                    "Mod+2",
                    "Mod+,",
                    "Enter",
                    "Escape",
                    "Shift+R",
                    "Mod+B",
                    "Mod+I",
                    "Mod+U",
                    "Mod+Shift+X",
                    "Mod+Shift+8",
                    "Mod+Shift+7",
                    // Mod+K stays with Search.
                    "",
                    "Mod+\\",
                    "Mod+A",
                    "Mod+D",
                    "I",
                    "Mod+F",
                    "F",
                    "Mod+P",
                    "",
                ])
                .map(|(a, k)| (a, k.into()))
                .collect(),
            BTreeMap::from([
                (Action::Archive, "Delete".into()),
                (Action::Sync, "F5".into()),
            ]),
        )
    }
}
impl Keymap {
    fn add_refresh_secondary(&mut self) {
        // An absent slot predates this default. An empty saved slot is an
        // explicit clear, and an entirely disabled action must stay disabled.
        if !self.1.contains_key(&Action::Sync) {
            let key = if !self.key(Action::Sync).is_empty() && self.resolve("F5").is_none() {
                "F5"
            } else {
                ""
            };
            // Persist a blank decision too, so removing another action's F5
            // later cannot silently enable refresh. The v2 format is unchanged.
            self.1.insert(Action::Sync, key.into());
        }
    }
    pub fn resolve(&self, chord: &str) -> Option<Action> {
        self.0
            .iter()
            .chain(&self.1)
            .find(|(_, k)| !k.is_empty() && k.eq_ignore_ascii_case(chord))
            .map(|(a, _)| *a)
    }
    pub fn key(&self, action: Action) -> &str {
        self.binding(action, Slot::Primary)
    }
    pub fn binding(&self, action: Action, slot: Slot) -> &str {
        match slot {
            Slot::Primary => &self.0,
            Slot::Secondary => &self.1,
        }
        .get(&action)
        .map(String::as_str)
        .unwrap_or("")
    }
    pub fn label(&self, action: Action) -> String {
        [self.key(action), self.binding(action, Slot::Secondary)]
            .into_iter()
            .filter(|k| !k.is_empty())
            .collect::<Vec<_>>()
            .join(" / ")
            .replace(
                "Mod",
                if cfg!(target_os = "macos") {
                    "⌘"
                } else {
                    "Ctrl"
                },
            )
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut seen = std::collections::HashSet::new();
        for (action, key) in self.0.iter().chain(&self.1) {
            let key = key.to_uppercase();
            if key.is_empty() {
                continue;
            }
            anyhow::ensure!(
                (key != "ESCAPE" || *action == Action::ClosePreview)
                    && key != "TAB"
                    && (key != "MOD+A" || *action == Action::SelectAll),
                "Escape and Ctrl+A are reserved for their matching actions; Tab switches focus."
            );
            anyhow::ensure!(
                !action.formatting()
                    || ["MOD+", "CTRL+", "ALT+"].iter().any(|m| key.starts_with(m)),
                "Formatting shortcuts need Ctrl, Alt or Command, so typing in the message is never interrupted."
            );
            anyhow::ensure!(
                seen.insert(key.clone()),
                "Shortcut {key} is assigned more than once."
            );
        }
        Ok(())
    }
    /// Turns every formatting shortcut off, in both slots.
    pub fn clear_formatting(&mut self) {
        for (action, _) in Action::FORMATTING {
            self.0.insert(action, String::new());
            self.1.insert(action, String::new());
        }
    }

    /// The rich text editor's keys and the shortcuts it handles: undo, redo
    /// and word deletion as iced-cube binds them, and each formatting action
    /// on its chords here. Escape is left to the app, which closes the
    /// composer, and a formatting action with no chord is off.
    pub fn editor(&self) -> (iced_cube::keys::Keymap<Shortcut>, Shortcuts) {
        let mut chords = iced_cube::rich_text_editor::default_keymap();
        let mut on = Shortcuts::all().without(Shortcut::EscapeUnfocus);
        for (action, shortcut) in Action::FORMATTING {
            chords = chords.unbind_action(&shortcut);
            let bound: Vec<iced_cube::keys::Chord> = [Slot::Primary, Slot::Secondary]
                .into_iter()
                .filter_map(|slot| self.binding(action, slot).parse().ok())
                .collect();
            if bound.is_empty() {
                on = on.without(shortcut);
            }
            for chord in bound {
                chords = chords.bind(chord, shortcut);
            }
        }
        (chords, on)
    }
    pub fn remap(&mut self, action: Action, chord: String) -> anyhow::Result<()> {
        self.remap_slot(action, Slot::Primary, chord)
    }
    pub fn remap_slot(&mut self, action: Action, slot: Slot, chord: String) -> anyhow::Result<()> {
        let mut next = self.clone();
        match slot {
            Slot::Primary => &mut next.0,
            Slot::Secondary => &mut next.1,
        }
        .insert(action, chord);
        next.validate()?;
        *self = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::keyboard::{Key, Modifiers};

    fn resolve(keys: &Keymap, key: &str, modifiers: Modifiers) -> Option<Shortcut> {
        keys.editor()
            .0
            .resolve(&Key::Character(key.into()), modifiers)
    }

    #[test]
    fn default_formatting_chords_reach_the_editor_and_leave_search_alone() {
        let keys = Keymap::default();
        keys.validate().expect("defaults are valid");
        let command = Modifiers::COMMAND;
        assert_eq!(resolve(&keys, "b", command), Some(Shortcut::Bold));
        assert_eq!(resolve(&keys, "i", command), Some(Shortcut::Italic));
        assert_eq!(resolve(&keys, "u", command), Some(Shortcut::Underline));
        assert_eq!(
            resolve(&keys, "x", command | Modifiers::SHIFT),
            Some(Shortcut::Strikethrough)
        );
        assert_eq!(
            resolve(&keys, "\\", command),
            Some(Shortcut::ClearFormatting)
        );
        assert_eq!(resolve(&keys, "z", command), Some(Shortcut::Undo));
        // Mod+K is Search, so the link button has no default chord.
        assert_eq!(resolve(&keys, "k", command), None);
        let (_, on) = keys.editor();
        assert!(!on.contains(Shortcut::Link));
        assert!(!on.contains(Shortcut::EscapeUnfocus));
        assert!(on.contains(Shortcut::Bold) && on.contains(Shortcut::Undo));
    }

    #[test]
    fn a_remapped_formatting_chord_moves_and_a_cleared_one_turns_off() {
        let mut keys = Keymap::default();
        keys.remap(Action::Bold, "Alt+B".into()).expect("remap");
        keys.remap_slot(Action::Bold, Slot::Secondary, "Mod+Shift+B".into())
            .expect("secondary");
        keys.remap(Action::Link, "Mod+L".into()).expect("link");
        keys.remap(Action::Italic, String::new()).expect("clear");
        let command = Modifiers::COMMAND;
        assert_eq!(resolve(&keys, "b", command), None);
        assert_eq!(resolve(&keys, "b", Modifiers::ALT), Some(Shortcut::Bold));
        assert_eq!(
            resolve(&keys, "b", command | Modifiers::SHIFT),
            Some(Shortcut::Bold)
        );
        assert_eq!(resolve(&keys, "l", command), Some(Shortcut::Link));
        assert_eq!(resolve(&keys, "i", command), None);
        let (_, on) = keys.editor();
        assert!(on.contains(Shortcut::Link));
        assert!(!on.contains(Shortcut::Italic));
    }

    #[test]
    fn formatting_shortcuts_need_a_modifier_and_cannot_take_another_key() {
        let mut keys = Keymap::default();
        assert!(keys.remap(Action::Bold, "B".into()).is_err());
        assert!(keys.remap(Action::Bold, "Shift+B".into()).is_err());
        assert!(keys.remap(Action::Bold, "Mod+K".into()).is_err());
        assert_eq!(keys.key(Action::Bold), "Mod+B");
    }

    #[test]
    fn turning_formatting_off_keeps_editing_and_survives_saving() {
        let mut keys = Keymap::default();
        keys.clear_formatting();
        let (chords, on) = keys.editor();
        for (_, shortcut) in Action::FORMATTING {
            assert!(!on.contains(shortcut), "{shortcut:?}");
            assert!(chords.chords(&shortcut).is_empty(), "{shortcut:?}");
        }
        assert!(on.contains(Shortcut::Undo) && on.contains(Shortcut::DeleteWordBackward));
        let saved = serde_json::to_string(&keys).expect("save");
        let read: Keymap = serde_json::from_str(&saved).expect("read");
        assert_eq!(read, keys);
        assert!(read.editor().1 == on);
        assert_eq!(read.key(Action::Search), "Mod+K");
    }

    #[test]
    fn saved_keymaps_from_before_formatting_gain_the_defaults_without_conflicts() {
        let mut primary: BTreeMap<Action, String> = Keymap::default()
            .0
            .into_iter()
            .filter(|(action, _)| !action.formatting())
            .collect();
        primary.insert(Action::Activity, "Mod+B".into());
        let saved = serde_json::json!({"version": 2, "primary": primary, "secondary": {}});
        let keys: Keymap = serde_json::from_value(saved).expect("read");
        assert_eq!(keys.key(Action::Activity), "Mod+B");
        assert_eq!(keys.key(Action::Bold), "");
        assert_eq!(keys.key(Action::Italic), "Mod+I");
        keys.validate().expect("valid");
    }
}
