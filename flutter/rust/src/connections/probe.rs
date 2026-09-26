use anyhow::{Context, Result};
use async_trait::async_trait;
use rusqlite::OptionalExtension;
use secrecy::SecretString;
use shep_mail_core::{model::Account, providers::mail};
use std::{future::Future, task::Poll};

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub(crate) trait ConnectionProbe: Send + Sync {
    async fn check(&self, account: Account, password: SecretString, smtp: bool) -> Result<()>;
}

#[cfg(test)]
mod tests;

pub(crate) struct ProviderProbe;

#[async_trait]
impl ConnectionProbe for ProviderProbe {
    async fn check(&self, account: Account, password: SecretString, smtp: bool) -> Result<()> {
        if smtp {
            mail::test_smtp(&account, &password).await?;
        } else {
            mail::test_incoming(&account, &password).await?;
        }
        Ok(())
    }
}

pub(crate) async fn run(
    profile: &crate::api::MobileProfile,
    api: &impl ConnectionProbe,
    attempt: String,
    password: SecretString,
    smtp: bool,
) -> Result<()> {
    let (_admission, _capacity) = profile.operations.try_connection_capacity()?;
    let slot = format!("credential-{attempt}");
    let lookup = slot.clone();
    let owner = profile
        .database
        .read(move |db| super::owner(db, &lookup))
        .await?;
    let _account = profile.operations.try_account(&owner).await?;
    let dispatch = profile.operations.connection_dispatch.lock().await;
    let account = profile.database.read(move |db| {
        super::validate(db, &attempt)?;
        crate::accounts::available(db, &owner)?;
        let (settings, expected): (String, String) = db.query_row(
            "SELECT settings,expected FROM credential_slots WHERE slot=?1 AND account_id=?2 AND state='prepared'",
            rusqlite::params![slot, owner], |r| Ok((r.get(0)?, r.get(1)?)),
        ).optional()?.context("This connection attempt was cancelled or replaced.")?;
        anyhow::ensure!(super::snapshot(db, &owner)? == expected,
            "This account changed. Reopen Preferences and reconnect.");
        let account: Account = serde_json::from_str(&settings)?;
        account.validate()?;
        Ok(account)
    }).await?;
    let mut request = Box::pin(tokio::time::timeout(
        std::time::Duration::from_secs(45),
        api.check(account, password, smtp),
    ));
    // Cancellation orders before validation or after the provider has started.
    let first = std::future::poll_fn(|cx| Poll::Ready(request.as_mut().poll(cx))).await;
    drop(dispatch);
    let result = match first {
        Poll::Ready(result) => result,
        Poll::Pending => request.await,
    };
    result
        .context("Connection timed out. Retry this saved connection.")?
        .map_err(|_| anyhow::anyhow!("Could not verify the connection. Check the hostname, TLS, username and password, then retry."))
}
