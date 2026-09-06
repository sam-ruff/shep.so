//! Calendar fixtures using the shared provider HTTP harness.
pub use crate::providers::test_http::{Reply, Server, client};

pub fn source(url: &url::Url) -> crate::model::CalendarSource {
    crate::model::CalendarSource {
        access: Default::default(),
        id: "test-calendar".into(),
        name: "Local test calendar".into(),
        kind: crate::model::CalendarKind::CalDav,
        url: url.to_string(),
        username: "test-user".into(),
    }
}
pub fn event() -> crate::model::CalendarEvent {
    let start = chrono::DateTime::parse_from_rfc3339("2026-09-06T09:00:00Z")
        .unwrap()
        .to_utc();
    crate::model::CalendarEvent {
        id: "d4f68a2e-6c8f-4c32-b742-d19136986526".into(),
        source_id: "test-calendar".into(),
        title: "Morning walk".into(),
        start,
        end: start + chrono::Duration::hours(1),
        location: "Park".into(),
        description: "Bring water".into(),
        all_day: false,
        etag: None,
        remote_url: None,
    }
}
