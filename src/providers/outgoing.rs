//! Object-scoped outgoing transports; test implementations never use real credentials.
use super::*;
use crate::outgoing::Submission;
use mail::{
    DeliveryFailure,
    sent::{SentMailbox, SentReceipt},
};

#[async_trait]
pub trait SentConnection: Send {
    fn folder(&self) -> &str;
    async fn find(&mut self, id: &str) -> anyhow::Result<Option<SentReceipt>>;
    async fn append(&mut self, raw: &[u8], timestamp: i64) -> anyhow::Result<SentReceipt>;
}
#[async_trait]
impl SentConnection for SentMailbox {
    fn folder(&self) -> &str {
        &self.folder
    }
    async fn find(&mut self, id: &str) -> anyhow::Result<Option<SentReceipt>> {
        self.find(id).await
    }
    async fn append(&mut self, raw: &[u8], timestamp: i64) -> anyhow::Result<SentReceipt> {
        self.append(raw, timestamp).await
    }
}
#[async_trait]
pub trait Outbound: Send + Sync {
    async fn submit(&self, message: &Submission) -> Result<(), DeliveryFailure>;
    async fn sent(&self, account: &Account) -> anyhow::Result<Box<dyn SentConnection>>;
}
pub struct Servers;
#[async_trait]
impl Outbound for Servers {
    async fn submit(&self, message: &Submission) -> Result<(), DeliveryFailure> {
        let account = &message.account;
        let secret = if account.smtp_auth == SmtpAuth::None {
            SecretString::from("")
        } else {
            read_secret(&if account.smtp_separate_password {
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
        let secret = read_secret(&account.id).await?;
        Ok(Box::new(SentMailbox::open(account, &secret).await?))
    }
}
