//! Imported metadata is never authority to read a device's existing credentials.
use crate::{model::Account, store::Store};
use anyhow::{Result, ensure};
use secrecy::{ExposeSecret, SecretString};

#[async_trait::async_trait]
pub(crate) trait Secrets: Send + Sync {
    async fn write(&self, key: &str, value: SecretString) -> Result<()>;
}
pub(crate) struct OsSecrets;
#[async_trait::async_trait]
impl Secrets for OsSecrets {
    async fn write(&self, key: &str, value: SecretString) -> Result<()> {
        crate::providers::write_secret(key, value).await
    }
}
/// The caller holds the connection lifecycle and account locks. Keep the guard
/// through both keychain writes and the checked SQLite activation transaction.
pub(crate) async fn reconnect(
    store: &Store,
    account: Account,
    password: SecretString,
    smtp: SecretString,
    secrets: &dyn Secrets,
) -> Result<()> {
    let expected = store.profile_reconnect_review(account.id.clone()).await?;
    account.validate()?;
    ensure!(
        !password.expose_secret().is_empty(),
        "Enter this device's account password before reconnecting."
    );
    ensure!(
        !account.smtp_separate_password || !smtp.expose_secret().is_empty(),
        "Enter this device's separate SMTP password before reconnecting."
    );
    if account.smtp_separate_password {
        secrets.write(&format!("{}:smtp", account.id), smtp).await?;
    }
    secrets.write(&account.id, password).await?;
    store.activate_profile_account(expected, account).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;
    struct MemorySecrets {
        writes: Mutex<Vec<String>>,
        fail_incoming: bool,
    }
    #[async_trait::async_trait]
    impl Secrets for MemorySecrets {
        async fn write(&self, key: &str, _value: SecretString) -> Result<()> {
            self.writes.lock().unwrap().push(key.into());
            ensure!(
                !self.fail_incoming || key.ends_with(":smtp"),
                "synthetic locked incoming password"
            );
            Ok(())
        }
    }
    pub(crate) fn account() -> Account {
        serde_json::from_value(json!({"id":"guarded-account","name":"Imported","email":"shared@example.test","protocol":"Imap","host":"imap.example.test","port":993,"username":"shared","smtp_host":"smtp.example.test","smtp_port":465,"smtp_separate_password":true})).unwrap()
    }
    #[tokio::test]
    async fn reconnect_requires_both_passwords_and_keeps_guard_after_partial_keychain_failure() {
        let store = Store::memory().unwrap();
        let account = account();
        store.save_account(account.clone()).await.unwrap();
        store
            .run(|db| crate::store::profile_reconnect::mark(db, "guarded-account"))
            .await
            .unwrap();
        let failed = MemorySecrets {
            writes: Mutex::new(vec![]),
            fail_incoming: true,
        };
        assert!(
            reconnect(
                &store,
                account.clone(),
                SecretString::from(""),
                SecretString::from("smtp"),
                &failed
            )
            .await
            .is_err()
        );
        assert!(failed.writes.lock().unwrap().is_empty());
        assert!(
            reconnect(
                &store,
                account.clone(),
                SecretString::from("incoming"),
                SecretString::from("smtp"),
                &failed
            )
            .await
            .is_err()
        );
        assert_eq!(
            *failed.writes.lock().unwrap(),
            vec!["guarded-account:smtp", "guarded-account"]
        );
        assert!(
            store
                .require_profile_active(account.id.clone())
                .await
                .is_err()
        );
        let success = MemorySecrets {
            writes: Mutex::new(vec![]),
            fail_incoming: false,
        };
        assert!(
            reconnect(
                &store,
                account.clone(),
                SecretString::from("incoming"),
                SecretString::from(""),
                &success
            )
            .await
            .is_err()
        );
        assert!(success.writes.lock().unwrap().is_empty());
        reconnect(
            &store,
            account.clone(),
            SecretString::from("incoming"),
            SecretString::from("smtp"),
            &success,
        )
        .await
        .unwrap();
        assert_eq!(
            *success.writes.lock().unwrap(),
            vec!["guarded-account:smtp", "guarded-account"]
        );
        store
            .require_profile_active(account.id.clone())
            .await
            .unwrap();
        assert_eq!(
            store.get::<Vec<Account>>("accounts").await.unwrap(),
            vec![account]
        );
    }
    #[tokio::test]
    async fn activation_rejects_changed_or_removed_account_and_keeps_guard() {
        let store = Store::memory().unwrap();
        let account = account();
        store.save_account(account.clone()).await.unwrap();
        store
            .run(|db| crate::store::profile_reconnect::mark(db, "guarded-account"))
            .await
            .unwrap();
        let review = store
            .profile_reconnect_review(account.id.clone())
            .await
            .unwrap();
        let mut changed = account.clone();
        changed.host = "changed.example.test".into();
        store.save_account(changed.clone()).await.unwrap();
        assert!(
            store
                .activate_profile_account(review, account)
                .await
                .is_err()
        );
        let review = store
            .profile_reconnect_review(changed.id.clone())
            .await
            .unwrap();
        store.put("accounts", Vec::<Account>::new()).await.unwrap();
        assert!(
            store
                .activate_profile_account(review, changed.clone())
                .await
                .is_err()
        );
        assert!(store.profile_reconnect_required(changed.id).await.unwrap());
    }
}
