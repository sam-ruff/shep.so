//! Bounded, destination-scoped observations for a manual multi-destination run.
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Status {
    SavingPreferences,
    Queued,
    Preparing,
    Uploading,
    Finishing,
    Saved,
    SavedWithWarning(String),
    NeedsSetup(String),
    Failed(String),
}
impl Status {
    pub fn pending(&self) -> bool {
        matches!(
            self,
            Self::SavingPreferences
                | Self::Queued
                | Self::Preparing
                | Self::Uploading
                | Self::Finishing
        )
    }
    pub fn label(&self) -> &str {
        match self {
            Self::SavingPreferences => "Saving settings…",
            Self::Queued => "Queued…",
            Self::Preparing => "Preparing copy…",
            Self::Uploading => "Uploading…",
            Self::Finishing => "Saving receipt and keeping rolling copies…",
            Self::Saved => "Backup saved",
            Self::SavedWithWarning(message) | Self::NeedsSetup(message) | Self::Failed(message) => {
                message
            }
        }
    }
}
