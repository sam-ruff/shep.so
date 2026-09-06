use crate::model::Mail;

/// Only the fields explicitly changed by the user are written to the server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flags {
    pub unread: Option<bool>,
    pub starred: Option<bool>,
}

impl Flags {
    pub fn between(before: &Mail, after: &Mail) -> Self {
        Self {
            unread: (before.unread != after.unread).then_some(after.unread),
            starred: (before.starred != after.starred).then_some(after.starred),
        }
    }
    pub fn apply(self, mail: &mut Mail) {
        if let Some(value) = self.unread {
            mail.unread = value;
        }
        if let Some(value) = self.starred {
            mail.starred = value;
        }
    }
    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}
