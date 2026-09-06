pub mod mail;

use crate::model::*;
use async_trait::async_trait;
use secrecy::SecretString;

#[async_trait]
pub trait MailProvider: Send + Sync {
    async fn sync(
        &self,
        account: &Account,
        password: &SecretString,
        known: &std::collections::HashSet<String>,
        output: tokio::sync::mpsc::Sender<MailSyncItem>,
    ) -> anyhow::Result<Vec<String>>;
    async fn move_mail(
        &self,
        account: &Account,
        password: &SecretString,
        mail: &Mail,
        folder: &str,
    ) -> anyhow::Result<Option<String>>;
    async fn set_flags(
        &self,
        account: &Account,
        password: &SecretString,
        mail: &Mail,
        changes: crate::mail_actions::Flags,
    ) -> anyhow::Result<()>;
}
