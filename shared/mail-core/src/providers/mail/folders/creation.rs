use super::*;
use crate::folder_actions::creation::{self, Connection as CreationConnection, CreateOutcome};
use tokio::sync::Mutex;

struct SessionConnection<
    'a,
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug,
> {
    session: Mutex<&'a mut async_imap::Session<T>>,
    encoding: NameEncoding,
    usable: std::sync::atomic::AtomicBool,
}

impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug>
    SessionConnection<'_, T>
{
    async fn listing(
        &self,
        reference: &str,
        pattern: &str,
        namespace: bool,
    ) -> anyhow::Result<Option<Mailbox>> {
        use std::sync::atomic::Ordering;
        anyhow::ensure!(
            self.usable.load(Ordering::Acquire),
            "Reconnect before checking the folder creation result."
        );
        let mut session = self.session.lock().await;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            let quote = |value: &str| -> anyhow::Result<String> {
                anyhow::ensure!(
                    !value.chars().any(char::is_control),
                    "Invalid folder listing reference."
                );
                Ok(format!(
                    "\"{}\"",
                    value.replace('\\', "\\\\").replace('"', "\\\"")
                ))
            };
            let command = format!("LIST {} {}", quote(reference)?, quote(pattern)?);
            let tag = session.run_command(&command).await?;
            let mut found = None;
            let mut count = 0;
            loop {
                let response = session
                    .read_response()
                    .await?
                    .context("The server disconnected during the folder listing.")?;
                match response.parsed() {
                    Response::MailboxData(MailboxDatum::List {
                        name,
                        delimiter,
                        name_attributes,
                    }) => {
                        count += 1;
                        anyhow::ensure!(
                            count <= 8192,
                            "The folder listing exceeded the supported limit."
                        );
                        anyhow::ensure!(
                            name.len() <= 1024 && !name.chars().any(char::is_control),
                            "The server returned an invalid folder name."
                        );
                        anyhow::ensure!(
                            delimiter
                                .as_deref()
                                .is_none_or(|value| value.chars().count() == 1),
                            "The server returned an invalid folder separator."
                        );
                        let mailbox =
                            mailbox(name, delimiter.as_deref(), name_attributes, self.encoding);
                        if namespace
                            || mailbox.name == pattern
                            || mailbox.path() == pattern
                            || (pattern.eq_ignore_ascii_case("INBOX")
                                && mailbox.name.eq_ignore_ascii_case("INBOX"))
                        {
                            anyhow::ensure!(
                                found.as_ref().is_none_or(|previous| previous == &mailbox),
                                "The server returned conflicting folder identities."
                            );
                            found = Some(mailbox);
                        }
                    }
                    Response::Done {
                        tag: received,
                        status,
                        ..
                    } => {
                        anyhow::ensure!(
                            *received == tag && *status == Status::Ok,
                            "The server did not confirm the complete folder listing."
                        );
                        return Ok(found);
                    }
                    Response::Data {
                        status: Status::Bye,
                        ..
                    } => anyhow::bail!("The server disconnected during the folder listing."),
                    _ => {}
                }
            }
        })
        .await
        .context("The folder listing took too long.")
        .and_then(|result| result);
        if result.is_err() {
            self.usable.store(false, Ordering::Release);
        }
        result
    }
}

#[async_trait]
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug>
    CreationConnection for SessionConnection<'_, T>
{
    async fn inspect(&self, path: String) -> anyhow::Result<Option<Mailbox>> {
        self.listing("", &path, false).await
    }
    async fn namespace(&self, reference: String) -> anyhow::Result<Mailbox> {
        self.listing(&reference, "", true)
            .await?
            .context("The server did not report its folder namespace.")
    }
    async fn create(&self, path: String) -> CreateOutcome {
        use async_imap::error::Error;
        use std::sync::atomic::Ordering;
        if !self.usable.load(Ordering::Acquire) {
            return CreateOutcome::Uncertain("Reconnect before creating the folder.".into());
        }
        let mut session = self.session.lock().await;
        match tokio::time::timeout(Duration::from_secs(30), session.create(&path)).await {
            Ok(Ok(())) => CreateOutcome::Acknowledged,
            Ok(Err(Error::No(_) | Error::Bad(_) | Error::Validate(_))) => CreateOutcome::Rejected(
                "The server refused folder creation. Check the folder name and your permissions."
                    .into(),
            ),
            _ => {
                self.usable.store(false, Ordering::Release);
                CreateOutcome::Uncertain("The connection ended before folder creation was confirmed. Retry the same folder after reconnecting.".into())
            }
        }
    }
}

fn connection<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug>(
    session: &mut async_imap::Session<T>,
    encoding: NameEncoding,
) -> SessionConnection<'_, T> {
    SessionConnection {
        session: Mutex::new(session),
        encoding,
        usable: std::sync::atomic::AtomicBool::new(true),
    }
}

pub async fn ensure_exact<
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
>(
    session: &mut async_imap::Session<T>,
    wire_path: &str,
) -> anyhow::Result<Mailbox> {
    creation::valid_path(wire_path)?;
    let capabilities = tokio::time::timeout(Duration::from_secs(30), session.capabilities())
        .await
        .context("The server took too long to report its capabilities.")??;
    creation::ensure(&connection(session, encoding(&capabilities)), wire_path).await
}

pub async fn exists_exact<
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
>(
    session: &mut async_imap::Session<T>,
    wire_path: &str,
) -> anyhow::Result<bool> {
    creation::valid_path(wire_path)?;
    let capabilities = tokio::time::timeout(Duration::from_secs(30), session.capabilities())
        .await
        .context("The server took too long to report its capabilities.")??;
    creation::exists(&connection(session, encoding(&capabilities)), wire_path).await
}

impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug>
    ImapFolders<T>
{
    pub async fn create_folder(
        &mut self,
        parent: Option<&str>,
        name: &str,
    ) -> anyhow::Result<Mailbox> {
        let target = self.plan_folder(parent, name).await?;
        self.ensure_folder_exact(&target.name).await
    }

    pub async fn ensure_folder_exact(&mut self, wire_path: &str) -> anyhow::Result<Mailbox> {
        creation::ensure(&connection(&mut self.session, self.encoding), wire_path).await
    }

    pub async fn find_folder_exact(&mut self, wire_path: &str) -> anyhow::Result<Option<Mailbox>> {
        creation::valid_path(wire_path)?;
        let result = connection(&mut self.session, self.encoding)
            .inspect(wire_path.into())
            .await?;
        if let Some(mailbox) = &result {
            anyhow::ensure!(
                mailbox.selectable && !mailbox.non_existent,
                "The destination exists but cannot receive mail."
            );
        }
        Ok(result)
    }

    pub async fn plan_folder(
        &mut self,
        parent: Option<&str>,
        name: &str,
    ) -> anyhow::Result<Mailbox> {
        creation::valid_path(name)?;
        let connection = connection(&mut self.session, self.encoding);
        let root = connection.namespace(parent.unwrap_or("").into()).await?;
        let parent = match parent {
            Some(path) => {
                creation::valid_path(path)?;
                Some(
                    connection
                        .inspect(path.into())
                        .await?
                        .context("The parent folder no longer exists. Refresh folders.")?,
                )
            }
            None => None,
        };
        creation::plan(&root, parent.as_ref(), name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    async fn script(
        commands: Vec<(&str, &str)>,
        action: impl AsyncFnOnce(&mut ImapFolders<tokio::io::DuplexStream>),
    ) {
        let (client, server) = tokio::io::duplex(16384);
        let peer = async {
            let mut server = BufReader::new(server);
            server
                .get_mut()
                .write_all(b"* OK fixture\r\n")
                .await
                .expect("greeting");
            let mut line = String::new();
            server.read_line(&mut line).await.expect("login");
            let tag = line.split_whitespace().next().expect("tag");
            server
                .get_mut()
                .write_all(format!("{tag} OK login\r\n").as_bytes())
                .await
                .expect("login response");
            for (expected, response) in commands {
                line.clear();
                server.read_line(&mut line).await.expect("command");
                let (tag, command) = line.trim_end().split_once(' ').expect("tagged command");
                assert_eq!(command, expected);
                if response == "EOF" {
                    return;
                }
                server
                    .get_mut()
                    .write_all(response.replace("$TAG", tag).as_bytes())
                    .await
                    .expect("response");
            }
        };
        let client = async {
            let mut client = async_imap::Client::new(client);
            client.read_response().await.expect("greeting");
            let session = client.login("fixture", "fixture").await.expect("login");
            let mut folders = ImapFolders {
                session,
                encoding: NameEncoding::ImapUtf7,
            };
            action(&mut folders).await;
        };
        tokio::join!(client, peer);
    }

    #[tokio::test]
    async fn empty_list_arguments_and_root_creation_are_real_wire_commands() {
        script(
            vec![
                (
                    "LIST \"\" \"\"",
                    "* LIST (\\Noselect) \"/\" \"\"\r\n$TAG OK namespace\r\n",
                ),
                ("LIST \"\" \"Archive\"", "$TAG OK absent\r\n"),
                (
                    "LIST \"Archive\" \"\"",
                    "* LIST (\\Noselect) \"/\" \"\"\r\n$TAG OK namespace\r\n",
                ),
                ("CREATE \"Archive\"", "$TAG OK created\r\n"),
                (
                    "LIST \"\" \"Archive\"",
                    "* LIST () \"/\" \"Archive\"\r\n$TAG OK complete\r\n",
                ),
            ],
            async |folders| {
                assert_eq!(
                    folders
                        .create_folder(None, "Archive")
                        .await
                        .expect("created")
                        .name,
                    "Archive"
                );
            },
        )
        .await;
    }

    #[tokio::test]
    async fn no_then_existing_is_success_without_another_create() {
        script(
            vec![
                ("LIST \"\" \"Archive\"", "$TAG OK absent\r\n"),
                (
                    "LIST \"Archive\" \"\"",
                    "* LIST (\\Noselect) \".\" \"\"\r\n$TAG OK namespace\r\n",
                ),
                ("CREATE \"Archive\"", "$TAG NO already exists\r\n"),
                (
                    "LIST \"\" \"Archive\"",
                    "* LIST () \".\" \"Archive\"\r\n$TAG OK complete\r\n",
                ),
            ],
            async |folders| {
                let adapter = connection(&mut folders.session, folders.encoding);
                assert_eq!(
                    creation::ensure(&adapter, "Archive")
                        .await
                        .expect("observed")
                        .name,
                    "Archive"
                );
            },
        )
        .await;
    }

    #[tokio::test]
    async fn unicode_child_preserves_wire_parent_and_server_delimiter() {
        script(
            vec![
                (
                    "LIST \"INBOX.Teams\" \"\"",
                    "* LIST (\\Noselect) \".\" \"INBOX.\"\r\n$TAG OK namespace\r\n",
                ),
                (
                    "LIST \"\" \"INBOX.Teams\"",
                    "* LIST (\\Noselect) \".\" \"INBOX.Teams\"\r\n$TAG OK parent\r\n",
                ),
                (
                    "LIST \"\" \"INBOX.Teams.&ZeVnLIqe- &- work\"",
                    "$TAG OK absent\r\n",
                ),
                (
                    "LIST \"INBOX.Teams.&ZeVnLIqe- &- work\" \"\"",
                    "* LIST (\\Noselect) \".\" \"INBOX.\"\r\n$TAG OK namespace\r\n",
                ),
                (
                    "LIST \"\" \"INBOX.Teams\"",
                    "* LIST (\\Noselect) \".\" \"INBOX.Teams\"\r\n$TAG OK parent\r\n",
                ),
                (
                    "CREATE \"INBOX.Teams.&ZeVnLIqe- &- work\"",
                    "$TAG OK created\r\n",
                ),
                (
                    "LIST \"\" \"INBOX.Teams.&ZeVnLIqe- &- work\"",
                    "* LIST () \".\" \"INBOX.Teams.&ZeVnLIqe- &- work\"\r\n$TAG OK complete\r\n",
                ),
            ],
            async |folders| {
                assert_eq!(
                    folders
                        .create_folder(Some("INBOX.Teams"), "日本語 & work")
                        .await
                        .expect("created")
                        .name,
                    "INBOX.Teams.&ZeVnLIqe- &- work"
                );
            },
        )
        .await;
    }

    #[tokio::test]
    async fn fresh_connection_retry_observes_existing_without_create() {
        script(
            vec![
                (
                    "LIST \"\" \"\"",
                    "* LIST (\\Noselect) \"/\" \"\"\r\n$TAG OK namespace\r\n",
                ),
                (
                    "LIST \"\" \"Archive\"",
                    "* LIST () \"/\" \"Archive\"\r\n$TAG OK present\r\n",
                ),
            ],
            async |folders| {
                assert_eq!(
                    folders
                        .create_folder(None, "Archive")
                        .await
                        .expect("already created")
                        .name,
                    "Archive"
                );
            },
        )
        .await;
    }

    #[tokio::test]
    async fn partial_wrong_tag_and_disconnected_list_never_prove_absence() {
        for response in [
            "* LIST () \"/\" \"Archive\"\r\n$TAG NO incomplete\r\n",
            "X999 OK wrong tag\r\n",
            "EOF",
        ] {
            script(vec![("LIST \"\" \"Archive\"", response)], async |folders| {
                let adapter = connection(&mut folders.session, folders.encoding);
                assert!(creation::exists(&adapter, "Archive").await.is_err());
            })
            .await;
        }
    }

    #[tokio::test]
    async fn wildcard_listing_requires_exact_identity_and_selectable_target() {
        for (response, rejected) in [
            (
                "* LIST () \"/\" \"Archive2026\"\r\n$TAG OK complete\r\n",
                false,
            ),
            (
                "* LIST (\\Noselect) \"/\" \"Archive%\"\r\n$TAG OK complete\r\n",
                true,
            ),
        ] {
            script(
                vec![("LIST \"\" \"Archive%\"", response)],
                async |folders| {
                    let adapter = connection(&mut folders.session, folders.encoding);
                    let result = creation::exists(&adapter, "Archive%").await;
                    if rejected {
                        assert!(result.is_err());
                    } else {
                        assert!(!result.expect("checked absence"));
                    }
                },
            )
            .await;
        }
    }

    #[tokio::test]
    async fn transport_loss_does_not_issue_post_check_on_poisoned_session() {
        script(
            vec![
                ("LIST \"\" \"Archive\"", "$TAG OK absent\r\n"),
                (
                    "LIST \"Archive\" \"\"",
                    "* LIST (\\Noselect) \"/\" \"\"\r\n$TAG OK namespace\r\n",
                ),
                ("CREATE \"Archive\"", "EOF"),
            ],
            async |folders| {
                let adapter = connection(&mut folders.session, folders.encoding);
                assert!(creation::ensure(&adapter, "Archive").await.is_err());
            },
        )
        .await;
    }
}
