//! Object-scoped outgoing transports; test implementations never use real credentials.
use super::*;
use crate::outgoing::Submission;
use mail::{DeliveryFailure, sent::SentMailbox};

pub use mail::sent::SentConnection;
#[async_trait]
pub trait Outbound: Send + Sync {
    async fn submit(&self, message: &Submission) -> Result<(), DeliveryFailure>;
    async fn sent(&self, account: &Account) -> anyhow::Result<Box<dyn SentConnection>>;
}
#[derive(Default)]
pub struct Servers {
    pub credentials: crate::credentials::Credentials,
}
#[async_trait]
impl Outbound for Servers {
    async fn submit(&self, message: &Submission) -> Result<(), DeliveryFailure> {
        let account = &message.account;
        let secret = if account.smtp_auth == SmtpAuth::None {
            SecretString::from("")
        } else {
            self.credentials
                .read(&if account.smtp_separate_password {
                    format!("{}:smtp", account.id)
                } else {
                    account.id.clone()
                })
                .await
                .map_err(|_| {
                    DeliveryFailure::Rejected(
                        "Unlock your credential store or update the SMTP password.".into(),
                    )
                })?
        };
        let envelope = message.envelope.envelope().map_err(|_| {
            DeliveryFailure::Rejected("Check the saved recipient addresses.".into())
        })?;
        mail::send_raw(account, &secret, &envelope, &message.raw).await
    }
    async fn sent(&self, account: &Account) -> anyhow::Result<Box<dyn SentConnection>> {
        let secret = self.credentials.read(&account.id).await?;
        Ok(Box::new(SentMailbox::open(account, &secret).await?))
    }
}
