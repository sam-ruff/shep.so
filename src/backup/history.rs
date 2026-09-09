//! Bounded local attempt history. Upload ownership remains in the upload journal.
use super::{BackupTarget, format};
use serde::{Deserialize, Serialize};

pub const PAGE: usize = 20;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Outcome {
    Unfinished,
    NeedsReview,
    Failed,
    Saved,
    SavedWithWarning,
    Recovered,
}
impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unfinished => "Unfinished attempt",
            Self::NeedsReview => "Upload needs review",
            Self::Failed => "Could not back up",
            Self::Saved => "Copy saved",
            Self::SavedWithWarning => "Copy saved · attention needed",
            Self::Recovered => "Recovered by a later attempt",
        }
    }
    pub fn attention(self) -> bool {
        matches!(
            self,
            Self::Unfinished | Self::NeedsReview | Self::Failed | Self::SavedWithWarning
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub target: BackupTarget,
    pub name: String,
    pub started: i64,
    pub finished: Option<i64>,
    pub format: format::Options,
    pub copy: Option<String>,
    pub outcome: Outcome,
    pub detail: String,
}
impl Entry {
    pub fn new(target: BackupTarget, name: String, format: format::Options) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            target,
            name,
            started: chrono::Utc::now().timestamp_millis(),
            finished: None,
            format,
            copy: None,
            outcome: Outcome::Unfinished,
            detail: String::new(),
        }
    }
}
