//! Portable calendar mutation identities and durable outcomes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(feature = "http")]
pub mod http;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub name: String,
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub all_day: bool,
    pub etag: Option<String>,
    pub remote_url: Option<String>,
}

impl Event {
    pub fn is_bounded(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= 1024
            && !self.source_id.is_empty()
            && self.source_id.len() <= 1024
            && self.title.len() <= 1024
            && self.location.len() <= 4096
            && self.description.len() <= 65_536
            && self.etag.as_ref().is_none_or(|value| value.len() <= 4096)
            && self
                .remote_url
                .as_ref()
                .is_none_or(|value| value.len() <= 8192)
    }

    pub fn key(&self) -> String {
        format!("{}:{}{}", self.source_id.len(), self.source_id, self.id)
    }

    pub fn is_create(&self) -> bool {
        self.etag.is_none() && self.remote_url.is_none()
    }

    pub fn same_content(&self, other: &Self) -> bool {
        self.source_id == other.source_id
            && self.title == other.title
            && self.start == other.start
            && self.end == other.end
            && self.location == other.location
            && self.description == other.description
            && self.all_day == other.all_day
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mutation {
    Save { before: Option<Event>, after: Event },
    Delete { before: Event },
}

impl Mutation {
    pub fn source_id(&self) -> &str {
        match self {
            Self::Save { after, .. } => &after.source_id,
            Self::Delete { before } => &before.source_id,
        }
    }

    pub fn expected_etag(&self) -> Option<&str> {
        match self {
            Self::Save { before, .. } => before.as_ref().and_then(|event| event.etag.as_deref()),
            Self::Delete { before } => before.etag.as_deref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub request_id: String,
    pub before: Option<Event>,
    pub after: Option<Event>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    Waiting,
    Rejected,
    Uncertain,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ProviderFailure {
    pub kind: FailureKind,
    pub message: String,
}

impl ProviderFailure {
    pub fn waiting(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::Waiting,
            message: message.into(),
        }
    }

    pub fn rejected(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::Rejected,
            message: message.into(),
        }
    }

    pub fn uncertain(message: impl Into<String>) -> Self {
        Self {
            kind: FailureKind::Uncertain,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event() -> Event {
        Event {
            id: "event".into(),
            source_id: "primary".into(),
            title: "Review".into(),
            start: DateTime::from_timestamp(1, 0).expect("time"),
            end: DateTime::from_timestamp(2, 0).expect("time"),
            location: String::new(),
            description: String::new(),
            all_day: false,
            etag: Some("etag".into()),
            remote_url: Some("event".into()),
        }
    }

    #[test]
    fn mutation_retains_exact_concurrency_identity() {
        let before = event();
        let mut after = before.clone();
        after.title = "Updated".into();
        let mutation = Mutation::Save {
            before: Some(before.clone()),
            after,
        };
        assert_eq!(mutation.source_id(), "primary");
        assert_eq!(mutation.expected_etag(), Some("etag"));
        assert_eq!(
            serde_json::from_str::<Mutation>(&serde_json::to_string(&mutation).expect("json"))
                .expect("json"),
            mutation
        );
    }
}
