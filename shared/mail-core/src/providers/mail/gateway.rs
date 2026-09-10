//! Explicit TCP destinations for a hosted gateway. TLS continues to validate the
//! account's hostname. Addresses come from administrator policy, never web input.
use super::*;
use std::net::SocketAddr;

#[derive(Default)]
pub(super) struct Route {
    pub peer: Option<SocketAddr>,
    #[cfg(test)]
    pub test_ca: Option<native_tls::Certificate>,
}
impl Route {
    pub async fn connect(&self, host: &str, port: u16) -> std::io::Result<TcpStream> {
        match self.peer {
            Some(peer) => TcpStream::connect(peer).await,
            None => TcpStream::connect((host, port)).await,
        }
    }
    pub fn connector(&self) -> anyhow::Result<tokio_native_tls::TlsConnector> {
        #[allow(unused_mut)]
        let mut builder = native_tls::TlsConnector::builder();
        #[cfg(test)]
        if let Some(ca) = &self.test_ca {
            builder.add_root_certificate(ca.clone());
        }
        Ok(tokio_native_tls::TlsConnector::from(builder.build()?))
    }
}

pub struct PinnedMail {
    incoming: Route,
    smtp: SocketAddr,
    #[cfg(test)]
    smtp_test_ca: Option<lettre::transport::smtp::client::Certificate>,
}
impl PinnedMail {
    /// Callers must authorize both exact destinations before constructing this.
    pub fn new(incoming: SocketAddr, smtp: SocketAddr) -> Self {
        Self {
            incoming: Route {
                peer: Some(incoming),
                #[cfg(test)]
                test_ca: None,
            },
            smtp,
            #[cfg(test)]
            smtp_test_ca: None,
        }
    }
    pub async fn probe_incoming(
        &self,
        account: &Account,
        password: &SecretString,
    ) -> anyhow::Result<()> {
        tokio::time::timeout(Duration::from_secs(35), async {
            match account.protocol {
                Protocol::Imap => {
                    let mut session = imap_routed(account, password, &self.incoming).await?;
                    session.examine("INBOX").await?;
                    session.logout().await?;
                }
                Protocol::Pop3 => {
                    let mut session = pop_routed(account, password, &self.incoming).await?;
                    session.command("STAT").await?;
                    session.command("QUIT").await?;
                }
            }
            anyhow::Ok(())
        })
        .await
        .context("Mail connection test timed out")?
    }
    pub async fn sync_folder(
        &self,
        account: &Account,
        password: &SecretString,
        known: &HashSet<String>,
        folder: &str,
        output: Sender<MailSyncItem>,
    ) -> anyhow::Result<Vec<String>> {
        match account.protocol {
            Protocol::Imap => {
                sync_imap_session(
                    imap_routed(account, password, &self.incoming).await?,
                    account,
                    known,
                    output,
                    Some(folder),
                )
                .await
            }
            Protocol::Pop3 => {
                anyhow::ensure!(
                    folder.eq_ignore_ascii_case("INBOX"),
                    "POP3 downloads Inbox only; other folders stay on the device."
                );
                sync_pop_session(
                    pop_routed(account, password, &self.incoming).await?,
                    account,
                    known,
                    output,
                )
                .await
            }
        }
    }
    pub async fn set_flags(
        &self,
        account: &Account,
        password: &SecretString,
        mail: &Mail,
        flags: crate::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            account.protocol == Protocol::Imap,
            "POP3 flags must be saved on the device."
        );
        flags_imap_session(
            imap_routed(account, password, &self.incoming).await?,
            mail,
            flags,
        )
        .await
    }
    pub async fn move_mail(
        &self,
        account: &Account,
        password: &SecretString,
        mail: &Mail,
        folder: &str,
    ) -> anyhow::Result<Option<String>> {
        anyhow::ensure!(
            account.protocol == Protocol::Imap,
            "POP3 folders must be saved on the device."
        );
        move_imap_session(
            imap_routed(account, password, &self.incoming).await?,
            mail,
            folder,
        )
        .await
    }
    pub async fn resolve_move(
        &self,
        account: &Account,
        password: &SecretString,
        receipt: &crate::mail_actions::MoveReceipt,
    ) -> anyhow::Result<StoredMail> {
        anyhow::ensure!(
            account.protocol == Protocol::Imap && account.id == receipt.account,
            "Choose a move in this IMAP account."
        );
        let fingerprint = receipt
            .fingerprint
            .as_ref()
            .context("Missing move identity proof.")?;
        let mut session = imap_routed(account, password, &self.incoming).await?;
        let result = recovery::resolve_session(
            &mut session,
            &account.id,
            &receipt.folder,
            fingerprint,
            receipt.current.as_ref().map(|m| m.remote_id.as_str()),
        )
        .await;
        let _ = session.logout().await;
        result
    }
    pub async fn sent(
        &self,
        account: &Account,
        password: &SecretString,
    ) -> anyhow::Result<sent::SentMailbox> {
        anyhow::ensure!(
            account.protocol == Protocol::Imap,
            "POP3 keeps Sent copies locally."
        );
        sent::SentMailbox::from_session(
            imap_routed(account, password, &self.incoming).await?,
            &account.sent_folder,
        )
        .await
    }
    fn smtp_transport(
        &self,
        account: &Account,
        password: &SecretString,
    ) -> anyhow::Result<lettre::AsyncSmtpTransport<lettre::Tokio1Executor>> {
        use lettre::transport::smtp::{
            authentication::{Credentials, Mechanism},
            client::{Tls, TlsParameters},
        };
        anyhow::ensure!(
            account.smtp_auth != SmtpAuth::None,
            "Hosted SMTP requires authentication."
        );
        #[allow(unused_mut)]
        let mut parameters = TlsParameters::builder(account.smtp_host.clone());
        #[cfg(test)]
        if let Some(ca) = &self.smtp_test_ca {
            parameters = parameters.add_root_certificate(ca.clone());
        }
        let tls = parameters.build()?;
        // builder_dangerous selects the pinned IP, then mandatory TLS is set
        // explicitly with the original hostname for certificate verification.
        let builder = lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous(
            self.smtp.ip().to_string(),
        )
        .port(self.smtp.port())
        .timeout(Some(Duration::from_secs(30)))
        .tls(match account.smtp_security() {
            ConnectionSecurity::Tls => Tls::Wrapper(tls),
            ConnectionSecurity::StartTls => Tls::Required(tls),
        })
        .credentials(Credentials::new(
            account.smtp_username().into(),
            password.expose_secret().into(),
        ));
        let builder = match account.smtp_auth {
            SmtpAuth::Plain => builder.authentication(vec![Mechanism::Plain]),
            SmtpAuth::Login => builder.authentication(vec![Mechanism::Login]),
            _ => builder,
        };
        Ok(builder.build())
    }
    pub async fn probe_smtp(
        &self,
        account: &Account,
        password: &SecretString,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.smtp_transport(account, password)?
                .test_connection()
                .await?,
            "SMTP connection was not accepted."
        );
        Ok(())
    }
    pub async fn send_raw(
        &self,
        account: &Account,
        password: &SecretString,
        envelope: &lettre::address::Envelope,
        raw: &[u8],
    ) -> Result<(), DeliveryFailure> {
        let transport = self.smtp_transport(account, password).map_err(|_| {
            DeliveryFailure::Rejected("Check SMTP authentication and TLS settings.".into())
        })?;
        deliver_raw(transport, envelope, raw).await
    }
}

#[cfg(test)]
#[path = "gateway_tests.rs"]
mod tests;
