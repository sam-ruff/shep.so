//! Hosted cross-account moves: check the source, upload the cached original to
//! the other account, then remove exactly the original UID in a later request.
//! Callers persist the upload receipt before asking for source cleanup.
use super::*;
use crate::mail_actions::{MoveFailure, MoveRefused, classify_move_failure};

/// Why an upload did not return a destination receipt.
#[derive(Debug)]
pub enum TransferFailure {
    /// Proven not to have stored the message; the source is untouched.
    NotApplied(anyhow::Error),
    /// The upload may have been stored. Never repeat it automatically.
    Uncertain(anyhow::Error),
}
impl std::fmt::Display for TransferFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotApplied(error) | Self::Uncertain(error) => write!(f, "{error:#}"),
        }
    }
}
impl std::error::Error for TransferFailure {}

/// Confirm the cached identity still names this UID and that the source can
/// later remove exactly that UID. Nothing is changed on the server.
pub async fn check_source<T>(
    session: &mut async_imap::Session<T>,
    mail: &Mail,
) -> anyhow::Result<()>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let uid = validate_uid(mail, session.examine(&mail.folder).await?.uid_validity)?;
    anyhow::ensure!(
        uid.parse::<u32>()? != 0,
        "The source message has an invalid UID. Refresh its folder."
    );
    if !session.capabilities().await?.has_str("UIDPLUS") {
        return Err(MoveRefused(
            "The source server needs UIDPLUS to move this message safely.".into(),
        )
        .into());
    }
    Ok(())
}

/// Store the exact original in the destination folder. A missing special
/// folder may be created first; the message itself is uploaded only once.
pub async fn upload<T>(
    session: &mut async_imap::Session<T>,
    mail: &Mail,
    folder: &str,
    raw: &[u8],
) -> Result<Option<String>, TransferFailure>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    folders::ensure_exact(session, folder)
        .await
        .map_err(TransferFailure::NotApplied)?;
    receipts::append_message(session, mail, folder, raw)
        .await
        .map_err(|error| match classify_move_failure(&error) {
            MoveFailure::Refused => TransferFailure::NotApplied(error),
            MoveFailure::Uncertain => TransferFailure::Uncertain(error),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// Scripted IMAP server answering with `capabilities` and the tagged APPEND
    /// reply `append` (empty closes the connection after the literal).
    async fn session(
        capabilities: &'static str,
        append: &'static str,
    ) -> (
        async_imap::Session<tokio::io::DuplexStream>,
        tokio::task::JoinHandle<Vec<String>>,
    ) {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            let mut seen = Vec::new();
            loop {
                let mut line = String::new();
                if server.read_line(&mut line).await.unwrap_or(0) == 0 {
                    break;
                }
                let Some((tag, command)) = line.trim_end().split_once(' ') else {
                    continue;
                };
                seen.push(command.to_owned());
                let reply = if command == "CAPABILITY" {
                    format!("* CAPABILITY IMAP4rev1 {capabilities}\r\n{tag} OK done\r\n")
                } else if command.starts_with("EXAMINE ") {
                    format!("* OK [UIDVALIDITY 42] ok\r\n{tag} OK [READ-ONLY] done\r\n")
                } else if command.starts_with("LIST ") {
                    format!("* LIST () \"/\" \"Plans\"\r\n{tag} OK listed\r\n")
                } else if let Some(size) = command
                    .strip_prefix("APPEND ")
                    .and_then(|rest| rest.rsplit_once('{'))
                    .and_then(|(_, size)| size.trim_end_matches('}').parse::<usize>().ok())
                {
                    if append.starts_with("NO") {
                        format!("{tag} {append}\r\n")
                    } else {
                        server.get_mut().write_all(b"+ go\r\n").await.unwrap();
                        let mut literal = vec![0; size + 2];
                        tokio::io::AsyncReadExt::read_exact(&mut server, &mut literal)
                            .await
                            .unwrap();
                        seen.push(String::from_utf8_lossy(&literal).into_owned());
                        if append.is_empty() {
                            break;
                        }
                        format!("{tag} {append}\r\n")
                    }
                } else if command == "LOGOUT" {
                    format!("* BYE\r\n{tag} OK bye\r\n")
                } else {
                    format!("{tag} OK done\r\n")
                };
                server.get_mut().write_all(reply.as_bytes()).await.unwrap();
            }
            seen
        });
        let session = async_imap::Client::new(client)
            .login("fixture", "secret")
            .await
            .map_err(|(error, _)| error)
            .expect("fixture login");
        (session, task)
    }

    fn mail() -> Mail {
        crate::model::parse_mail(
            "work",
            "42.7",
            "INBOX",
            b"Subject: Plans\r\n\r\nBody".to_vec(),
            true,
            false,
        )
        .map(|stored| stored.summary)
        .expect("fixture mail")
    }

    #[tokio::test]
    async fn source_check_requires_the_cached_uidvalidity_and_uidplus() -> anyhow::Result<()> {
        let (mut ready, _) = session("UIDPLUS", "OK").await;
        check_source(&mut ready, &mail()).await?;
        let (mut missing, _) = session("MOVE", "OK").await;
        let error = check_source(&mut missing, &mail())
            .await
            .expect_err("UIDPLUS is required");
        assert_eq!(classify_move_failure(&error), MoveFailure::Refused);
        let mut changed = mail();
        changed.remote_id = "41.7".into();
        let (mut stale, _) = session("UIDPLUS", "OK").await;
        assert!(check_source(&mut stale, &changed).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn upload_returns_the_appenduid_and_classifies_rejection_and_loss() {
        let raw = b"Subject: Plans\r\n\r\nBody";
        let (mut accepted, server) = session("UIDPLUS", "OK [APPENDUID 9 3] stored").await;
        let receipt = upload(&mut accepted, &mail(), "Plans", raw).await;
        assert_eq!(receipt.ok().flatten().as_deref(), Some("9.3"));
        drop(accepted);
        let seen = server.await.unwrap_or_default();
        assert!(seen.iter().any(|line| line.starts_with("APPEND \"Plans\"")));
        assert!(seen.iter().any(|line| line.starts_with("Subject: Plans")));

        let (mut rejected, _) = session("UIDPLUS", "NO [OVERQUOTA] full").await;
        assert!(matches!(
            upload(&mut rejected, &mail(), "Plans", raw).await,
            Err(TransferFailure::NotApplied(_))
        ));

        // The server received every byte but the connection closed before a reply.
        let (mut lost, _) = session("UIDPLUS", "").await;
        assert!(matches!(
            upload(&mut lost, &mail(), "Plans", raw).await,
            Err(TransferFailure::Uncertain(_))
        ));
    }
}
