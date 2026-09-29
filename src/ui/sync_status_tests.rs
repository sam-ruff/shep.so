//! Mail-check results through a real Store and App. Each IMAP check saves its
//! folder listing, which republishes the workspace before the result arrives;
//! these regressions keep that from making the check's own result stale.
use super::*;
use crate::engine::{SyncAttempt, SyncOrigin};
use crate::store::Store;
use std::time::Duration;

fn account(id: &str, name: &str, host: &str) -> Account {
    serde_json::from_value(serde_json::json!({
        "id": id, "name": name, "email": format!("{id}@example.test"),
        "protocol": "Imap", "host": host, "port": 993,
        "username": id, "smtp_host": "smtp.example.test", "smtp_port": 465
    }))
    .expect("fixture account")
}

struct Fixture {
    store: Store,
    app: App,
    sequence: u64,
}

impl Fixture {
    async fn new(accounts: &[Account]) -> anyhow::Result<Self> {
        let store = Store::memory()?;
        for account in accounts {
            store.save_account(account.clone()).await?;
        }
        let (app, _) = App::new();
        let mut fixture = Self {
            store,
            app,
            sequence: 0,
        };
        fixture.publish().await?;
        Ok(fixture)
    }

    async fn publish(&mut self) -> anyhow::Result<()> {
        let workspace = self.store.workspace().await?;
        let _ = self
            .app
            .handle(Message::Backend(Event::Workspace(Arc::new(workspace))));
        Ok(())
    }

    /// Lists the account as the scheduler does and starts one attempt.
    async fn start(&mut self, id: &str, origin: SyncOrigin) -> anyhow::Result<SyncAttempt> {
        let (_, connection) = self
            .store
            .accounts_ready_to_watch()
            .await?
            .into_iter()
            .find(|(account, _)| account.id == id)
            .expect("listed account");
        self.sequence += 1;
        let attempt = SyncAttempt {
            account: id.into(),
            connection,
            sequence: self.sequence,
            origin,
        };
        let _ = self
            .app
            .handle(Message::Backend(Event::MailSyncStarted(attempt.clone())));
        Ok(attempt)
    }

    /// The check's own folder LIST advances the connections revision and
    /// republishes the workspace.
    async fn list_folders(&mut self, id: &str) -> anyhow::Result<()> {
        let before = self.store.workspace().await?.connections_revision;
        self.store
            .save_folders(id.into(), vec!["INBOX".into(), "Archive".into()])
            .await?;
        assert!(self.store.workspace().await?.connections_revision > before);
        self.publish().await
    }

    fn finish(&mut self, attempt: SyncAttempt, result: Result<(), &str>) {
        let _ = self.app.handle(Message::Backend(Event::MailSyncFinished(
            attempt,
            result.map_err(str::to_owned),
        )));
    }

    /// Stands in for the grace period elapsing after the first failure.
    fn age(&mut self, attempt: &SyncAttempt) {
        self.app
            .sync_status
            .backdate(&attempt.account, Duration::from_secs(31));
    }

    fn notice(&self) -> Option<&str> {
        self.app.notice.as_ref().map(|notice| notice.0.as_str())
    }
}

const TIMEOUT: &str = "Cognition sync failed: Account sync timed out without download progress";

#[tokio::test]
async fn refresh_error_after_the_folder_listing_is_shown() -> anyhow::Result<()> {
    let mut f = Fixture::new(&[account("cognition", "Cognition", "imap.example.test")]).await?;
    let manual = f.start("cognition", SyncOrigin::Refresh).await?;
    f.list_folders("cognition").await?;
    f.finish(manual, Err(TIMEOUT));
    assert_eq!(f.notice(), Some(TIMEOUT));
    assert!(!f.app.sync_status.has_failures(), "Refresh starts no timer");
    Ok(())
}

#[tokio::test]
async fn background_failure_after_the_folder_listing_reaches_the_banner() -> anyhow::Result<()> {
    let mut f = Fixture::new(&[account("cognition", "Cognition", "imap.example.test")]).await?;
    let background = f.start("cognition", SyncOrigin::Background).await?;
    f.list_folders("cognition").await?;
    f.finish(background.clone(), Err(TIMEOUT));
    assert_eq!(f.app.sync_status.failure_count(), 1);
    let _ = f.app.handle(Message::Tick);
    assert_eq!(f.notice(), None, "no banner inside the grace period");
    f.age(&background);
    let _ = f.app.handle(Message::Tick);
    assert_eq!(
        f.notice(),
        Some(
            format!("Cognition: mail checks are still failing. Try Refresh. Last error: {TIMEOUT}")
                .as_str()
        )
    );
    Ok(())
}

#[tokio::test]
async fn recovery_after_the_folder_listing_closes_the_episode_without_a_banner()
-> anyhow::Result<()> {
    let mut f = Fixture::new(&[account("cognition", "Cognition", "imap.example.test")]).await?;
    // Refused before LOGIN, so no listing; aged past the grace period.
    let failed = f.start("cognition", SyncOrigin::Background).await?;
    f.finish(failed.clone(), Err("Connection refused"));
    f.age(&failed);
    // The account recovers before any Tick: this check lists, then succeeds.
    let ok = f.start("cognition", SyncOrigin::Background).await?;
    f.list_folders("cognition").await?;
    f.finish(ok, Ok(()));
    assert!(!f.app.sync_status.has_failures());
    let _next = f.start("cognition", SyncOrigin::Background).await?;
    let _ = f.app.handle(Message::Tick);
    assert_eq!(f.notice(), None, "a recovered account raised the banner");
    Ok(())
}

#[tokio::test]
async fn refresh_error_clears_when_the_account_recovers_after_listing() -> anyhow::Result<()> {
    let mut f = Fixture::new(&[account("cognition", "Cognition", "imap.example.test")]).await?;
    let manual = f.start("cognition", SyncOrigin::Refresh).await?;
    f.finish(manual, Err("Connection refused"));
    assert_eq!(f.notice(), Some("Connection refused"));
    let ok = f.start("cognition", SyncOrigin::Background).await?;
    f.list_folders("cognition").await?;
    f.finish(ok, Ok(()));
    assert_eq!(f.notice(), None);
    Ok(())
}

#[tokio::test]
async fn a_healthy_accounts_listing_does_not_hide_another_accounts_failure() -> anyhow::Result<()> {
    let mut f = Fixture::new(&[
        account("cognition", "Cognition", "imap.example.test"),
        account("healthy", "Healthy", "imap.healthy.example.test"),
    ])
    .await?;
    let failing = f.start("cognition", SyncOrigin::Background).await?;
    let healthy = f.start("healthy", SyncOrigin::Background).await?;
    f.list_folders("healthy").await?;
    f.finish(healthy, Ok(()));
    f.list_folders("cognition").await?;
    f.finish(failing.clone(), Err(TIMEOUT));
    assert_eq!(f.app.sync_status.failure_count(), 1);
    f.age(&failing);
    let _ = f.app.handle(Message::Tick);
    assert!(
        f.notice()
            .is_some_and(|notice| notice.starts_with("Cognition: mail checks are still failing"))
    );
    Ok(())
}

#[tokio::test]
async fn rename_keeps_the_episode_while_reconfiguration_retires_it() -> anyhow::Result<()> {
    let mut f = Fixture::new(&[account("cognition", "Cognition", "imap.example.test")]).await?;
    let failed = f.start("cognition", SyncOrigin::Background).await?;
    f.finish(failed.clone(), Err(TIMEOUT));
    f.store
        .save_account(account("cognition", "Work", "imap.example.test"))
        .await?;
    f.publish().await?;
    assert_eq!(f.app.sync_status.failure_count(), 1, "a rename keeps it");
    let late = f.start("cognition", SyncOrigin::Background).await?;
    f.store
        .save_account(account("cognition", "Work", "imap.moved.example.test"))
        .await?;
    f.publish().await?;
    assert!(!f.app.sync_status.has_failures(), "a new server ends it");
    // A late result from the previous server cannot start another episode.
    f.finish(late, Err(TIMEOUT));
    assert!(!f.app.sync_status.has_failures());
    f.finish(failed, Err(TIMEOUT));
    assert!(!f.app.sync_status.has_failures());
    let _ = f.app.handle(Message::Tick);
    assert_eq!(f.notice(), None);
    Ok(())
}
