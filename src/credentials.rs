//! A profile owns its credential namespace and one bounded FIFO worker. Imported
//! databases never select that namespace: local profile creation supplies it.
use anyhow::Context;
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

const CAPACITY: usize = 32;

/// Validate IDs from portable data before they can address this profile's
/// credential store. Display names and email addresses are not restricted.
pub(crate) fn connection_id(id: &str, caldav: bool) -> anyhow::Result<()> {
    let ordinary = !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'));
    let discovered = caldav
        && id
            .strip_prefix("caldav:")
            .is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()));
    anyhow::ensure!(
        (ordinary || discovered)
            && !id.eq_ignore_ascii_case("google-oauth")
            && !id.eq_ignore_ascii_case("backup-passphrase"),
        "An account or CalDAV source has an invalid or reserved credential identifier. Export a fresh copy from the original device."
    );
    Ok(())
}

pub(crate) fn validate_connections(
    accounts: &[crate::model::Account],
    calendars: &[crate::model::CalendarSource],
) -> anyhow::Result<()> {
    let mut keys = std::collections::HashSet::new();
    for account in accounts {
        connection_id(&account.id, false)?;
        anyhow::ensure!(
            keys.insert(account.id.to_ascii_lowercase())
                && keys.insert(format!("{}:smtp", account.id.to_ascii_lowercase())),
            "Account credential identifiers overlap. Export a fresh copy after correcting the original accounts."
        );
    }
    for source in calendars {
        if source.kind == crate::model::CalendarKind::CalDav {
            connection_id(&source.id, true)?;
            anyhow::ensure!(
                keys.insert(source.id.to_ascii_lowercase()),
                "Account and calendar credential identifiers overlap. Export a fresh copy after correcting the original connections."
            );
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    Legacy,
    Profile(uuid::Uuid),
}
impl Scope {
    fn key(&self, id: &str) -> String {
        match self {
            Self::Legacy => id.to_owned(),
            Self::Profile(profile) => format!("profile:{profile}:{id}"),
        }
    }
}

enum Operation {
    Read,
    Write(SecretString),
    RestoreMissing(SecretString),
    Delete,
}
struct Request {
    key: String,
    operation: Operation,
    reply: oneshot::Sender<anyhow::Result<Option<SecretString>>>,
}

#[derive(Clone)]
pub struct Credentials {
    scope: Scope,
    commands: mpsc::Sender<Request>,
    start_error: Option<Arc<String>>,
}
impl Default for Credentials {
    fn default() -> Self {
        Self::new(Scope::Legacy)
    }
}
impl Credentials {
    pub fn new(scope: Scope) -> Self {
        Self::with_backend(scope, OsBackend)
    }

    pub(crate) fn with_backend(scope: Scope, mut backend: impl Backend) -> Self {
        let (commands, mut input) = mpsc::channel::<Request>(CAPACITY);
        let start_error = std::thread::Builder::new()
            .name("shep-credentials".into())
            .spawn(move || {
                while let Some(request) = input.blocking_recv() {
                    // Accepted writes run even when their observer closes. Later
                    // reads/writes cannot pass a still-running OS credential call.
                    let result = execute(&mut backend, &request.key, request.operation);
                    let _ = request.reply.send(result);
                }
            })
            .err()
            .map(|error| {
                Arc::new(format!(
                    "Could not start the credential service: {error}. Reopen Shep."
                ))
            });
        Self {
            scope,
            commands,
            start_error,
        }
    }

    async fn call(&self, id: &str, operation: Operation) -> anyhow::Result<Option<SecretString>> {
        if let Some(error) = &self.start_error {
            anyhow::bail!("{error}");
        }
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Request {
                key: self.scope.key(id),
                operation,
                reply,
            })
            .await
            .map_err(|_| {
                anyhow::anyhow!("The credential service stopped. Reopen Shep and retry.")
            })?;
        result.await.context("The credential service stopped before confirming the operation. Reopen Shep and check the account.")?
    }

    pub async fn read_optional(&self, id: &str) -> anyhow::Result<Option<SecretString>> {
        self.call(id, Operation::Read).await
    }
    pub async fn read(&self, id: &str) -> anyhow::Result<SecretString> {
        self.read_optional(id).await?.context(
            "Credentials are missing from this profile. Reconnect or enter them in Preferences.",
        )
    }
    pub async fn write(&self, id: &str, secret: SecretString) -> anyhow::Result<()> {
        self.call(id, Operation::Write(secret)).await?;
        Ok(())
    }
    pub async fn restore_missing(&self, id: &str, secret: SecretString) -> anyhow::Result<()> {
        self.call(id, Operation::RestoreMissing(secret)).await?;
        Ok(())
    }
    pub async fn delete(&self, id: &str) -> anyhow::Result<()> {
        self.call(id, Operation::Delete).await?;
        Ok(())
    }
}

pub(crate) trait Backend: Send + 'static {
    fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>>;
    fn write(&mut self, key: &str, value: SecretString) -> anyhow::Result<()>;
    fn delete(&mut self, key: &str) -> anyhow::Result<()>;
}
struct OsBackend;
impl Backend for OsBackend {
    fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>> {
        match keyring::Entry::new("so.shep.desktop", key)?.get_password() {
            Ok(value) => Ok(Some(SecretString::from(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    fn write(&mut self, key: &str, value: SecretString) -> anyhow::Result<()> {
        keyring::Entry::new("so.shep.desktop", key)?.set_password(value.expose_secret())?;
        Ok(())
    }
    fn delete(&mut self, key: &str) -> anyhow::Result<()> {
        match keyring::Entry::new("so.shep.desktop", key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

fn execute(
    backend: &mut impl Backend,
    key: &str,
    operation: Operation,
) -> anyhow::Result<Option<SecretString>> {
    match operation {
        Operation::Read => return backend.read(key),
        Operation::Write(value) => backend.write(key, value)?,
        Operation::RestoreMissing(value) => {
            if backend.read(key)?.is_none() {
                backend.write(key, value)?;
            }
        }
        Operation::Delete => backend.delete(key)?,
    }
    Ok(None)
}

#[cfg(test)]
mod tests;
