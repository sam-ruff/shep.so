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
}
impl Action {
    pub const ALL: [Self; 15] = [
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Keymap(pub BTreeMap<Action, String>);
impl<'de> Deserialize<'de> for Keymap {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bindings = BTreeMap::<Action, String>::deserialize(deserializer)?;
        let mut keys = Self::default();
        keys.0.extend(bindings);
        Ok(keys)
    }
}
impl Default for Keymap {
    fn default() -> Self {
        Self(
            Action::ALL
                .into_iter()
                .zip([
                    "M", "C", "R", "E", "S", "Mod+K", "Mod+R", "J", "K", "Mod+1", "Mod+2", "Mod+,",
                    "Enter", "Escape", "Shift+R",
                ])
                .map(|(a, k)| (a, k.into()))
                .collect(),
        )
    }
}
impl Keymap {
    pub fn resolve(&self, chord: &str) -> Option<Action> {
        self.0
            .iter()
            .find(|(_, k)| k.eq_ignore_ascii_case(chord))
            .map(|(a, _)| *a)
    }
    pub fn key(&self, action: Action) -> &str {
        self.0.get(&action).map(String::as_str).unwrap_or("")
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        let mut seen = std::collections::HashSet::new();
        for action in Action::ALL {
            let key = self.key(action).to_uppercase();
            anyhow::ensure!(
                !key.is_empty()
                    && (key != "ESCAPE" || action == Action::ClosePreview)
                    && key != "TAB",
                "Every action needs a shortcut; Escape and Tab are reserved."
            );
            anyhow::ensure!(
                seen.insert(key.clone()),
                "Shortcut {key} is assigned more than once."
            );
        }
        Ok(())
    }
    pub fn remap(&mut self, action: Action, chord: String) -> anyhow::Result<()> {
        let mut next = self.clone();
        next.0.insert(action, chord);
        next.validate()?;
        *self = next;
        Ok(())
    }
}
