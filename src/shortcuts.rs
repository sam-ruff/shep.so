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
    Delete,
    Inbox,
    Find,
    Forward,
    Print,
}
impl Action {
    pub const ALL: [Self; 20] = [
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
        Self::Delete,
        Self::Inbox,
        Self::Find,
        Self::Forward,
        Self::Print,
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
            Self::Delete => "Move to Trash",
            Self::Inbox => "Go to Inbox (sidebar)",
            Self::Find => "Find in message",
            Self::Forward => "Forward message",
            Self::Print => "Print message",
        }
    }
}

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
                Ok(Self(primary, secondary))
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
                    "Mod+D",
                    "I",
                    "Mod+F",
                    "F",
                    "Mod+P",
                ])
                .map(|(a, k)| (a, k.into()))
                .collect(),
            BTreeMap::from([(Action::Archive, "Delete".into())]),
        )
    }
}
impl Keymap {
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
                    && key != "MOD+A",
                "Escape, Tab and Select all are reserved."
            );
            anyhow::ensure!(
                seen.insert(key.clone()),
                "Shortcut {key} is assigned more than once."
            );
        }
        Ok(())
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
