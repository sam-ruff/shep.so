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

pub async fn read_secret(id: &str) -> anyhow::Result<SecretString> {
    let id = id.to_string();
    tokio::task::spawn_blocking(move || {
        keyring::Entry::new("so.shep.desktop", &id)?
            .get_password()
            .map(SecretString::from)
            .map_err(anyhow::Error::from)
    })
    .await?
}
pub async fn write_secret(id: &str, secret: SecretString) -> anyhow::Result<()> {
    use secrecy::ExposeSecret;
    let id = id.to_string();
    tokio::task::spawn_blocking(move || {
        keyring::Entry::new("so.shep.desktop", &id)?.set_password(secret.expose_secret())?;
        Ok(())
    })
    .await?
}
