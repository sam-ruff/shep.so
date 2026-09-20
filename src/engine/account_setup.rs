//! Native connection setup resolves secrets on background owners. A shared
//! definition requiring reconnection must receive fresh explicit credentials.
use super::*;

impl Engine {
    pub(super) fn account_setup_allowed(&self) -> bool {
        #[cfg(feature = "test-support")]
        if self.demo && crate::test_support::passwords::active() {
            return true;
        }
        !self.demo
    }
    pub(super) async fn connect_admitted_account(
        &self,
        id: String,
        password: SecretString,
        smtp: SecretString,
        mut output: Output,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.account_setup_allowed(),
            "Account changes are disabled in preview."
        );
        let attempt = self.store.validate_account_setup(id.clone()).await?;
        let lifecycle = self.connection_lifecycle.write().await;
        let guard = self.account_exclusive(&attempt.account.id).await;
        let writes = self.account_setup_writes.read().await;
        if self.bulk_control.stopping.get() {
            self.store.interrupt_account_setup(id).await?;
            return Ok(());
        }
        let attempt = self.store.validate_account_setup(id.clone()).await?;
        self.store
            .ensure_folder_idle(attempt.account.id.clone())
            .await?;
        self.store
            .check_account_mailbox_identity(attempt.account.clone())
            .await?;
        let incoming = self
            .setup_password(
                &attempt.account,
                &password,
                &smtp,
                ConnectionTarget::Incoming,
            )
            .await?;
        let separate = if attempt.slots.smtp.is_some() {
            Some(
                self.setup_password(&attempt.account, &incoming, &smtp, ConnectionTarget::Smtp)
                    .await?,
            )
        } else {
            None
        };
        let staged = self
            .credentials
            .stage_account_setup(&self.store, id.clone(), incoming, separate)
            .await?;
        drop(guard);
        drop(lifecycle);
        drop(writes);
        output.send(Event::AccountSetupChanged(staged)).await?;
        tokio::select! {
            biased;
            _ = self.bulk_control.stopping.requested() => {
                self.store.interrupt_account_setup(id).await?;
                return Ok(());
            }
            checked = self.credentials.check_account_setup(&self.store, id.clone(), self.account_tester.as_ref()) => { checked?; }
        }
        let _lifecycle = self.connection_lifecycle.write().await;
        let _guard = self.account_exclusive(&attempt.account.id).await;
        let _writes = self.account_setup_writes.read().await;
        if self.bulk_control.stopping.get() {
            self.store.interrupt_account_setup(id).await?;
            return Ok(());
        }
        self.store.activate_account_setup(id).await?;
        self.workspace(&mut output).await?;
        Ok(())
    }

    pub(super) async fn setup_password(
        &self,
        account: &Account,
        incoming: &SecretString,
        smtp: &SecretString,
        target: ConnectionTarget,
    ) -> anyhow::Result<SecretString> {
        if target == ConnectionTarget::Smtp && account.smtp_auth == SmtpAuth::None {
            return Ok(SecretString::from(""));
        }
        let separate = target == ConnectionTarget::Smtp && account.smtp_separate_password;
        let supplied = if separate { smtp } else { incoming };
        if !supplied.expose_secret().is_empty() {
            return Ok(supplied.clone());
        }
        self.store
            .require_account_reconnected(account.id.clone())
            .await?;
        let key = if separate {
            format!("{}:smtp", account.id)
        } else {
            account.id.clone()
        };
        self.credentials.read(&key).await.context(if separate {
            "Enter the separate SMTP password"
        } else {
            "Enter an account password or app password"
        })
    }
}

pub(super) fn tester(demo: bool) -> Arc<dyn crate::profile_sync::vault::Tester> {
    #[cfg(feature = "test-support")]
    if demo && crate::test_support::passwords::active() {
        return Arc::new(crate::test_support::passwords::Tester);
    }
    let _ = demo;
    Arc::new(crate::profile_sync::vault::MailTester)
}

#[cfg(test)]
mod tests;
