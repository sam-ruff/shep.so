pub mod calendar;
pub mod google;
pub mod mail;
pub mod outgoing;
#[cfg(test)]
pub(crate) mod test_http;

use crate::model::*;
use async_trait::async_trait;
use secrecy::SecretString;

pub use shep_mail_core::providers::MailProvider;

#[async_trait]
pub trait CalendarProvider: Send + Sync {
    async fn events(
        &self,
        source: &CalendarSource,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> anyhow::Result<Vec<CalendarEvent>>;
    async fn save_event(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
    ) -> anyhow::Result<CalendarEvent>;
    async fn delete_event(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
    ) -> anyhow::Result<()>;
}

/// Legacy-workspace compatibility for explicit account diagnostics. Application
/// providers use the Engine's shared, profile-scoped credential worker instead.
pub async fn read_secret(id: &str) -> anyhow::Result<SecretString> {
    crate::credentials::Credentials::default().read(id).await
}
pub async fn write_secret(id: &str, secret: SecretString) -> anyhow::Result<()> {
    crate::credentials::Credentials::default()
        .write(id, secret)
        .await
}
pub(crate) mod drive_http;
