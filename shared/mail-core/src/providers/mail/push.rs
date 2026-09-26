//! A dedicated IMAP IDLE connection per account that reports Inbox changes as
//! they happen, so a check can start without waiting for the next poll.

use super::imap;
use crate::model::{Account, Protocol};
use async_imap::extensions::idle::IdleResponse;
use secrecy::SecretString;
use std::{future::Future, time::Duration};

/// IDLE is re-issued at least this often. RFC 2177 lets servers drop a
/// connection that has idled for 30 minutes.
pub const REISSUE: Duration = Duration::from_secs(25 * 60);

/// Progress a watcher reports while it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Push {
    /// IDLE is active; the server will report Inbox changes until the watcher ends.
    Connected,
    /// The server reported a change in INBOX.
    Changed,
}

/// Why a watcher returned without an error.
#[derive(Debug, PartialEq, Eq)]
pub enum WatchEnd {
    /// The stop signal fired and the connection was closed cleanly.
    Stopped,
    /// The server does not offer IDLE, so interval polling is the only option.
    Unsupported,
}

enum Outcome {
    Changed,
    Reissue,
    Stop,
}

/// Opens its own session for the account and watches INBOX until `stop`
/// resolves. Errors mean the connection was lost and may be retried.
pub async fn watch_inbox(
    account: &Account,
    password: &SecretString,
    on_push: impl FnMut(Push),
    stop: impl Future<Output = ()>,
) -> anyhow::Result<WatchEnd> {
    if account.protocol != Protocol::Imap {
        return Ok(WatchEnd::Unsupported);
    }
    let session = imap(account, password).await?;
    watch_session(session, on_push, stop).await
}

/// Public for scripted-session tests.
pub async fn watch_session<T>(
    mut session: async_imap::Session<T>,
    mut on_push: impl FnMut(Push),
    stop: impl Future<Output = ()>,
) -> anyhow::Result<WatchEnd>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let capabilities = session.capabilities().await?;
    if !capabilities.has_str("IDLE") {
        let _ = session.logout().await;
        return Ok(WatchEnd::Unsupported);
    }
    // Expunges then arrive as VANISHED, which IDLE reports as new data like
    // EXPUNGE; a refusal only keeps EXPUNGE.
    if capabilities.has_str("QRESYNC") && !super::qresync::enable(&mut session).await? {
        tracing::debug!("The mail server refused QRESYNC for IDLE");
    }
    session.select("INBOX").await?;
    let mut stop = std::pin::pin!(stop);
    let mut connected = false;
    loop {
        let mut handle = session.idle();
        handle.init().await?;
        if !connected {
            connected = true;
            on_push(Push::Connected);
        }
        let outcome = {
            // The library timer restarts on keepalives, so the outer timeout
            // bounds the whole IDLE regardless of server chatter.
            let (wait, _interrupt) = handle.wait_with_timeout(REISSUE);
            tokio::select! {
                biased;
                _ = &mut stop => Outcome::Stop,
                waited = tokio::time::timeout(REISSUE, wait) => match waited {
                    Ok(Ok(IdleResponse::NewData(_))) => Outcome::Changed,
                    Ok(Ok(IdleResponse::Timeout)) | Err(_) => Outcome::Reissue,
                    Ok(Ok(IdleResponse::ManualInterrupt)) => {
                        anyhow::bail!("The mail server closed the IDLE connection")
                    }
                    Ok(Err(error)) => return Err(error.into()),
                },
            }
        };
        session = handle.done().await?;
        match outcome {
            Outcome::Changed => on_push(Push::Changed),
            Outcome::Reissue => {}
            Outcome::Stop => {
                session.logout().await?;
                return Ok(WatchEnd::Stopped);
            }
        }
    }
}
