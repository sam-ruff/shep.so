use super::*;
use crate::{
    model::ConnectionTarget,
    profile_sync::vault::Tester,
    store::{
        Store,
        account_setup::{Attempt, Stage},
    },
};

impl Credentials {
    /// Own lifecycle/account guards during these writes, then release them
    /// before probing. Checked activation revalidates the captured attempt.
    pub async fn stage_account_setup(
        &self,
        store: &Store,
        id: String,
        incoming: SecretString,
        smtp: Option<SecretString>,
    ) -> anyhow::Result<Attempt> {
        let attempt = store.validate_account_setup(id.clone()).await?;
        anyhow::ensure!(
            attempt.stage == Stage::Admitted,
            "Re-enter the credentials for a new account setup request."
        );
        let result = async {
            anyhow::ensure!(
                attempt.slots.smtp.is_some() == smtp.is_some(),
                "Supply the required separate SMTP credential."
            );
            self.write(&attempt.slots.incoming, incoming.clone())
                .await?;
            if let (Some(slot), Some(secret)) = (&attempt.slots.smtp, &smtp) {
                self.write(slot, secret.clone()).await?;
            }
            let saved = self.read(&attempt.slots.incoming).await?;
            anyhow::ensure!(
                saved.expose_secret() == incoming.expose_secret(),
                "The credential store did not confirm the incoming credential."
            );
            if let (Some(slot), Some(secret)) = (&attempt.slots.smtp, &smtp) {
                anyhow::ensure!(
                    self.read(slot).await?.expose_secret() == secret.expose_secret(),
                    "The credential store did not confirm the SMTP credential."
                );
            }
            store
                .advance_account_setup(id.clone(), Stage::Admitted, Stage::Staged)
                .await
        }
        .await;
        if let Err(error) = &result {
            store.fail_account_setup(id, format!("{error:#}")).await?;
        }
        result
    }

    pub async fn prepare_account_setup(
        &self,
        store: &Store,
        id: String,
        incoming: SecretString,
        smtp: Option<SecretString>,
        tester: &dyn Tester,
    ) -> anyhow::Result<Attempt> {
        self.stage_account_setup(store, id.clone(), incoming, smtp)
            .await?;
        self.check_account_setup(store, id, tester).await
    }

    pub async fn check_account_setup(
        &self,
        store: &Store,
        id: String,
        tester: &dyn Tester,
    ) -> anyhow::Result<Attempt> {
        let attempt = store.validate_account_setup(id.clone()).await?;
        anyhow::ensure!(
            attempt.stage == Stage::Staged,
            "This account setup is not ready for connection checks."
        );
        let result = async {
            let incoming = self.read(&attempt.slots.incoming).await?;
            let smtp = match &attempt.slots.smtp {
                Some(key) => Some(self.read(key).await?),
                None => None,
            };
            tokio::time::timeout(
                std::time::Duration::from_secs(45),
                tester.test(&attempt.account, ConnectionTarget::Incoming, &incoming),
            )
            .await
            .context("The incoming connection check timed out")??;
            store.validate_account_setup(id.clone()).await?;
            let smtp = if attempt.account.smtp_auth == crate::model::SmtpAuth::None {
                SecretString::from("")
            } else {
                smtp.unwrap_or(incoming)
            };
            tokio::time::timeout(
                std::time::Duration::from_secs(45),
                tester.test(&attempt.account, ConnectionTarget::Smtp, &smtp),
            )
            .await
            .context("The SMTP connection check timed out")??;
            store
                .advance_account_setup(id.clone(), Stage::Staged, Stage::Checked)
                .await
        }
        .await;
        if let Err(error) = &result {
            store.fail_account_setup(id, format!("{error:#}")).await?;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::Account,
        profile_sync::vault::MockTester,
        store::{ConnectionKind, ConnectionRef},
    };
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    struct Secrets {
        values: Arc<Mutex<BTreeMap<String, String>>>,
        fail_smtp: bool,
    }
    impl Backend for Secrets {
        fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>> {
            Ok(self
                .values
                .lock()
                .expect("secrets")
                .get(key)
                .cloned()
                .map(Into::into))
        }
        fn write(&mut self, key: &str, value: SecretString) -> anyhow::Result<()> {
            anyhow::ensure!(
                !self.fail_smtp || !key.ends_with(":smtp"),
                "Keychain rejected second write"
            );
            self.values
                .lock()
                .expect("secrets")
                .insert(key.into(), value.expose_secret().into());
            Ok(())
        }
        fn delete(&mut self, key: &str) -> anyhow::Result<()> {
            self.values.lock().expect("secrets").remove(key);
            Ok(())
        }
    }
    fn account() -> Account {
        serde_json::from_value(serde_json::json!({"id":"fixture","name":"Fixture","email":"fixture@example.test","protocol":"Imap","host":"old.example.test","port":993,"username":"fixture","smtp_host":"smtp.example.test","smtp_port":465,"smtp_separate_password":true})).expect("account")
    }
    async fn setup(fail_smtp: bool) -> (tempfile::TempDir, Store, Credentials, Attempt) {
        let dir = tempfile::tempdir().expect("directory");
        let store = Store::open(dir.path().join("cache.sqlite")).expect("store");
        let old = account();
        store.save_account(old.clone()).await.expect("old account");
        let values = Arc::new(Mutex::new(BTreeMap::from([
            ("fixture".into(), "old-incoming".into()),
            ("fixture:smtp".into(), "old-smtp".into()),
        ])));
        let credentials = Credentials::with_backend(Scope::Legacy, Secrets { values, fail_smtp });
        let mut new = old.clone();
        new.host = "new.example.test".into();
        let attempt = store
            .admit_account_setup(uuid::Uuid::new_v4().to_string(), new, Some(old))
            .await
            .expect("admission");
        (dir, store, credentials, attempt)
    }
    fn probes() -> MockTester {
        let mut tester = MockTester::new();
        tester.expect_test().times(2).returning(|_, _, _| Ok(()));
        tester
    }

    #[tokio::test]
    async fn checked_smtp_binding_change_rejects_old_queued_delivery_even_without_smtp_auth() {
        for auth in [
            crate::model::SmtpAuth::None,
            crate::model::SmtpAuth::Automatic,
        ] {
            let (_dir, store, credentials, _) = setup(false).await;
            let mut original = account();
            original.smtp_auth = auth;
            store
                .save_account(original.clone())
                .await
                .expect("original binding");
            let draft = crate::model::Draft {
                id: "queued-fixture".into(),
                account_id: original.id.clone(),
                to: "recipient@example.test".into(),
                body: "Saved message".into(),
                revision: 1,
                ..Default::default()
            };
            store.save_draft(draft.clone()).await.expect("draft");
            let wire = crate::compose::build(&original, &draft, vec![]).expect("wire");
            let mut submission = crate::outgoing::Submission::new(original.clone(), &draft, wire)
                .expect("submission");
            submission.info.delivery = crate::outgoing::DeliveryState::Queued;
            let queued = store
                .begin_outgoing(submission, draft)
                .await
                .expect("queue");
            let mut changed = original.clone();
            changed.smtp_host = "other-smtp.example.test".into();
            let attempt = store
                .admit_account_setup(
                    uuid::Uuid::new_v4().to_string(),
                    changed,
                    Some(original.clone()),
                )
                .await
                .expect("new SMTP binding");
            credentials
                .prepare_account_setup(
                    &store,
                    attempt.id.clone(),
                    "new-incoming".into(),
                    (auth != crate::model::SmtpAuth::None).then(|| "new-smtp".into()),
                    &probes(),
                )
                .await
                .expect("checked");
            assert_eq!(
                attempt.slots.smtp.is_none(),
                auth == crate::model::SmtpAuth::None
            );
            store
                .activate_account_setup(attempt.id)
                .await
                .expect("activate");
            assert!(
                !store
                    .claim_outgoing(queued.attempt.clone())
                    .await
                    .expect("old binding refused")
            );
            assert_eq!(
                store
                    .outgoing_info(queued.attempt)
                    .await
                    .expect("review")
                    .delivery,
                crate::outgoing::DeliveryState::Rejected
            );
            let credentials = credentials.with_account_store(store.clone());
            assert!(credentials.account_password(&original, true).await.is_err());
            assert_eq!(
                credentials
                    .account_password(&original, false)
                    .await
                    .expect("unchanged incoming identity remains usable")
                    .expose_secret(),
                "new-incoming"
            );
        }
    }
    async fn prepare(
        store: &Store,
        credentials: &Credentials,
        attempt: &Attempt,
        tester: &dyn Tester,
    ) -> anyhow::Result<Attempt> {
        credentials
            .prepare_account_setup(
                store,
                attempt.id.clone(),
                "new-incoming".into(),
                Some("new-smtp".into()),
                tester,
            )
            .await
    }

    #[tokio::test]
    async fn checked_activation_is_atomic_and_lost_reply_keeps_the_active_pair() {
        let (_dir, store, credentials, attempt) = setup(false).await;
        assert_eq!(
            store
                .account_credential_key(account(), false)
                .await
                .expect("legacy"),
            "fixture"
        );
        prepare(&store, &credentials, &attempt, &probes())
            .await
            .expect("checks");
        assert_eq!(
            credentials
                .read("fixture")
                .await
                .expect("old")
                .expose_secret(),
            "old-incoming"
        );
        store
            .activate_account_setup(attempt.id.clone())
            .await
            .expect("activation");
        store
            .activate_account_setup(attempt.id.clone())
            .await
            .expect("lost reply retry");
        store
            .fail_account_setup(attempt.id.clone(), "Lost UI reply".into())
            .await
            .expect("late error");
        assert_eq!(
            store
                .account_setup(attempt.id)
                .await
                .expect("attempt")
                .stage,
            Stage::Activated
        );
        let key = store
            .account_credential_key(attempt.account.clone(), false)
            .await
            .expect("slot");
        assert_eq!(key, attempt.slots.incoming);
        assert_eq!(
            credentials.read(&key).await.expect("new").expose_secret(),
            "new-incoming"
        );
        assert!(store.credential_in_use(key).await.expect("active"));
        assert!(
            !store
                .credential_in_use("fixture".into())
                .await
                .expect("retired")
        );
        assert!(
            store
                .account_credential_key(account(), false)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn second_credential_write_failure_preserves_settings_and_both_active_secrets() {
        let (_dir, store, credentials, attempt) = setup(true).await;
        let tester = MockTester::new();
        assert!(
            prepare(&store, &credentials, &attempt, &tester)
                .await
                .expect_err("write failed")
                .to_string()
                .contains("second write")
        );
        assert_eq!(
            credentials
                .read("fixture")
                .await
                .expect("old incoming")
                .expose_secret(),
            "old-incoming"
        );
        assert_eq!(
            credentials
                .read("fixture:smtp")
                .await
                .expect("old smtp")
                .expose_secret(),
            "old-smtp"
        );
        assert_eq!(
            store
                .account_credential_key(account(), false)
                .await
                .expect("old settings"),
            "fixture"
        );
        assert_eq!(
            store
                .account_setup(attempt.id.clone())
                .await
                .expect("attempt")
                .stage,
            Stage::Failed
        );
        assert!(store.activate_account_setup(attempt.id).await.is_err());
        assert!(
            !store
                .credential_in_use(attempt.slots.incoming)
                .await
                .expect("cleanup eligible")
        );
    }

    #[tokio::test]
    async fn logical_backup_lookup_and_restore_preserve_the_verified_pair_and_reject_endpoint_reuse()
     {
        let (_dir, store, credentials, attempt) = setup(false).await;
        prepare(&store, &credentials, &attempt, &probes())
            .await
            .expect("checked");
        store
            .activate_account_setup(attempt.id.clone())
            .await
            .expect("activate");
        let credentials = credentials.with_account_store(store.clone());
        assert_eq!(
            credentials
                .read("fixture")
                .await
                .expect("logical export")
                .expose_secret(),
            "new-incoming"
        );
        assert_eq!(
            credentials
                .account_password(&attempt.account, true)
                .await
                .expect("SMTP")
                .expose_secret(),
            "new-smtp"
        );
        assert!(
            credentials
                .write("fixture", "unchecked".into())
                .await
                .is_err()
        );
        credentials
            .restore_missing("fixture", "old-backup".into())
            .await
            .expect("keep active");
        assert_eq!(
            credentials
                .read("fixture")
                .await
                .expect("unchanged")
                .expose_secret(),
            "new-incoming"
        );
        let mut renamed = attempt.account.clone();
        renamed.name = "Renamed".into();
        store
            .save_account(renamed.clone())
            .await
            .expect("name edit");
        assert!(
            credentials
                .account_password(&attempt.account, false)
                .await
                .is_ok()
        );
        credentials
            .delete(&attempt.slots.incoming)
            .await
            .expect("lost OS secret");
        assert!(
            credentials
                .restore_missing("fixture", "unverified-backup".into())
                .await
                .expect_err("checked reactivation required")
                .to_string()
                .contains("Reconnect")
        );
        renamed.host = "other.example.test".into();
        store
            .save_account(renamed.clone())
            .await
            .expect("changed endpoint");
        assert!(credentials.account_password(&renamed, true).await.is_err());
        assert!(credentials.read("fixture:smtp").await.is_err());
        credentials
            .write("google-oauth", "grant".into())
            .await
            .expect("unrelated namespace");
        assert_eq!(
            credentials
                .read("google-oauth")
                .await
                .expect("grant")
                .expose_secret(),
            "grant"
        );
    }

    #[tokio::test]
    async fn probe_refusal_keeps_old_activation_and_durable_failure() {
        let (_dir, store, credentials, attempt) = setup(false).await;
        let mut tester = MockTester::new();
        tester
            .expect_test()
            .times(1)
            .returning(|_, _, _| anyhow::bail!("Incoming rejected"));
        assert!(
            prepare(&store, &credentials, &attempt, &tester)
                .await
                .is_err()
        );
        let saved = store.account_setup(attempt.id).await.expect("saved");
        assert_eq!(saved.stage, Stage::Failed);
        assert_eq!(saved.error.as_deref(), Some("Incoming rejected"));
        assert_eq!(
            store
                .account_credential_key(account(), true)
                .await
                .expect("legacy"),
            "fixture:smtp"
        );
    }

    #[tokio::test]
    async fn newer_attempt_and_removal_fence_checked_old_activation() {
        let (_dir, store, credentials, attempt) = setup(false).await;
        prepare(&store, &credentials, &attempt, &probes())
            .await
            .expect("checks");
        let mut newer = attempt.account.clone();
        newer.host = "newer.example.test".into();
        let next = store
            .admit_account_setup(uuid::Uuid::new_v4().to_string(), newer, Some(account()))
            .await
            .expect("new admission");
        assert!(store.activate_account_setup(attempt.id).await.is_err());
        assert_eq!(
            store
                .account_setup(next.id.clone())
                .await
                .expect("newest")
                .stage,
            Stage::Admitted
        );
        let target = ConnectionRef {
            kind: ConnectionKind::Account,
            id: "fixture".into(),
        };
        let review = store.removal_preview(target).await.expect("review");
        store.remove_connection(review, true).await.expect("remove");
        assert!(store.activate_account_setup(next.id).await.is_err());
        assert!(
            !store
                .credential_in_use(next.slots.incoming)
                .await
                .expect("cleanup")
        );
    }

    #[tokio::test]
    async fn smtp_probe_refusal_never_activates_the_successful_incoming_probe() {
        let (_dir, store, credentials, attempt) = setup(false).await;
        let mut tester = MockTester::new();
        tester.expect_test().times(2).returning(|_, target, _| {
            anyhow::ensure!(target != ConnectionTarget::Smtp, "SMTP refused");
            Ok(())
        });
        assert!(
            prepare(&store, &credentials, &attempt, &tester)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .account_credential_key(account(), false)
                .await
                .expect("old settings"),
            "fixture"
        );
        assert!(store.activate_account_setup(attempt.id).await.is_err());
    }

    #[tokio::test]
    async fn restart_requires_explicit_credentials_and_never_promotes_checked_staging() {
        let (dir, store, credentials, attempt) = setup(false).await;
        prepare(&store, &credentials, &attempt, &probes())
            .await
            .expect("checks");
        drop(store);
        let reopened = Store::open(dir.path().join("cache.sqlite")).expect("reopen");
        reopened.interrupt_account_setups().await.expect("recovery");
        assert_eq!(
            reopened
                .account_setup(attempt.id.clone())
                .await
                .expect("saved")
                .stage,
            Stage::Interrupted
        );
        assert!(reopened.activate_account_setup(attempt.id).await.is_err());
        assert_eq!(
            reopened
                .account_credential_key(account(), false)
                .await
                .expect("old"),
            "fixture"
        );
    }
}
