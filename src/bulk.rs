//! Compact, durable mail-operation metadata. Bodies and provider credentials never
//! cross the bulk-control channel or enter an operation receipt.
use crate::{
    mail_actions::{Flags, MoveReceipt},
    model::Mail,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Action {
    Move {
        account: Option<String>,
        folder: String,
    },
    Flags(Flags),
}
impl Action {
    pub fn apply(&self, mail: &mut Mail) {
        match self {
            Self::Move { account, folder } => {
                if let Some(account) = account {
                    mail.account_id.clone_from(account);
                }
                mail.folder.clone_from(folder);
            }
            Self::Flags(flags) => flags.apply(mail),
        }
    }
    pub fn review_label(&self, count: usize) -> String {
        let mail = format!(
            "{count} {}",
            if count == 1 { "message" } else { "messages" }
        );
        match self {
            Self::Move { folder, .. } if folder.eq_ignore_ascii_case("Archive") => {
                format!("Archive {mail}?")
            }
            Self::Move { folder, .. } => format!(
                "Move {mail} to {}?",
                if folder.eq_ignore_ascii_case("INBOX") {
                    "Inbox"
                } else {
                    folder
                }
            ),
            Self::Flags(flags) if flags.unread == Some(false) => format!("Mark {mail} as read?"),
            Self::Flags(flags) if flags.unread == Some(true) => format!("Mark {mail} as unread?"),
            Self::Flags(flags) if flags.starred == Some(true) => format!("Flag {mail}?"),
            Self::Flags(_) => format!("Remove flags from {mail}?"),
        }
    }
    pub fn label(&self) -> String {
        match self {
            Self::Move { folder, .. } if folder.eq_ignore_ascii_case("Archive") => "Archive".into(),
            Self::Move { folder, .. } if folder.eq_ignore_ascii_case("Trash") => {
                "Move to Trash".into()
            }
            Self::Move { folder, .. } => format!(
                "Move to {}",
                if folder.eq_ignore_ascii_case("INBOX") {
                    "Inbox"
                } else {
                    folder
                }
            ),
            Self::Flags(flags) if flags.unread == Some(false) => "Mark as read".into(),
            Self::Flags(flags) if flags.unread == Some(true) => "Mark as unread".into(),
            Self::Flags(flags) if flags.starred == Some(true) => "Flag".into(),
            Self::Flags(_) => "Remove flag".into(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Receipt {
    Move(Box<MoveReceipt>),
    Flags { before: Flags, after: Flags },
    Unchanged,
}
#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub action: Action,
    pub undo_requested: bool,
    pub paused: bool,
    pub total: usize,
    pub remaining: usize,
    pub running: usize,
    pub completed: usize,
    pub restored: usize,
    pub failed: usize,
    pub uncertain: usize,
    pub cancelled: usize,
    pub revision: u64,
}
#[derive(Debug, Clone)]
pub struct Item {
    pub job: String,
    pub position: u64,
    pub id: String,
    pub original: Option<Mail>,
    pub undo: bool,
    pub status: String,
    pub receipt: Option<Receipt>,
    pub error: Option<String>,
}
