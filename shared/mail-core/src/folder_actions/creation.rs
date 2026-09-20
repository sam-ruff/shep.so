//! Non-destructive creation with a checked observation after every attempt.
use crate::folders::{FolderRole, Mailbox, NameEncoding};
use anyhow::Context;
use base64::Engine as _;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreateOutcome {
    Acknowledged,
    Rejected(String),
    Uncertain(String),
}

/// The server answered the CREATE with a definite refusal, so no folder exists
/// and repeating the same request cannot succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreationRejected(pub String);
impl std::fmt::Display for CreationRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CreationRejected {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRejected(pub String);
impl std::fmt::Display for PlanRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PlanRejected {}

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub trait Connection: Send + Sync {
    async fn inspect(&self, path: String) -> anyhow::Result<Option<Mailbox>>;
    /// The hierarchy root and delimiter for `reference` (`LIST "<reference>" ""`),
    /// or `None` when the server lists nothing for it.
    async fn namespace(&self, reference: String) -> anyhow::Result<Option<Mailbox>>;
    /// The personal namespace prefix from NAMESPACE, when the server offers one.
    async fn namespaces(&self) -> anyhow::Result<Option<Mailbox>>;
    async fn create(&self, path: String, role: Option<FolderRole>) -> CreateOutcome;
}

/// RFC 3501 defines `LIST "" ""` as the request for the root and delimiter;
/// some servers (Stalwart) list nothing for a reference that does not exist yet.
pub async fn discover_namespace(
    connection: &impl Connection,
    reference: &str,
) -> anyhow::Result<Mailbox> {
    if let Some(root) = connection.namespace(String::new()).await? {
        return Ok(root);
    }
    if !reference.is_empty()
        && let Some(root) = connection.namespace(reference.into()).await?
    {
        return Ok(root);
    }
    if let Some(root) = connection.namespaces().await? {
        return Ok(root);
    }
    Err(PlanRejected("The server did not report its folder namespace.".into()).into())
}

pub fn valid_path(path: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!path.trim().is_empty(), "Enter a folder name.");
    anyhow::ensure!(path.len() <= 1024, "The folder name is too long.");
    anyhow::ensure!(
        !path.chars().any(char::is_control),
        "Folder names cannot contain control characters."
    );
    Ok(())
}

fn encode(name: &str, encoding: NameEncoding) -> String {
    if encoding == NameEncoding::Utf8 {
        return name.to_owned();
    }
    let mut result = String::new();
    let mut shifted = Vec::new();
    let flush = |result: &mut String, shifted: &mut Vec<u8>| {
        if !shifted.is_empty() {
            result.push('&');
            result.push_str(
                &base64::engine::general_purpose::STANDARD_NO_PAD
                    .encode(&*shifted)
                    .replace('/', ","),
            );
            result.push('-');
            shifted.clear();
        }
    };
    for character in name.chars() {
        if character.is_ascii() {
            flush(&mut result, &mut shifted);
            result.push(character);
            if character == '&' {
                result.push('-');
            }
        } else {
            for unit in character.encode_utf16(&mut [0; 2]) {
                shifted.extend_from_slice(&unit.to_be_bytes());
            }
        }
    }
    flush(&mut result, &mut shifted);
    result
}

/// `root` is the server's LIST reference/empty-pattern namespace response.
pub fn plan(root: &Mailbox, parent: Option<&Mailbox>, name: &str) -> anyhow::Result<Mailbox> {
    valid_path(name)?;
    let base = parent.unwrap_or(root);
    if let Some(parent) = parent {
        anyhow::ensure!(
            !parent.non_existent,
            "Refresh the missing parent folder before creating a child."
        );
        anyhow::ensure!(
            !parent.no_inferiors,
            "This folder cannot contain child folders."
        );
        anyhow::ensure!(
            parent.delimiter.is_some(),
            "This account uses a flat folder hierarchy."
        );
        anyhow::ensure!(
            parent.encoding == root.encoding,
            "The parent folder encoding changed. Refresh folders."
        );
    }
    if let Some(delimiter) = base.delimiter {
        anyhow::ensure!(
            !name.contains(delimiter),
            "Enter one folder name, without the hierarchy separator."
        );
    }
    let mut path = if parent.is_some() {
        base.path().to_owned()
    } else {
        base.name.clone()
    };
    if let Some(delimiter) = base.delimiter
        && !path.is_empty()
        && (parent.is_some()
            || last_separator(&path, base.encoding, delimiter)
                .is_none_or(|index| index + delimiter.len_utf8() != path.len()))
    {
        path.push(delimiter);
    }
    path.push_str(&encode(name, base.encoding));
    valid_path(&path)?;
    Ok(Mailbox {
        name: path,
        delimiter: base.delimiter,
        encoding: base.encoding,
        selectable: true,
        no_inferiors: false,
        non_existent: false,
        role: None,
    })
}

fn selectable(mailbox: Mailbox) -> anyhow::Result<Mailbox> {
    anyhow::ensure!(
        mailbox.selectable && !mailbox.non_existent,
        "The destination exists but cannot receive mail."
    );
    Ok(mailbox)
}

pub async fn exists(connection: &impl Connection, path: &str) -> anyhow::Result<bool> {
    valid_path(path)?;
    match connection.inspect(path.into()).await? {
        Some(mailbox) => {
            selectable(mailbox)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn last_separator(path: &str, encoding: NameEncoding, delimiter: char) -> Option<usize> {
    let mut shifted = false;
    let mut result = None;
    for (index, character) in path.char_indices() {
        if encoding == NameEncoding::ImapUtf7 {
            if shifted {
                if character == '-' {
                    shifted = false;
                }
                continue;
            }
            if character == '&' {
                shifted = true;
                continue;
            }
        }
        if character == delimiter {
            result = Some(index);
        }
    }
    result
}

/// `role` marks a missing destination with its special use at creation.
pub async fn ensure(
    connection: &impl Connection,
    path: &str,
    role: Option<FolderRole>,
) -> anyhow::Result<Mailbox> {
    valid_path(path)?;
    if let Some(mailbox) = connection.inspect(path.into()).await? {
        return selectable(mailbox);
    }
    let namespace = discover_namespace(connection, path).await?;
    anyhow::ensure!(
        path.starts_with(&namespace.name),
        "The destination is outside the reported namespace."
    );
    anyhow::ensure!(
        path != namespace.name,
        "The destination must name a folder inside its namespace."
    );
    if let Some(delimiter) = namespace.delimiter {
        anyhow::ensure!(
            last_separator(path, namespace.encoding, delimiter)
                .is_none_or(|index| index + delimiter.len_utf8() != path.len()),
            "The destination needs a folder name after its separator."
        );
        if let Some(index) = last_separator(path, namespace.encoding, delimiter)
            && index >= namespace.name.len()
        {
            let parent = connection
                .inspect(path[..index].into())
                .await?
                .context("The destination's parent folder does not exist.")?;
            anyhow::ensure!(
                !parent.non_existent && !parent.no_inferiors,
                "The destination's parent cannot contain folders."
            );
            anyhow::ensure!(
                parent.delimiter == namespace.delimiter && parent.encoding == namespace.encoding,
                "The destination hierarchy changed. Refresh folders."
            );
        }
    }
    let outcome = connection.create(path.into(), role).await;
    if let Some(mailbox) = connection
        .inspect(path.into())
        .await
        .context("Could not confirm folder creation. Retry the same folder after reconnecting.")?
    {
        return selectable(mailbox);
    }
    match outcome {
        CreateOutcome::Acknowledged => anyhow::bail!(
            "The server acknowledged creation but did not list a selectable destination. Refresh folders before retrying."
        ),
        CreateOutcome::Rejected(message) => Err(CreationRejected(message).into()),
        CreateOutcome::Uncertain(message) => anyhow::bail!(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::predicate::eq;

    fn root() -> Mailbox {
        Mailbox {
            name: String::new(),
            delimiter: Some('/'),
            encoding: NameEncoding::ImapUtf7,
            selectable: false,
            no_inferiors: false,
            non_existent: false,
            role: None,
        }
    }
    fn target() -> Mailbox {
        Mailbox {
            name: "Archive".into(),
            selectable: true,
            ..root()
        }
    }
    fn expect_root(connection: &mut MockConnection, sequence: &mut mockall::Sequence) {
        connection
            .expect_namespace()
            .with(eq(String::new()))
            .times(1)
            .in_sequence(sequence)
            .returning(|_| Ok(Some(root())));
    }

    #[test]
    fn plans_unicode_ampersands_namespace_and_flat_names() {
        let unicode = plan(&root(), None, "日本語 & work").expect("valid leaf");
        assert_eq!(unicode.name, "&ZeVnLIqe- &- work");
        assert_eq!(unicode.encoding.display(&unicode.name), "日本語 & work");
        let namespace = Mailbox {
            name: "INBOX.".into(),
            delimiter: Some('.'),
            ..root()
        };
        assert_eq!(
            plan(&namespace, None, "Archive").expect("root").name,
            "INBOX.Archive"
        );
        let flat = Mailbox {
            delimiter: None,
            encoding: NameEncoding::Utf8,
            ..root()
        };
        assert_eq!(
            plan(&flat, None, "Plans/2026").expect("flat literal").name,
            "Plans/2026"
        );
        let parent = Mailbox {
            name: "&ZeVnLIqe-".into(),
            delimiter: Some('-'),
            ..target()
        };
        assert_eq!(
            plan(&root(), Some(&parent), "Reports")
                .expect("UTF7 parent")
                .name,
            "&ZeVnLIqe--Reports"
        );
    }

    #[test]
    fn rejects_invalid_leaf_and_unusable_parent() {
        for name in ["", "  ", "A/B", "A\nB", "A\0B"] {
            assert!(plan(&root(), None, name).is_err(), "{name:?}");
        }
        for parent in [
            Mailbox {
                no_inferiors: true,
                ..target()
            },
            Mailbox {
                non_existent: true,
                ..target()
            },
            Mailbox {
                delimiter: None,
                ..target()
            },
        ] {
            assert!(plan(&root(), Some(&parent), "Reports").is_err());
        }
    }

    #[tokio::test]
    async fn existing_target_never_creates_or_discovers_namespace() {
        let mut connection = MockConnection::new();
        connection
            .expect_inspect()
            .with(eq("Archive".to_owned()))
            .times(1)
            .returning(|_| Ok(Some(target())));
        assert_eq!(
            ensure(&connection, "Archive", Some(FolderRole::Archive))
                .await
                .expect("existing"),
            target()
        );
    }

    #[tokio::test]
    async fn creation_and_lost_or_refused_acknowledgements_use_checked_result() {
        for outcome in [
            CreateOutcome::Acknowledged,
            CreateOutcome::Rejected("NO".into()),
            CreateOutcome::Uncertain("lost".into()),
        ] {
            let mut connection = MockConnection::new();
            let mut sequence = mockall::Sequence::new();
            connection
                .expect_inspect()
                .times(1)
                .in_sequence(&mut sequence)
                .returning(|_| Ok(None));
            expect_root(&mut connection, &mut sequence);
            connection
                .expect_create()
                .with(eq("Archive".to_owned()), eq(Some(FolderRole::Archive)))
                .times(1)
                .in_sequence(&mut sequence)
                .return_once(|_, _| outcome);
            connection
                .expect_inspect()
                .times(1)
                .in_sequence(&mut sequence)
                .returning(|_| Ok(Some(target())));
            assert_eq!(
                ensure(&connection, "Archive", Some(FolderRole::Archive))
                    .await
                    .expect("confirmed"),
                target()
            );
        }
    }

    #[tokio::test]
    async fn namespace_discovery_tries_root_then_reference_then_namespace_command() {
        // [root listing, reference listing, NAMESPACE, discovered root]
        let cases: Vec<[Option<Mailbox>; 4]> = vec![
            [Some(root()), None, None, Some(root())],
            [
                None,
                Some(Mailbox {
                    name: "INBOX.".into(),
                    delimiter: Some('.'),
                    ..root()
                }),
                None,
                Some(Mailbox {
                    name: "INBOX.".into(),
                    delimiter: Some('.'),
                    ..root()
                }),
            ],
            [
                None,
                None,
                Some(Mailbox {
                    delimiter: Some('/'),
                    ..root()
                }),
                Some(root()),
            ],
            [None, None, None, None],
        ];
        for [by_root, by_reference, by_command, expected] in cases {
            let mut connection = MockConnection::new();
            let mut sequence = mockall::Sequence::new();
            let root_listed = by_root.is_some();
            let reference_listed = by_reference.is_some();
            connection
                .expect_namespace()
                .with(eq(String::new()))
                .times(1)
                .in_sequence(&mut sequence)
                .return_once(move |_| Ok(by_root));
            if !root_listed {
                connection
                    .expect_namespace()
                    .with(eq("Archive".to_owned()))
                    .times(1)
                    .in_sequence(&mut sequence)
                    .return_once(move |_| Ok(by_reference));
            }
            if !root_listed && !reference_listed {
                connection
                    .expect_namespaces()
                    .times(1)
                    .in_sequence(&mut sequence)
                    .return_once(move || Ok(by_command));
            }
            let discovered = discover_namespace(&connection, "Archive").await;
            match expected {
                Some(expected) => assert_eq!(discovered.expect("discovered"), expected),
                None => assert!(
                    discovered
                        .expect_err("nothing reported")
                        .to_string()
                        .contains("did not report its folder namespace")
                ),
            }
        }
    }

    #[tokio::test]
    async fn missing_namespace_never_creates() {
        let mut connection = MockConnection::new();
        connection.expect_inspect().times(1).returning(|_| Ok(None));
        connection
            .expect_namespace()
            .times(2)
            .returning(|_| Ok(None));
        connection
            .expect_namespaces()
            .times(1)
            .returning(|| Ok(None));
        connection.expect_create().times(0);
        assert!(
            ensure(&connection, "Archive", Some(FolderRole::Archive))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn plain_folders_are_created_without_a_special_use() {
        let mut connection = MockConnection::new();
        let mut sequence = mockall::Sequence::new();
        connection
            .expect_inspect()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Ok(None));
        expect_root(&mut connection, &mut sequence);
        connection
            .expect_create()
            .with(eq("Projects".to_owned()), eq(None))
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _| CreateOutcome::Acknowledged);
        connection
            .expect_inspect()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| {
                Ok(Some(Mailbox {
                    name: "Projects".into(),
                    ..target()
                }))
            });
        assert_eq!(
            ensure(&connection, "Projects", None)
                .await
                .expect("created")
                .name,
            "Projects"
        );
    }

    #[tokio::test]
    async fn failed_listing_and_nonselectable_target_never_create() {
        for observation in [
            Err(anyhow::anyhow!("partial LIST then NO")),
            Ok(Some(Mailbox {
                selectable: false,
                ..target()
            })),
        ] {
            let mut connection = MockConnection::new();
            connection
                .expect_inspect()
                .times(1)
                .return_once(|_| observation);
            assert!(ensure(&connection, "Archive", None).await.is_err());
        }
    }

    #[tokio::test]
    async fn failed_post_check_never_retries_create() {
        for observation in [Ok(None), Err(anyhow::anyhow!("disconnected"))] {
            let mut connection = MockConnection::new();
            let mut sequence = mockall::Sequence::new();
            connection
                .expect_inspect()
                .times(1)
                .in_sequence(&mut sequence)
                .returning(|_| Ok(None));
            expect_root(&mut connection, &mut sequence);
            connection
                .expect_create()
                .times(1)
                .in_sequence(&mut sequence)
                .returning(|_, _| CreateOutcome::Rejected("denied".into()));
            let listing_failed = observation.is_err();
            connection
                .expect_inspect()
                .times(1)
                .in_sequence(&mut sequence)
                .return_once(|_| observation);
            let error = ensure(&connection, "Archive", None).await.unwrap_err();
            // A clean listing after a refused CREATE is a definite rejection; a
            // failed listing cannot prove what the server did.
            assert_eq!(
                error.downcast_ref::<CreationRejected>().is_some(),
                !listing_failed,
                "{error:#}"
            );
        }
    }

    #[tokio::test]
    async fn absent_or_noinferiors_parent_blocks_creation() {
        for parent in [
            None,
            Some(Mailbox {
                no_inferiors: true,
                name: "Projects".into(),
                ..target()
            }),
        ] {
            let mut connection = MockConnection::new();
            connection
                .expect_inspect()
                .with(eq("Projects/Archive".to_owned()))
                .times(1)
                .returning(|_| Ok(None));
            connection
                .expect_namespace()
                .with(eq(String::new()))
                .times(1)
                .returning(|_| Ok(Some(root())));
            connection
                .expect_inspect()
                .with(eq("Projects".to_owned()))
                .times(1)
                .return_once(|_| Ok(parent));
            assert!(ensure(&connection, "Projects/Archive", None).await.is_err());
        }
    }

    #[tokio::test]
    async fn exists_distinguishes_absence_from_listing_failure() {
        let mut connection = MockConnection::new();
        connection.expect_inspect().times(1).returning(|_| Ok(None));
        assert!(
            !exists(&connection, "Archive")
                .await
                .expect("checked absence")
        );
        let mut connection = MockConnection::new();
        connection
            .expect_inspect()
            .times(1)
            .returning(|_| Err(anyhow::anyhow!("LIST refused")));
        assert!(exists(&connection, "Archive").await.is_err());
    }
}
