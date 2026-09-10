//! A checked LIST is required before changing a reviewed subtree. In particular,
//! a partial LIST followed by NO must never become an authoritative empty tree.
use super::*;
use crate::folder_actions::{Connection, Outcome, Step, valid_name};
use crate::folders::{Mailbox, NameEncoding};
use async_imap::imap_proto::{MailboxDatum, Response, Status};
use async_imap::types::{Capabilities, NameAttribute};

pub(super) fn encoding(capabilities: &Capabilities) -> NameEncoding {
    if capabilities.has_str("IMAP4rev2") && !capabilities.has_str("IMAP4rev1")
        || capabilities.has_str("UTF8=ONLY")
    {
        NameEncoding::Utf8
    } else {
        NameEncoding::ImapUtf7
    }
}

pub(super) fn mailbox(
    name: &str,
    delimiter: Option<&str>,
    attributes: &[NameAttribute<'_>],
    encoding: NameEncoding,
) -> Mailbox {
    let non_existent = attributes.iter().any(|attribute| matches!(attribute,
        NameAttribute::Extension(value) if value.trim_start_matches('\\').eq_ignore_ascii_case("NonExistent")));
    Mailbox {
        name: name.into(),
        delimiter: delimiter.and_then(|value| value.chars().next()),
        selectable: !non_existent && !attributes.contains(&NameAttribute::NoSelect),
        no_inferiors: attributes.contains(&NameAttribute::NoInferiors),
        non_existent,
        encoding,
    }
}

pub struct ImapFolders<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug> {
    session: async_imap::Session<T>,
    encoding: NameEncoding,
}

impl ImapFolders<Tls> {
    pub async fn open(account: &Account, password: &SecretString) -> anyhow::Result<Self> {
        anyhow::ensure!(
            account.protocol == Protocol::Imap,
            "POP3 folders are managed in the local cache."
        );
        tokio::time::timeout(Duration::from_secs(30), async {
            let mut session = imap(account, password).await?;
            let encoding = encoding(&session.capabilities().await?);
            Ok(Self { session, encoding })
        })
        .await
        .context("The mail server took too long to connect.")?
    }
}

#[async_trait]
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug> Connection
    for ImapFolders<T>
{
    async fn catalog(&mut self) -> anyhow::Result<Vec<Mailbox>> {
        tokio::time::timeout(Duration::from_secs(30), async {
            let tag = self.session.run_command("LIST \"\" \"*\"").await?;
            let mut catalog = Vec::new();
            loop {
                let response = self.session.read_response().await?.context("The server disconnected during the folder listing.")?;
                match response.parsed() {
                    Response::MailboxData(MailboxDatum::List { name, delimiter, name_attributes }) => {
                        catalog.push(mailbox(name, delimiter.as_deref(), name_attributes, self.encoding));
                    }
                    Response::Done { tag: received, status, .. } => {
                        anyhow::ensure!(*received == tag && *status == Status::Ok, "The server did not confirm the complete folder listing. Refresh before changing folders.");
                        return Ok(catalog);
                    }
                    Response::Data { status: Status::Bye, .. } => anyhow::bail!("The server disconnected during the folder listing."),
                    _ => {}
                }
            }
        }).await.context("The folder listing took too long. Refresh before changing folders.")?
    }

    async fn apply(&mut self, step: &Step) -> Outcome {
        if matches!(step, Step::Forget { .. }) {
            return Outcome::Rejected("A virtual container is managed in the local cache.".into());
        }
        let validation = (|| {
            let source = match step {
                Step::Rename {
                    source,
                    destination,
                } => {
                    valid_name(destination)?;
                    source
                }
                Step::Delete { source } | Step::Forget { source } => source,
            };
            valid_name(source)?;
            anyhow::ensure!(
                !source.eq_ignore_ascii_case("INBOX"),
                "Inbox cannot be moved or deleted."
            );
            Ok::<_, anyhow::Error>(())
        })();
        if let Err(error) = validation {
            return Outcome::Rejected(error.to_string());
        }
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            match step {
                Step::Rename {
                    source,
                    destination,
                } => self.session.rename(source, destination).await,
                Step::Delete { source } => self.session.delete(source).await,
                Step::Forget { .. } => unreachable!("Local-only step was rejected above"),
            }
        })
        .await;
        use async_imap::error::Error;
        match result {
            Ok(Ok(())) => Outcome::Applied,
            Ok(Err(Error::No(_) | Error::Bad(_) | Error::Validate(_))) => Outcome::Rejected("The server refused this folder change. Refresh the folders and check your access before retrying.".into()),
            _ => Outcome::Uncertain("The connection ended before the server confirmed the folder change. Check the server's folders before deciding how to continue.".into()),
        }
        // A later logout/refresh failure never reverses an acknowledged change.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn session() -> (
        ImapFolders<tokio::io::DuplexStream>,
        BufReader<tokio::io::DuplexStream>,
    ) {
        let (client, server) = tokio::io::duplex(8192);
        let mut server = BufReader::new(server);
        let handshake = async {
            server
                .get_mut()
                .write_all(b"* OK fixture ready\r\n")
                .await
                .unwrap();
            let mut line = String::new();
            server.read_line(&mut line).await.unwrap();
            let tag = line.split_whitespace().next().unwrap();
            server
                .get_mut()
                .write_all(format!("{tag} OK logged in\r\n").as_bytes())
                .await
                .unwrap();
        };
        let connection = async {
            let mut client = async_imap::Client::new(client);
            client.read_response().await.unwrap();
            client.login("fixture", "fixture").await.unwrap()
        };
        let (session, ()) = tokio::join!(connection, handshake);
        (
            ImapFolders {
                session,
                encoding: NameEncoding::ImapUtf7,
            },
            server,
        )
    }
    #[tokio::test]
    async fn listing_preserves_capabilities_and_requires_tagged_ok() {
        for status in ["OK", "NO", "BAD"] {
            let (mut connection, mut server) = session().await;
            let reply = async {
                let mut line = String::new();
                server.read_line(&mut line).await.unwrap();
                let (tag, command) = line.trim_end().split_once(' ').unwrap();
                assert_eq!(command, "LIST \"\" \"*\"");
                server.get_mut().write_all(format!("* LIST (\\Noselect) \"/\" \"Teams/\"\r\n* LIST (\\Noinferiors) \"/\" \"Teams/&ZeVnLIqe-\"\r\n* LIST (\\NonExistent) \".\" \"Missing\"\r\n{tag} {status} done\r\n").as_bytes()).await.unwrap();
            };
            let (catalog, ()) = tokio::join!(connection.catalog(), reply);
            if status == "OK" {
                let catalog = catalog.unwrap();
                assert!(!catalog[0].selectable);
                assert_eq!(catalog[0].name, "Teams/");
                assert!(catalog[1].no_inferiors && catalog[1].selectable);
                assert_eq!(
                    catalog[1].encoding.display(&catalog[1].name),
                    "Teams/日本語"
                );
                assert!(catalog[2].non_existent && !catalog[2].selectable);
            } else {
                assert!(catalog.is_err());
            }
        }
    }
    #[tokio::test]
    async fn rename_and_delete_distinguish_refusal_from_lost_acknowledgment() {
        for step in [
            Step::Rename {
                source: "A. Keep/\"quotes\"".into(),
                destination: "Archive/A. Keep/\"quotes\"".into(),
            },
            Step::Delete {
                source: "A. Keep".into(),
            },
        ] {
            for status in [Some("OK"), Some("NO"), Some("BAD"), None] {
                let (mut connection, mut server) = session().await;
                let reply = async {
                    let mut line = String::new();
                    server.read_line(&mut line).await.unwrap();
                    let (tag, command) = line.trim_end().split_once(' ').unwrap();
                    let expected = match &step {
                        Step::Rename {
                            source,
                            destination,
                        } => format!(
                            "RENAME {} {}",
                            receipts::quoted(source).unwrap(),
                            receipts::quoted(destination).unwrap()
                        ),
                        Step::Delete { source } => {
                            format!("DELETE {}", receipts::quoted(source).unwrap())
                        }
                        Step::Forget { .. } => unreachable!(),
                    };
                    assert_eq!(command, expected);
                    if let Some(status) = status {
                        server
                            .get_mut()
                            .write_all(format!("{tag} {status} result\r\n").as_bytes())
                            .await
                            .unwrap();
                    }
                    drop(server);
                };
                let (outcome, ()) = tokio::join!(connection.apply(&step), reply);
                match status {
                    Some("OK") => assert_eq!(outcome, Outcome::Applied),
                    Some(_) => assert!(matches!(outcome, Outcome::Rejected(_))),
                    None => assert!(matches!(outcome, Outcome::Uncertain(_))),
                }
            }
        }
    }
    #[tokio::test]
    async fn inbox_and_command_injection_are_rejected_before_writing() {
        let (mut connection, mut server) = session().await;
        for source in ["inbox", "A\r\nDELETE Other", "A\0B"] {
            assert!(matches!(
                connection
                    .apply(&Step::Delete {
                        source: source.into()
                    })
                    .await,
                Outcome::Rejected(_)
            ));
        }
        let mut line = String::new();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), server.read_line(&mut line))
                .await
                .is_err()
        );
    }
}
