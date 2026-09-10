use super::*;
use crate::store::{ConnectionKind, ConnectionRef, RemovalPreview};

#[async_trait::async_trait]
pub(super) trait SecretRemover: Send + Sync {
    async fn remove(&self, key: &str) -> anyhow::Result<()>;
}
#[derive(Default)]
pub(super) struct OsSecretRemover(pub crate::credentials::Credentials);
#[async_trait::async_trait]
impl SecretRemover for OsSecretRemover {
    async fn remove(&self, key: &str) -> anyhow::Result<()> {
        self.0.delete(key).await
    }
}
impl Engine {
    pub(super) async fn connection_access(&self, target: &ConnectionRef) -> account_work::Access {
        match target.kind {
            ConnectionKind::Account => self.account_access(&target.id).await,
            ConnectionKind::Calendar => self.calendar_access(&target.id).await,
        }
    }
    async fn cleanup_owner(&self, target: &ConnectionRef) -> anyhow::Result<usize> {
        let mut failed = 0;
        for job in self
            .store
            .cleanup_jobs()
            .await?
            .into_iter()
            .filter(|job| job.target == *target)
        {
            let in_use = self.store.credential_in_use(job.key.clone()).await?;
            if in_use || self.demo || self.secret_remover.remove(&job.key).await.is_ok() {
                self.store.finish_credential_cleanup(job).await?;
            } else {
                failed += 1;
            }
        }
        Ok(failed)
    }
    pub(super) async fn remove_connection(
        &self,
        preview: RemovalPreview,
        cancel_transfers: bool,
    ) -> anyhow::Result<usize> {
        let _lifecycle = self.connection_lifecycle_lock.lock().await;
        let _owner = self.connection_access(&preview.target).await;
        self.store
            .remove_connection(preview.clone(), cancel_transfers)
            .await?;
        // Local removal is committed. A keychain failure remains a durable job,
        // never a reason to restore the removed account or pretend nothing changed.
        match self.cleanup_owner(&preview.target).await {
            Ok(failed) => Ok(failed),
            Err(_) => Ok(self
                .store
                .cleanup_jobs()
                .await
                .map(|jobs| jobs.iter().filter(|j| j.target == preview.target).count())
                .unwrap_or(1)
                .max(1)),
        }
    }
    pub(super) async fn cleanup_credentials(&self) -> anyhow::Result<usize> {
        let _lifecycle = self.connection_lifecycle_lock.lock().await;
        let mut owners = Vec::new();
        for job in self.store.cleanup_jobs().await? {
            if !owners.contains(&job.target) {
                owners.push(job.target);
            }
        }
        let mut failed = 0;
        for owner in owners {
            let _guard = self.connection_access(&owner).await;
            failed += self.cleanup_owner(&owner).await?;
        }
        Ok(failed)
    }
    pub(super) async fn restore_google_calendars(&self) -> anyhow::Result<usize> {
        let _google = self.google_connection_lock.read().await;
        anyhow::ensure!(
            !self
                .store
                .get::<Preferences>("preferences")
                .await?
                .google_lifecycle
                .disconnected,
            "Reconnect Google before restoring calendars."
        );
        let observed_revision = self.store.workspace().await?.connections_revision;
        let removed = self.store.removed_google_calendars().await?;
        let available = if self.demo {
            removed.clone()
        } else {
            let prefs: Preferences = self.store.get("preferences").await?;
            self.google.calendars(&prefs).await?
        };
        let selected: Vec<_> = available
            .into_iter()
            .filter(|s| removed.iter().any(|r| r.id == s.id))
            .collect();
        anyhow::ensure!(
            !selected.is_empty(),
            "No removed Google calendars are currently accessible. Check calendar access or reconnect Google."
        );
        let _lifecycle = self.connection_lifecycle_lock.lock().await;
        let mut ids: Vec<_> = selected.iter().map(|s| s.id.as_str()).collect();
        ids.sort();
        let mut guards = Vec::new();
        for id in ids {
            guards.push(self.calendar_access(id).await);
            self.store
                .check_calendar_reconnect(id.to_owned(), observed_revision)
                .await?;
        }
        let count = selected.len();
        self.store.save_sources(selected).await?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    #[derive(Default)]
    struct FakeRemover {
        fail: AtomicBool,
        calls: Mutex<Vec<String>>,
    }
    #[async_trait::async_trait]
    impl SecretRemover for FakeRemover {
        async fn remove(&self, key: &str) -> anyhow::Result<()> {
            self.calls.lock().unwrap().push(key.into());
            anyhow::ensure!(!self.fail.load(Ordering::SeqCst), "Fixture keychain locked");
            Ok(())
        }
    }
    fn source(id: &str) -> CalendarSource {
        CalendarSource {
            id: id.into(),
            name: "Home".into(),
            kind: CalendarKind::CalDav,
            url: "https://calendar.example.test/home/".into(),
            username: "alex".into(),
            access: Default::default(),
        }
    }
    fn target(id: &str) -> ConnectionRef {
        ConnectionRef {
            kind: ConnectionKind::Calendar,
            id: id.into(),
        }
    }

    #[tokio::test]
    async fn failed_keychain_cleanup_keeps_removal_committed_and_retries_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("connections.sqlite");
        let mut engine = super::super::calendar_tests::engine();
        engine.demo = false;
        engine.store = Store::open(&path).unwrap();
        let fake = Arc::new(FakeRemover::default());
        fake.fail.store(true, Ordering::SeqCst);
        engine.secret_remover = fake.clone();
        engine.store.save_source(source("home")).await.unwrap();
        engine
            .store
            .save_event(super::super::calendar_tests::event("home"))
            .await
            .unwrap();
        let review = engine.store.removal_preview(target("home")).await.unwrap();
        assert_eq!(engine.remove_connection(review, false).await.unwrap(), 1);
        assert!(engine.store.events().await.unwrap().is_empty());
        assert!(engine.store.workspace().await.unwrap().calendars.is_empty());
        drop(engine);
        let mut restarted = super::super::calendar_tests::engine();
        restarted.demo = false;
        restarted.store = Store::open(path).unwrap();
        restarted.secret_remover = fake.clone();
        assert_eq!(restarted.store.cleanup_jobs().await.unwrap().len(), 1);
        fake.fail.store(false, Ordering::SeqCst);
        assert_eq!(restarted.cleanup_credentials().await.unwrap(), 0);
        assert!(restarted.store.cleanup_jobs().await.unwrap().is_empty());
        assert_eq!(fake.calls.lock().unwrap().as_slice(), ["home", "home"]);
        assert!(
            restarted
                .store
                .check_connection(target("home"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn cleanup_does_not_delete_a_credential_still_claimed_by_another_connection() {
        let mut engine = super::super::calendar_tests::engine();
        engine.demo = false;
        let fake = Arc::new(FakeRemover::default());
        engine.secret_remover = fake.clone();
        let account:Account=serde_json::from_value(serde_json::json!({"id":"home","name":"Mail","email":"alex@example.test","protocol":"Imap","host":"imap.example.test","port":993,"username":"alex","smtp_host":"smtp.example.test","smtp_port":465})).unwrap();
        engine.store.save_account(account).await.unwrap();
        engine.store.save_source(source("home")).await.unwrap();
        let review = engine.store.removal_preview(target("home")).await.unwrap();
        assert_eq!(engine.remove_connection(review, false).await.unwrap(), 0);
        assert!(fake.calls.lock().unwrap().is_empty());
        assert_eq!(engine.store.workspace().await.unwrap().accounts.len(), 1);
        assert!(engine.store.cleanup_jobs().await.unwrap().is_empty());
    }

    struct HoldingRemover {
        started: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl SecretRemover for HoldingRemover {
        async fn remove(&self, _: &str) -> anyhow::Result<()> {
            self.started.notify_one();
            self.release.notified().await;
            Ok(())
        }
    }
    #[tokio::test]
    async fn reconnect_waits_for_credential_cleanup_and_stale_setup_cannot_revive_removal() {
        let mut engine = super::super::calendar_tests::engine();
        engine
            .connect_calendars(vec![source("unused")], "fixture".into(), 0)
            .await
            .unwrap();
        let saved = engine.store.workspace().await.unwrap();
        let id = saved.calendars[0].id.clone();
        let old_revision = saved.connections_revision;
        let review = engine.store.removal_preview(target(&id)).await.unwrap();
        engine.store.remove_connection(review, false).await.unwrap();
        assert!(
            engine
                .connect_calendars(vec![source("unused")], "fixture".into(), old_revision)
                .await
                .is_err()
        );
        let revision = engine.store.workspace().await.unwrap().connections_revision;
        let fake = Arc::new(HoldingRemover {
            started: Default::default(),
            release: Default::default(),
        });
        engine.secret_remover = fake.clone();
        engine.demo = false; // Only the injected fake credential remover is used.
        let cleanup_engine = engine.clone();
        let cleanup = tokio::spawn(async move { cleanup_engine.cleanup_credentials().await });
        fake.started.notified().await;
        assert!(engine.connection_lifecycle_lock.try_lock().is_err());
        engine.demo = true; // Calendar setup must not touch the real keychain.
        let reconnect =
            engine.connect_calendars(vec![source("unused")], "fixture".into(), revision);
        tokio::pin!(reconnect);
        assert!(futures::poll!(&mut reconnect).is_pending());
        fake.release.notify_one();
        cleanup.await.unwrap().unwrap();
        reconnect.await.unwrap();
        assert!(engine.store.cleanup_jobs().await.unwrap().is_empty());
        assert_eq!(engine.store.workspace().await.unwrap().calendars[0].id, id);
        engine.store.check_connection(target(&id)).await.unwrap();
    }
}
