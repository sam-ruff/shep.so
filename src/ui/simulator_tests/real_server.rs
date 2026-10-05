//! An owned GreenMail process and an independent IMAP oracle for UI scenarios.
use crate::{
    credentials::{Backend, Credentials, Scope},
    model::{Account, ConnectionSecurity, IncomingAuth, Protocol, SentCopyPolicy, SmtpAuth},
};
use anyhow::Context;
use futures::TryStreamExt;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Tokio1Executor,
    address::Envelope,
    transport::smtp::{
        authentication::{Credentials as SmtpCredentials, Mechanism},
        client::{Certificate, Tls, TlsParameters},
    },
};
use mailparse::MailHeaderMap;
use secrecy::SecretString;
use std::{
    collections::HashMap,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[path = "real_server/relay.rs"]
mod relay;

const IMAGE: &str = "greenmail/standalone:2.1.14@sha256:1ef95a966418cd09b7ea91d504d8c0826bbe7a2f6e679a75c601a831587c1626";
const PASSWORD: &str = "owned-test-mail-only";
const CERTIFICATE: &[u8] = include_bytes!("../../../shared/mail-core/tests/fixtures/tls-cert.pem");
const NETWORK_DEADLINE: Duration = Duration::from_secs(10);
type Session = async_imap::Session<tokio_native_tls::TlsStream<tokio::net::TcpStream>>;

pub struct RealServer {
    container: String,
    imap_port: u16,
    smtp_port: u16,
    pop3_port: u16,
    relay: Option<relay::ImapRelay>,
    _files: tempfile::TempDir,
}

#[derive(Debug)]
pub struct ServerMessage {
    pub uid: u32,
    pub validity: u32,
    pub message_id: String,
    pub subject: String,
    pub flags: Vec<String>,
    pub raw: Vec<u8>,
}

impl ServerMessage {
    pub fn remote_id(&self) -> String {
        format!("{}.{}", self.validity, self.uid)
    }
}

impl RealServer {
    pub async fn start() -> anyhow::Result<Self> {
        let files = tempfile::tempdir().context("Create the owned mail-server directory")?;
        let keystore = files.path().join("greenmail.p12");
        let fixtures =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("shared/mail-core/tests/fixtures");
        let mut openssl = Command::new("openssl");
        openssl.args(["pkcs12", "-export", "-in"]);
        openssl.arg(fixtures.join("tls-cert.pem"));
        openssl.arg("-inkey").arg(fixtures.join("tls-private.pem"));
        openssl.args(["-passout", "pass:changeit", "-out"]);
        openssl.arg(&keystore);
        command(openssl)
            .await
            .context("Create the test TLS keystore")?;

        let mut server = Self {
            container: format!("shep-real-mail-{}", uuid::Uuid::new_v4()),
            imap_port: 0,
            smtp_port: 0,
            pop3_port: 0,
            relay: None,
            _files: files,
        };
        let mut run = Command::new("docker");
        run.args([
            "run",
            "--detach",
            "--rm",
            "--name",
            &server.container,
            "--label",
            "so.shep.test=real-mail",
            "--publish",
            "127.0.0.1::3993",
            "--publish",
            "127.0.0.1::3465",
            "--publish",
            "127.0.0.1::3995",
            "--memory",
            "512m",
            "--env",
            "JAVA_OPTS=-Djava.net.preferIPv4Stack=true -Xmx256m",
            "--env",
            "GREENMAIL_OPTS=-Dgreenmail.setup.test.imaps -Dgreenmail.setup.test.smtps -Dgreenmail.setup.test.pop3s -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.users=alice:owned-test-mail-only@shep.test,bob:owned-test-mail-only@shep.test -Dgreenmail.tls.keystore.file=/tmp/shep.p12 -Dgreenmail.tls.keystore.password=changeit",
        ]);
        run.arg("--mount").arg(format!(
            "type=bind,source={},target=/tmp/shep.p12,readonly",
            keystore.display()
        ));
        // The exported file is private to this test; Docker reads it as this UID.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&keystore, std::fs::Permissions::from_mode(0o644))?;
        }
        run.arg(IMAGE);
        command(run)
            .await
            .context("Start disposable GreenMail; Docker must be available")?;
        server.imap_port = server.port(3993).await?;
        server.smtp_port = server.port(3465).await?;
        server.pop3_port = server.port(3995).await?;
        server.wait_ready().await?;
        server.relay = Some(relay::ImapRelay::start(server.imap_port).await?);
        for user in ["alice", "bob"] {
            tokio::time::timeout(NETWORK_DEADLINE, async {
                let mut session = server.session(user).await?;
                for folder in ["Archive", "Trash", "Sent", "Projects", "Projects.Design"] {
                    session.create(folder).await?;
                }
                session.logout().await?;
                anyhow::Ok(())
            })
            .await
            .context("Create fixture folders deadline")??;
        }
        Ok(server)
    }

    pub fn account(&self, user: &str) -> Account {
        assert!(matches!(user, "alice" | "bob"), "Unknown owned mail user");
        Account {
            id: format!("real-{user}"),
            name: match user {
                "alice" => "Alice",
                _ => "Bob",
            }
            .into(),
            email: format!("{user}@shep.test"),
            protocol: Protocol::Imap,
            host: "127.0.0.1".into(),
            port: self
                .relay
                .as_ref()
                .map_or(self.imap_port, |relay| relay.port()),
            username: user.into(),
            smtp_host: "127.0.0.1".into(),
            smtp_port: self.smtp_port,
            incoming_security: ConnectionSecurity::Tls,
            incoming_auth: IncomingAuth::Password,
            smtp_security: Some(ConnectionSecurity::Tls),
            smtp_auth: SmtpAuth::Plain,
            smtp_username: user.into(),
            smtp_separate_password: false,
            sent_copy: SentCopyPolicy::Automatic,
            sent_folder: "Sent".into(),
        }
    }

    pub fn pop3_account(&self, user: &str) -> Account {
        Account {
            protocol: Protocol::Pop3,
            port: self.pop3_port,
            ..self.account(user)
        }
    }

    pub async fn credentials(&self, scope: Scope) -> anyhow::Result<Credentials> {
        let credentials = Credentials::with_backend(scope, MemoryCredentials::default());
        for user in ["alice", "bob"] {
            credentials
                .write(&self.account(user).id, SecretString::from(PASSWORD))
                .await?;
        }
        Ok(credentials)
    }

    pub async fn seed(&self, count: usize) -> anyhow::Result<Vec<String>> {
        let mut ids = Vec::with_capacity(count);
        for index in 0..count {
            let id = format!("<shep-seed-{index:04}@shep.test>");
            let subject = format!("Server message {index:04}");
            let mut raw = format!(
                "From: Bob <bob@shep.test>\r\nTo: Alice <alice@shep.test>\r\n\
                 Message-ID: {id}\r\nSubject: {subject}\r\n\
                 Date: Sun, 04 Oct 2026 12:00:00 +0000\r\nMIME-Version: 1.0\r\n"
            );
            if index == 0 {
                raw.push_str(concat!(
                    "Content-Type: multipart/mixed; boundary=\"shep-real-mail-boundary\"\r\n\r\n",
                    "--shep-real-mail-boundary\r\n",
                    "Content-Type: text/plain; charset=UTF-8\r\n\r\n",
                    "A real SMTP message with a retained attachment.\r\n",
                    "--shep-real-mail-boundary\r\n",
                    "Content-Type: text/plain; name=\"evidence.txt\"\r\n",
                    "Content-Disposition: attachment; filename=\"evidence.txt\"\r\n",
                    "Content-Transfer-Encoding: base64\r\n\r\n",
                    "U2hlcCByZWFsLXNlcnZlciBhdHRhY2htZW50Cg==\r\n",
                    "--shep-real-mail-boundary--\r\n"
                ));
            } else {
                raw.push_str(&format!(
                    "Content-Type: text/plain; charset=UTF-8\r\n\r\n\
                     Deterministic message {index:04} delivered over authenticated SMTP.\r\n"
                ));
            }
            self.deliver("alice", raw.as_bytes()).await?;
            ids.push(id);
        }
        let observed = self.messages("alice", "INBOX").await?;
        let observed_ids: std::collections::HashSet<_> =
            observed.iter().map(|mail| &mail.message_id).collect();
        anyhow::ensure!(
            ids.iter().all(|id| observed_ids.contains(id)),
            "GreenMail did not retain every seeded Message-ID"
        );
        Ok(ids)
    }

    pub async fn deliver(&self, recipient: &str, raw: &[u8]) -> anyhow::Result<()> {
        let account = self.account(recipient);
        let envelope = Envelope::new(Some("bob@shep.test".parse()?), vec![account.email.parse()?])?;
        tokio::time::timeout(
            NETWORK_DEADLINE,
            self.smtp("bob", PASSWORD)?.send_raw(&envelope, raw),
        )
        .await
        .context("SMTP seed deadline")??;
        Ok(())
    }

    pub fn large_message(index: usize, body_bytes: usize) -> Vec<u8> {
        let headers = format!(
            "From: Bob <bob@shep.test>\r\nTo: Alice <alice@shep.test>\r\n\
             Message-ID: <shep-large-{index:04}@shep.test>\r\n\
             Subject: Large server message {index:04}\r\n\
             Date: Sun, 04 Oct 2026 13:00:00 +0000\r\n\
             MIME-Version: 1.0\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n"
        );
        let mut raw = headers.into_bytes();
        let line = b"Bounded real-server download fixture keeps UI input observable.\r\n";
        let mut remaining = body_bytes;
        while remaining > 0 {
            let length = remaining.min(line.len());
            raw.extend_from_slice(&line[..length]);
            remaining -= length;
        }
        raw.extend_from_slice(b"\r\n");
        raw
    }

    pub fn hold_imap(&self) {
        self.relay.as_ref().expect("Started IMAP relay").hold();
    }

    pub fn release_imap(&self) {
        self.relay.as_ref().expect("Started IMAP relay").release();
    }

    pub fn set_imap_delay(&self, delay: Duration) {
        self.relay
            .as_ref()
            .expect("Started IMAP relay")
            .set_delay(delay);
    }

    pub fn imap_forwarded_bytes(&self) -> u64 {
        self.relay
            .as_ref()
            .expect("Started IMAP relay")
            .forwarded_bytes()
    }

    pub fn imap_connections(&self) -> usize {
        self.relay
            .as_ref()
            .expect("Started IMAP relay")
            .connections()
    }

    pub fn imap_held_responses(&self) -> usize {
        self.relay
            .as_ref()
            .expect("Started IMAP relay")
            .held_responses()
    }

    pub async fn messages(&self, user: &str, folder: &str) -> anyhow::Result<Vec<ServerMessage>> {
        tokio::time::timeout(NETWORK_DEADLINE, async {
            let mut session = self.session(user).await?;
            let mailbox = session.examine(folder).await?;
            let mut messages = Vec::new();
            if mailbox.exists > 0 {
                let mut fetched = session.uid_fetch("1:*", "(UID FLAGS BODY.PEEK[])").await?;
                while let Some(message) = fetched.try_next().await? {
                    let raw = message
                        .body()
                        .context("Missing raw server message")?
                        .to_vec();
                    let parsed = mailparse::parse_mail(&raw)?;
                    messages.push(ServerMessage {
                        uid: message.uid.context("Missing server UID")?,
                        validity: mailbox.uid_validity.context("Missing server UIDVALIDITY")?,
                        message_id: parsed
                            .headers
                            .get_first_value("Message-ID")
                            .context("Missing server Message-ID")?,
                        subject: parsed
                            .headers
                            .get_first_value("Subject")
                            .unwrap_or_default(),
                        flags: message.flags().map(flag).collect(),
                        raw,
                    });
                }
            }
            session.logout().await?;
            messages.sort_by_key(|message| message.uid);
            anyhow::Ok(messages)
        })
        .await
        .context("IMAP oracle deadline")?
    }

    pub async fn folders(&self, user: &str) -> anyhow::Result<Vec<String>> {
        tokio::time::timeout(NETWORK_DEADLINE, async {
            let mut session = self.session(user).await?;
            let mut folders = {
                let names = session.list(Some(""), Some("*")).await?;
                names
                    .map_ok(|name| name.name().to_owned())
                    .try_collect::<Vec<_>>()
                    .await?
            };
            session.logout().await?;
            folders.sort();
            anyhow::Ok(folders)
        })
        .await
        .context("IMAP folder oracle deadline")?
    }

    pub async fn rejects_wrong_password(&self) -> anyhow::Result<bool> {
        let client = self.client().await?;
        let imap =
            tokio::time::timeout(NETWORK_DEADLINE, client.login("alice", "not-the-password"))
                .await
                .context("IMAP authentication deadline")?;
        let imap_rejected = matches!(imap, Err((async_imap::error::Error::No(_), _)));
        let smtp = tokio::time::timeout(
            NETWORK_DEADLINE,
            self.smtp("alice", "not-the-password")?.test_connection(),
        )
        .await
        .context("SMTP authentication deadline")?;
        let smtp_rejected =
            smtp.is_err_and(|error| error.status().is_some_and(|code| u16::from(code) == 535));
        Ok(imap_rejected && smtp_rejected)
    }

    async fn port(&self, internal: u16) -> anyhow::Result<u16> {
        let mut process = Command::new("docker");
        process.args(["port", &self.container, &format!("{internal}/tcp")]);
        let output = command(process).await?;
        let address: std::net::SocketAddr = output.trim().parse()?;
        anyhow::ensure!(address.ip().is_loopback(), "Mail port is not loopback-only");
        Ok(address.port())
    }

    async fn wait_ready(&self) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if let Ok(mut session) = self.session("alice").await {
                tokio::time::timeout(NETWORK_DEADLINE, session.logout()).await??;
                return Ok(());
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "GreenMail did not become ready"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn client(
        &self,
    ) -> anyhow::Result<async_imap::Client<tokio_native_tls::TlsStream<tokio::net::TcpStream>>>
    {
        tokio::time::timeout(NETWORK_DEADLINE, async {
            let tcp = tokio::net::TcpStream::connect(("127.0.0.1", self.imap_port)).await?;
            let connector = native_tls::TlsConnector::builder()
                .add_root_certificate(native_tls::Certificate::from_pem(CERTIFICATE)?)
                .build()?;
            let tls = tokio_native_tls::TlsConnector::from(connector)
                .connect("127.0.0.1", tcp)
                .await?;
            let mut client = async_imap::Client::new(tls);
            client
                .read_response()
                .await?
                .context("Missing IMAP greeting")?;
            anyhow::Ok(client)
        })
        .await
        .context("IMAP connection deadline")?
    }

    async fn session(&self, user: &str) -> anyhow::Result<Session> {
        let client = self.client().await?;
        tokio::time::timeout(NETWORK_DEADLINE, client.login(user, PASSWORD))
            .await
            .context("IMAP login deadline")?
            .map_err(|(error, _)| error.into())
    }

    fn smtp(
        &self,
        user: &str,
        password: &str,
    ) -> anyhow::Result<AsyncSmtpTransport<Tokio1Executor>> {
        let tls = TlsParameters::builder("127.0.0.1".into())
            .add_root_certificate(Certificate::from_pem(CERTIFICATE)?)
            .build()?;
        Ok(
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous("127.0.0.1")
                .port(self.smtp_port)
                .tls(Tls::Wrapper(tls))
                .authentication(vec![Mechanism::Plain])
                .credentials(SmtpCredentials::new(user.into(), password.into()))
                .timeout(Some(NETWORK_DEADLINE))
                .build(),
        )
    }
}

impl Drop for RealServer {
    fn drop(&mut self) {
        let mut command = Command::new("docker");
        command.args(["rm", "--force", &self.container]);
        if let Err(error) = command_blocking(command) {
            eprintln!(
                "Could not remove owned mail container {}: {error}",
                self.container
            );
        }
    }
}

fn flag(value: async_imap::types::Flag<'_>) -> String {
    use async_imap::types::Flag;
    match value {
        Flag::Seen => "\\Seen",
        Flag::Answered => "\\Answered",
        Flag::Flagged => "\\Flagged",
        Flag::Deleted => "\\Deleted",
        Flag::Draft => "\\Draft",
        Flag::Recent => "\\Recent",
        Flag::MayCreate => "\\*",
        Flag::Custom(name) => return name.into_owned(),
    }
    .into()
}

async fn command(command: Command) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(|| command_blocking(command)).await?
}

fn command_blocking(mut command: Command) -> anyhow::Result<String> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while child.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Owned mail-server command exceeded its deadline");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output()?;
    anyhow::ensure!(
        output.status.success(),
        "Owned mail-server command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

#[derive(Default)]
struct MemoryCredentials(HashMap<String, SecretString>);

impl Backend for MemoryCredentials {
    fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>> {
        Ok(self.0.get(key).cloned())
    }

    fn write(&mut self, key: &str, value: SecretString) -> anyhow::Result<()> {
        self.0.insert(key.to_owned(), value);
        Ok(())
    }

    fn delete(&mut self, key: &str) -> anyhow::Result<()> {
        self.0.remove(key);
        Ok(())
    }
}
