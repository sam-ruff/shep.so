pub mod calendar;
pub mod google;
pub mod mail;
pub mod outgoing;
#[cfg(test)]
pub(crate) mod test_http;

use crate::model::*;
use async_trait::async_trait;
use secrecy::SecretString;

#[async_trait]
pub trait MailProvider: Send + Sync {
    async fn sync(
        &self,
        account: &Account,
        password: &SecretString,
        known: &std::collections::HashSet<String>,
        output: tokio::sync::mpsc::Sender<MailSyncItem>,
    ) -> anyhow::Result<Vec<String>>;
    async fn move_mail(
        &self,
        account: &Account,
        password: &SecretString,
        mail: &Mail,
        folder: &str,
    ) -> anyhow::Result<Option<String>>;
    async fn set_flags(
        &self,
        account: &Account,
        password: &SecretString,
        mail: &Mail,
        changes: crate::mail_actions::Flags,
    ) -> anyhow::Result<()>;
}

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
