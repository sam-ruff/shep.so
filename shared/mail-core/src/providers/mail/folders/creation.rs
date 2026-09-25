use super::*;
use crate::folder_actions::creation::{self, Connection as CreationConnection, CreateOutcome};
use tokio::sync::Mutex;

/// Optional server extensions that change how folders are discovered and created.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Extensions {
    /// CREATE-SPECIAL-USE: a created folder can carry its special-use attribute.
    pub create_special_use: bool,
    /// NAMESPACE (RFC 2342): the last discovery fallback.
    pub namespace: bool,
}
impl Extensions {
    pub(super) fn from(capabilities: &Capabilities) -> Self {
        Self {
            create_special_use: capabilities.has_str("CREATE-SPECIAL-USE"),
            namespace: capabilities.has_str("NAMESPACE"),
        }
    }
    fn without_special_use(self) -> Self {
        Self {
            create_special_use: false,
            ..self
        }
    }
}

struct SessionConnection<
    'a,
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug,
> {
    session: Mutex<&'a mut async_imap::Session<T>>,
    encoding: NameEncoding,
    extensions: Extensions,
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
    async fn namespace(&self, reference: String) -> anyhow::Result<Option<Mailbox>> {
        self.listing(&reference, "", true).await
    }
    async fn namespaces(&self) -> anyhow::Result<Option<Mailbox>> {
        use std::sync::atomic::Ordering;
        if !self.extensions.namespace {
            return Ok(None);
        }
        anyhow::ensure!(
            self.usable.load(Ordering::Acquire),
            "Reconnect before checking the folder namespace."
        );
        let mut session = self.session.lock().await;
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            super::namespace::exchange(session.get_mut()),
        )
        .await
        .context("The server took too long to report its folder namespace.")
        .and_then(|result| result);
        let personal = match result {
            Ok(personal) => personal,
            Err(error) => {
                self.usable.store(false, Ordering::Release);
                return Err(error);
            }
        };
        // The first personal namespace is the default for new folders.
        Ok(personal.into_iter().next().map(|namespace| Mailbox {
            name: namespace.prefix,
            delimiter: namespace.delimiter,
            selectable: false,
            encoding: self.encoding,
            no_inferiors: false,
            non_existent: false,
            role: None,
        }))
    }
    async fn create(&self, path: String, role: Option<FolderRole>) -> CreateOutcome {
        use async_imap::error::Error;
        use std::sync::atomic::Ordering;
        if !self.usable.load(Ordering::Acquire) {
            return CreateOutcome::Uncertain("Reconnect before creating the folder.".into());
        }
        let mut session = self.session.lock().await;
        let command = async {
            match role.filter(|_| self.extensions.create_special_use) {
                Some(role) => {
                    let name = match receipts::quoted(&path) {
                        Ok(name) => name,
                        Err(_) => {
                            return Err(Error::Validate(async_imap::error::ValidateError('\n')));
                        }
                    };
                    session
                        .run_command_and_check_ok(format!(
                            "CREATE {name} (USE ({}))",
                            role.attribute()
                        ))
                        .await
                }
                None => session.create(&path).await,
            }
        };
        match tokio::time::timeout(Duration::from_secs(30), command).await {
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
    extensions: Extensions,
) -> SessionConnection<'_, T> {
    SessionConnection {
        session: Mutex::new(session),
        encoding,
        extensions,
        usable: std::sync::atomic::AtomicBool::new(true),
    }
}

/// A missing move destination named like a logical special folder is created
/// with that special use; resolution against the catalog runs before this.
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
    creation::ensure(
        &connection(
            session,
            encoding(&capabilities),
            Extensions::from(&capabilities),
        ),
        wire_path,
        FolderRole::for_logical_name(wire_path),
    )
    .await
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
    creation::exists(
        &connection(
            session,
            encoding(&capabilities),
            Extensions::from(&capabilities).without_special_use(),
        ),
        wire_path,
    )
    .await
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
        creation::ensure(
            &connection(&mut self.session, self.encoding, self.extensions),
            wire_path,
            None,
        )
        .await
    }

    pub async fn ensure_planned_folder(&mut self, target: &Mailbox) -> anyhow::Result<Mailbox> {
        anyhow::ensure!(
            self.encoding == target.encoding,
            "The server's folder encoding changed. The saved folder request has not been retried."
        );
        self.ensure_folder_exact(&target.name).await
    }

    pub async fn create_planned_folder(&mut self, target: &Mailbox) -> CreateOutcome {
        if self.encoding != target.encoding {
            return CreateOutcome::Rejected(
                "The server's folder encoding changed. Refresh the saved request.".into(),
            );
        }
        connection(
            &mut self.session,
            self.encoding,
            self.extensions.without_special_use(),
        )
        .create(target.name.clone(), None)
        .await
    }

    pub async fn find_planned_folder(
        &mut self,
        target: &Mailbox,
    ) -> anyhow::Result<Option<Mailbox>> {
        anyhow::ensure!(
            self.encoding == target.encoding,
            "The server's folder encoding changed. The saved folder request could not be checked."
        );
        self.find_folder_exact(&target.name).await
    }

    pub async fn find_folder_exact(&mut self, wire_path: &str) -> anyhow::Result<Option<Mailbox>> {
        creation::valid_path(wire_path)?;
        let result = connection(
            &mut self.session,
            self.encoding,
            self.extensions.without_special_use(),
        )
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
        creation::valid_path(name).map_err(|error| creation::PlanRejected(error.to_string()))?;
        let connection = connection(
            &mut self.session,
            self.encoding,
            self.extensions.without_special_use(),
        );
        let root = creation::discover_namespace(&connection, parent.unwrap_or("")).await?;
        let parent = match parent {
            Some(path) => {
                creation::valid_path(path)
                    .map_err(|error| creation::PlanRejected(error.to_string()))?;
                Some(connection.inspect(path.into()).await?.ok_or_else(|| {
                    creation::PlanRejected(
                        "The parent folder no longer exists. Refresh folders.".into(),
                    )
                })?)
            }
            None => None,
        };
        creation::plan(&root, parent.as_ref(), name)
            .map_err(|error| creation::PlanRejected(error.to_string()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn invalid_child_name_is_a_typed_plan_rejection_before_create() {
        script(
            vec![(
                "LIST \"\" \"\"",
                "* LIST (\\NoSelect) \"/\" \"\"\r\n$TAG OK namespace\r\n",
            )],
            async |folders| {
                let error = folders
                    .plan_folder(None, "invalid/child")
                    .await
                    .expect_err("invalid leaf");
                assert!(error.downcast_ref::<creation::PlanRejected>().is_some());
            },
        )
        .await;
    }

    #[tokio::test]
    async fn incomplete_namespace_read_does_not_reject_the_requested_name() {
        script(
            vec![(
                "LIST \"\" \"\"",
                "* LIST (\\NoSelect) \"/\" \"\"\r\n$TAG NO unavailable\r\n",
            )],
            async |folders| {
                let error = folders
                    .plan_folder(None, "Projects")
                    .await
                    .expect_err("incomplete read");
                assert!(error.downcast_ref::<creation::PlanRejected>().is_none());
            },
        )
        .await;
    }

    #[tokio::test]
    async fn changed_encoding_rejects_saved_target_before_any_wire_command() {
        for (current, frozen) in [
            (NameEncoding::Utf8, NameEncoding::ImapUtf7),
            (NameEncoding::ImapUtf7, NameEncoding::Utf8),
        ] {
            script(vec![], async |folders| {
                folders.encoding = current;
                let target = Mailbox {
                    encoding: frozen,
                    ..Mailbox::flat("&ZeVnLIqe-".into())
                };
                assert!(
                    folders
                        .ensure_planned_folder(&target)
                        .await
                        .expect_err("changed encoding")
                        .to_string()
                        .contains("encoding changed")
                );
                assert!(
                    folders
                        .find_planned_folder(&target)
                        .await
                        .expect_err("changed encoding")
                        .to_string()
                        .contains("encoding changed")
                );
            })
            .await;
        }
    }

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
                // A trailing EOF closes the connection after any partial reply.
                let (response, close) = response
                    .strip_suffix("EOF")
                    .map_or((response, false), |partial| (partial, true));
                server
                    .get_mut()
                    .write_all(response.replace("$TAG", tag).as_bytes())
                    .await
                    .expect("response");
                if close {
                    return;
                }
            }
            line.clear();
            let bytes = tokio::time::timeout(Duration::from_secs(1), server.read_line(&mut line))
                .await
                .expect("client closes after its operation")
                .expect("read final input");
            assert_eq!(bytes, 0, "Unexpected command after the script: {line}");
        };
        let client = async {
            let mut client = async_imap::Client::new(client);
            client.read_response().await.expect("greeting");
            let session = client.login("fixture", "fixture").await.expect("login");
            let mut folders = ImapFolders {
                session,
                encoding: NameEncoding::ImapUtf7,
                extensions: Extensions::default(),
            };
            action(&mut folders).await;
        };
        tokio::join!(client, peer);
    }

    #[tokio::test]
    async fn planned_create_returns_wire_outcome_without_a_post_write_list() {
        for (response, expected) in [
            ("$TAG OK created\r\n", 0),
            ("$TAG NO denied\r\n", 1),
            ("EOF", 2),
        ] {
            script(vec![("CREATE \"Archive\"", response)], async |folders| {
                let target = Mailbox {
                    encoding: NameEncoding::ImapUtf7,
                    ..Mailbox::flat("Archive".into())
                };
                let outcome = folders.create_planned_folder(&target).await;
                assert!(matches!(
                    (expected, outcome),
                    (0, CreateOutcome::Acknowledged)
                        | (1, CreateOutcome::Rejected(_))
                        | (2, CreateOutcome::Uncertain(_))
                ));
            })
            .await;
        }
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
                    "LIST \"\" \"\"",
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
                    "LIST \"\" \"\"",
                    "* LIST (\\Noselect) \".\" \"\"\r\n$TAG OK namespace\r\n",
                ),
                ("CREATE \"Archive\"", "$TAG NO already exists\r\n"),
                (
                    "LIST \"\" \"Archive\"",
                    "* LIST () \".\" \"Archive\"\r\n$TAG OK complete\r\n",
                ),
            ],
            async |folders| {
                let adapter = connection(
                    &mut folders.session,
                    folders.encoding,
                    Extensions::default(),
                );
                assert_eq!(
                    creation::ensure(&adapter, "Archive", None)
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
    async fn stalwart_empty_reference_listing_creates_archive_with_its_special_use() {
        script(
            vec![
                (
                    "CAPABILITY",
                    "* CAPABILITY IMAP4rev1 NAMESPACE SPECIAL-USE CREATE-SPECIAL-USE LIST-EXTENDED MOVE UIDPLUS\r\n$TAG OK done\r\n",
                ),
                ("LIST \"\" \"Archive\"", "$TAG OK absent\r\n"),
                (
                    "LIST \"\" \"\"",
                    "* LIST (\\NoSelect) \"/\" \"\"\r\n$TAG OK namespace\r\n",
                ),
                (
                    "CREATE \"Archive\" (USE (\\Archive))",
                    "$TAG OK created\r\n",
                ),
                (
                    "LIST \"\" \"Archive\"",
                    "* LIST (\\Archive) \"/\" \"Archive\"\r\n$TAG OK complete\r\n",
                ),
            ],
            async |folders| {
                let created = ensure_exact(&mut folders.session, "Archive")
                    .await
                    .expect("created");
                assert_eq!(created.name, "Archive");
                assert_eq!(created.role, Some(FolderRole::Archive));
                assert!(created.selectable);
            },
        )
        .await;
    }

    #[tokio::test]
    async fn special_use_creation_needs_the_capability_and_a_logical_name() {
        for (capabilities, path, create) in [
            (
                "* CAPABILITY IMAP4rev1 SPECIAL-USE\r\n$TAG OK done\r\n",
                "Trash",
                "CREATE \"Trash\"",
            ),
            (
                "* CAPABILITY IMAP4rev1 SPECIAL-USE CREATE-SPECIAL-USE\r\n$TAG OK done\r\n",
                "Projects",
                "CREATE \"Projects\"",
            ),
            (
                "* CAPABILITY IMAP4rev1 SPECIAL-USE CREATE-SPECIAL-USE\r\n$TAG OK done\r\n",
                "Trash",
                "CREATE \"Trash\" (USE (\\Trash))",
            ),
        ] {
            let inspect = format!("LIST \"\" \"{path}\"");
            let listed = format!("* LIST () \"/\" \"{path}\"\r\n$TAG OK complete\r\n");
            script(
                vec![
                    ("CAPABILITY", capabilities),
                    (&inspect, "$TAG OK absent\r\n"),
                    (
                        "LIST \"\" \"\"",
                        "* LIST (\\Noselect) \"/\" \"\"\r\n$TAG OK namespace\r\n",
                    ),
                    (create, "$TAG OK created\r\n"),
                    (&inspect, &listed),
                ],
                async |folders| {
                    assert_eq!(
                        ensure_exact(&mut folders.session, path)
                            .await
                            .expect("created")
                            .name,
                        path
                    );
                },
            )
            .await;
        }
    }

    #[tokio::test]
    async fn missing_root_and_reference_listings_fail_before_create() {
        script(
            vec![
                ("LIST \"\" \"Archive\"", "$TAG OK absent\r\n"),
                ("LIST \"\" \"\"", "$TAG OK nothing\r\n"),
                ("LIST \"Archive\" \"\"", "$TAG OK nothing\r\n"),
            ],
            async |folders| {
                let adapter = connection(
                    &mut folders.session,
                    folders.encoding,
                    Extensions {
                        create_special_use: true,
                        namespace: false,
                    },
                );
                let error = creation::ensure(&adapter, "Archive", Some(FolderRole::Archive))
                    .await
                    .expect_err("no namespace");
                assert!(
                    error
                        .to_string()
                        .contains("did not report its folder namespace")
                );
            },
        )
        .await;
    }

    const NAMESPACE_CAPABILITY: &str =
        "* CAPABILITY IMAP4rev1 NAMESPACE MOVE UIDPLUS\r\n$TAG OK done\r\n";

    /// A server that lists nothing for both the root and the destination's
    /// reference (RFC 3501 permits either) still creates through NAMESPACE, and
    /// the session keeps working for the async-imap commands that follow.
    #[tokio::test]
    async fn empty_root_and_reference_listings_fall_back_to_namespace() {
        script(
            vec![
                ("CAPABILITY", NAMESPACE_CAPABILITY),
                ("LIST \"\" \"Archive\"", "$TAG OK absent\r\n"),
                ("LIST \"\" \"\"", "$TAG OK nothing\r\n"),
                ("LIST \"Archive\" \"\"", "$TAG OK nothing\r\n"),
                (
                    "NAMESPACE",
                    "* NAMESPACE ((\"\" \"/\")) NIL ((\"Shared/\" \"/\"))\r\n$TAG OK done\r\n",
                ),
                ("CREATE \"Archive\"", "$TAG OK created\r\n"),
                (
                    "LIST \"\" \"Archive\"",
                    "* LIST () \"/\" \"Archive\"\r\n$TAG OK complete\r\n",
                ),
            ],
            async |folders| {
                let created = ensure_exact(&mut folders.session, "Archive")
                    .await
                    .expect("created");
                assert_eq!(created.name, "Archive");
                assert!(created.selectable);
            },
        )
        .await;
    }

    #[tokio::test]
    async fn namespace_prefix_plans_new_folders_inside_the_personal_namespace() {
        let namespace = "* NAMESPACE ((\"INBOX.\" \".\")) NIL NIL\r\n$TAG OK done\r\n";
        script(
            vec![
                ("LIST \"\" \"\"", "$TAG OK nothing\r\n"),
                ("NAMESPACE", namespace),
                ("LIST \"\" \"INBOX.Projects\"", "$TAG OK absent\r\n"),
                ("LIST \"\" \"\"", "$TAG OK nothing\r\n"),
                ("LIST \"INBOX.Projects\" \"\"", "$TAG OK nothing\r\n"),
                ("NAMESPACE", namespace),
                ("CREATE \"INBOX.Projects\"", "$TAG OK created\r\n"),
                (
                    "LIST \"\" \"INBOX.Projects\"",
                    "* LIST () \".\" \"INBOX.Projects\"\r\n$TAG OK complete\r\n",
                ),
            ],
            async |folders| {
                folders.extensions.namespace = true;
                assert_eq!(
                    folders
                        .create_folder(None, "Projects")
                        .await
                        .expect("created")
                        .name,
                    "INBOX.Projects"
                );
            },
        )
        .await;
    }

    #[tokio::test]
    async fn namespace_refusal_or_partial_reply_is_an_error_and_never_creates() {
        for reply in [
            "$TAG NO unavailable\r\n",
            "* NAMESPACE ((\"\" \"/\")) NIL NIL\r\n$TAG BAD later\r\n",
            "* NAMESPACE ((\"\" \"/\")) NIL NIL\r\nEOF",
        ] {
            script(
                vec![
                    ("CAPABILITY", NAMESPACE_CAPABILITY),
                    ("LIST \"\" \"Archive\"", "$TAG OK absent\r\n"),
                    ("LIST \"\" \"\"", "$TAG OK nothing\r\n"),
                    ("LIST \"Archive\" \"\"", "$TAG OK nothing\r\n"),
                    ("NAMESPACE", reply),
                ],
                async |folders| {
                    let error = ensure_exact(&mut folders.session, "Archive")
                        .await
                        .expect_err("unconfirmed namespace");
                    assert!(
                        error.downcast_ref::<creation::PlanRejected>().is_none(),
                        "{error:#}"
                    );
                },
            )
            .await;
        }
    }

    #[tokio::test]
    async fn namespace_without_a_personal_entry_reports_no_namespace() {
        script(
            vec![
                ("CAPABILITY", NAMESPACE_CAPABILITY),
                ("LIST \"\" \"Archive\"", "$TAG OK absent\r\n"),
                ("LIST \"\" \"\"", "$TAG OK nothing\r\n"),
                ("LIST \"Archive\" \"\"", "$TAG OK nothing\r\n"),
                (
                    "NAMESPACE",
                    "* NAMESPACE NIL NIL ((\"Public/\" \"/\"))\r\n$TAG OK done\r\n",
                ),
            ],
            async |folders| {
                let error = ensure_exact(&mut folders.session, "Archive")
                    .await
                    .expect_err("no personal namespace");
                assert!(
                    error.downcast_ref::<creation::PlanRejected>().is_some(),
                    "{error:#}"
                );
            },
        )
        .await;
    }

    #[tokio::test]
    async fn trash_resolves_to_the_special_use_folder_and_issues_no_create() {
        script(
            vec![
                (
                    "LIST \"\" \"*\"",
                    "* LIST (\\Trash) \"/\" \"Deleted Items\"\r\n* LIST (\\Drafts) \"/\" \"Drafts\"\r\n* LIST () \"/\" \"INBOX\"\r\n* LIST (\\Junk) \"/\" \"Junk Mail\"\r\n* LIST (\\Sent) \"/\" \"Sent Items\"\r\n$TAG OK complete\r\n",
                ),
                (
                    "CAPABILITY",
                    "* CAPABILITY IMAP4rev1 SPECIAL-USE CREATE-SPECIAL-USE\r\n$TAG OK done\r\n",
                ),
                (
                    "LIST \"\" \"Deleted Items\"",
                    "* LIST (\\Trash) \"/\" \"Deleted Items\"\r\n$TAG OK present\r\n",
                ),
            ],
            async |folders| {
                let catalog = folders.catalog().await.expect("catalog");
                assert_eq!(
                    catalog
                        .iter()
                        .map(|mailbox| (mailbox.name.as_str(), mailbox.role))
                        .collect::<Vec<_>>(),
                    [
                        ("Deleted Items", Some(FolderRole::Trash)),
                        ("Drafts", Some(FolderRole::Drafts)),
                        ("INBOX", None),
                        ("Junk Mail", Some(FolderRole::Junk)),
                        ("Sent Items", Some(FolderRole::Sent)),
                    ]
                );
                let destination = crate::folders::resolve_destination(&catalog, "Trash");
                assert_eq!(destination, "Deleted Items");
                assert_eq!(
                    crate::folders::resolve_destination(&catalog, "Junk"),
                    "Junk Mail"
                );
                assert_eq!(
                    crate::folders::resolve_destination(&catalog, "Archive"),
                    "Archive"
                );
                let target = ensure_exact(&mut folders.session, &destination)
                    .await
                    .expect("existing");
                assert_eq!(target.name, "Deleted Items");
                assert_eq!(target.role, Some(FolderRole::Trash));
            },
        )
        .await;
    }

    #[tokio::test]
    async fn unicode_child_preserves_wire_parent_and_server_delimiter() {
        // The reference listing is the fallback when the root lists nothing.
        script(
            vec![
                ("LIST \"\" \"\"", "$TAG OK nothing\r\n"),
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
                ("LIST \"\" \"\"", "$TAG OK nothing\r\n"),
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
                let adapter = connection(
                    &mut folders.session,
                    folders.encoding,
                    Extensions::default(),
                );
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
                    let adapter = connection(
                        &mut folders.session,
                        folders.encoding,
                        Extensions::default(),
                    );
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
                    "LIST \"\" \"\"",
                    "* LIST (\\Noselect) \"/\" \"\"\r\n$TAG OK namespace\r\n",
                ),
                ("CREATE \"Archive\"", "EOF"),
            ],
            async |folders| {
                let adapter = connection(
                    &mut folders.session,
                    folders.encoding,
                    Extensions::default(),
                );
                assert!(creation::ensure(&adapter, "Archive", None).await.is_err());
            },
        )
        .await;
    }
}
