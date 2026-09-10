//! Native connection setup resolves secrets on background owners. A shared
//! definition requiring reconnection must receive fresh explicit credentials.
use super::*;

impl Engine {
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

#[cfg(test)]
mod tests;
