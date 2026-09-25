pub mod mail;

use crate::model::*;
use async_trait::async_trait;
use secrecy::SecretString;

#[async_trait]
pub trait MailProvider: Send + Sync {
    #[cfg(feature = "staged-receive")]
    async fn sync_staged(
        &self,
        account: &Account,
        password: &SecretString,
        known: &std::collections::HashSet<String>,
        output: tokio::sync::mpsc::Sender<MailSyncItem>,
        plaintext_staging: bool,
    ) -> anyhow::Result<Vec<String>> {
        let _ = plaintext_staging;
        self.sync(account, password, known, output).await
    }
    async fn sync(
        &self,
        account: &Account,
        password: &SecretString,
        known: &std::collections::HashSet<String>,
        output: tokio::sync::mpsc::Sender<MailSyncItem>,
    ) -> anyhow::Result<Vec<String>>;
    /// Like `sync_staged`, refreshing cached flags from the saved CONDSTORE
    /// folder states where the provider supports it.
    #[cfg(feature = "condstore")]
    async fn sync_resuming(
        &self,
        account: &Account,
        password: &SecretString,
        known: &std::collections::HashSet<String>,
        resume: &mail::condstore::Resume,
        output: tokio::sync::mpsc::Sender<MailSyncItem>,
        plaintext_staging: bool,
    ) -> anyhow::Result<Vec<String>> {
        let _ = resume;
        self.sync_staged(account, password, known, output, plaintext_staging)
            .await
    }
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
    async fn inspect_flags(
        &self,
        _account: &Account,
        _password: &SecretString,
        _mail: &Mail,
    ) -> anyhow::Result<crate::mail_actions::Flags> {
        anyhow::bail!("This provider cannot inspect one message's flags.")
    }
    async fn inspect_move(
        &self,
        _account: &Account,
        _password: &SecretString,
        _receipt: &crate::mail_actions::MoveReceipt,
    ) -> anyhow::Result<Mail> {
        anyhow::bail!("This provider cannot inspect a moved message.")
    }
}
