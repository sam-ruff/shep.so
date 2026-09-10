use super::Snapshot;
use crate::model::*;
use anyhow::Context;
use async_trait::async_trait;
use secrecy::SecretString;
use std::collections::HashSet;

/// Account and CalDAV IDs share the OS keychain namespace. Google OAuth and
/// destination passphrases are device credentials, never snapshot credentials.
fn credential_owner(id: &str) -> anyhow::Result<()> {
    crate::credentials::connection_id(id, false)
}

fn identifier(value: &str, label: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !value.is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control),
        "The backup contains an invalid {label}."
    );
    Ok(())
}

impl Snapshot {
    /// Validate the complete archive before any local configuration or secret
    /// can be changed. Orphaned cached messages and locally moved IDs are valid.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.version == 1, "This backup version is not supported.");
        anyhow::ensure!(
            chrono::DateTime::from_timestamp(self.created_at, 0).is_some(),
            "The backup has an invalid creation date."
        );
        self.preferences.validate()?;
        crate::credentials::validate_connections(&self.accounts, &self.calendars)?;
        let mut owners = HashSet::new();
        let mut allowed = HashSet::new();
        for account in &self.accounts {
            credential_owner(&account.id)?;
            anyhow::ensure!(
                owners.insert(&account.id),
                "The backup contains duplicate account or calendar identifiers."
            );
            account.validate()?;
            anyhow::ensure!(
                !account.smtp_host.trim().is_empty()
                    && !account.smtp_host.contains(['/', '\r', '\n', ' '])
                    && !account.smtp_username.contains(['\r', '\n', '\0']),
                "The backup contains invalid SMTP connection settings."
            );
            allowed.insert(account.id.clone());
            if account.smtp_separate_password {
                allowed.insert(format!("{}:smtp", account.id));
            }
        }
        for source in &self.calendars {
            identifier(&source.name, "calendar name")?;
            anyhow::ensure!(
                owners.insert(&source.id),
                "The backup contains duplicate account or calendar identifiers."
            );
            match source.kind {
                CalendarKind::CalDav => {
                    crate::credentials::connection_id(&source.id, true)?;
                    identifier(&source.username, "CalDAV username")?;
                    let url = crate::providers::calendar::validate_caldav_url(&source.url)?;
                    anyhow::ensure!(
                        url.host().is_some() && url.fragment().is_none(),
                        "The backup contains an invalid CalDAV URL."
                    );
                    allowed.insert(source.id.clone());
                }
                CalendarKind::Google => {
                    identifier(&source.url, "Google calendar identifier")?;
                    anyhow::ensure!(
                        source.id == format!("google:{}", source.url),
                        "The backup contains an invalid Google calendar source."
                    );
                }
            }
        }
        let mut credentials = HashSet::new();
        for (id, secret) in &self.credentials {
            anyhow::ensure!(
                allowed.contains(id),
                "The backup contains a password without a matching account or CalDAV source."
            );
            anyhow::ensure!(
                credentials.insert(id),
                "The backup contains duplicate passwords."
            );
            anyhow::ensure!(
                !secret.is_empty() && secret.len() <= 64 * 1024,
                "The backup contains an invalid account password."
            );
        }
        let mut messages = HashSet::new();
        let mut bytes = 0usize;
        for message in &self.messages {
            let mail = &message.summary;
            identifier(&mail.id, "message identifier")?;
            credential_owner(&mail.account_id)?;
            identifier(&mail.remote_id, "remote message identifier")?;
            identifier(&mail.folder, "message folder")?;
            anyhow::ensure!(
                messages.insert(&mail.id),
                "The backup contains duplicate message identifiers."
            );
            anyhow::ensure!(
                chrono::DateTime::from_timestamp(mail.timestamp, 0).is_some(),
                "The backup contains an invalid message date."
            );
            anyhow::ensure!(
                !message.raw.is_empty() && message.raw.len() <= MAX_MESSAGE_BYTES,
                "The backup contains an empty or oversized original email."
            );
            bytes = bytes
                .checked_add(message.raw.len())
                .context("Backup size overflow")?;
            anyhow::ensure!(
                bytes <= 256 * 1024 * 1024,
                "The backup exceeds the 256 MiB mail restore limit."
            );
        }
        Ok(())
    }

    /// Rebuild display/search data from the original MIME off the store worker.
    /// Preserve stable local IDs, folder moves, flags and recorded receipt dates.
    pub(crate) fn prepare_restore(mut self) -> anyhow::Result<Self> {
        self.validate()?;
        for message in &mut self.messages {
            let summary = &message.summary;
            let mut parsed = parse_mail(
                &summary.account_id,
                &summary.remote_id,
                &summary.folder,
                std::mem::take(&mut message.raw),
                summary.unread,
                summary.starred,
            )
            .context("The backup contains an unreadable original email")?;
            parsed.summary.id.clone_from(&summary.id);
            parsed.summary.timestamp = summary.timestamp;
            *message = parsed;
        }
        Ok(self)
    }
}

#[async_trait]
pub(crate) trait CredentialRestorer: Send + Sync {
    /// Preserve existing passwords. A locked/unavailable keychain is an error,
    /// not evidence that a password is missing. Retrying fills only missing keys.
    async fn restore_missing(&self, id: &str, secret: SecretString) -> anyhow::Result<()>;
}

#[derive(Default)]
pub(crate) struct OsCredentialRestorer(pub crate::credentials::Credentials);
#[async_trait]
impl CredentialRestorer for OsCredentialRestorer {
    async fn restore_missing(&self, id: &str, secret: SecretString) -> anyhow::Result<()> {
        self.0.restore_missing(id, secret).await
    }
}
