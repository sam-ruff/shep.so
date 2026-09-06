//! Sent mailbox discovery and exact Message-ID lookup. SMTP delivery is separate.
use super::*;
use async_imap::types::{Name, NameAttribute};
use mailparse::MailHeaderMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentReceipt {
    pub folder: String,
    pub remote_id: Option<String>,
}
pub struct SentMailbox {
    session: async_imap::Session<Tls>,
    pub folder: String,
}
impl SentMailbox {
    pub async fn open(account: &Account, secret: &SecretString) -> anyhow::Result<Self> {
        anyhow::ensure!(
            account.protocol == Protocol::Imap,
            "POP3 keeps Sent copies locally."
        );
        let mut session = imap(account, secret).await?;
        let folder = discover(&mut session, &account.sent_folder).await?;
        Ok(Self { session, folder })
    }
    pub async fn find(&mut self, message_id: &str) -> anyhow::Result<Option<SentReceipt>> {
        lookup(&mut self.session, &self.folder, message_id).await
    }
    pub async fn append(&mut self, raw: &[u8], timestamp: i64) -> anyhow::Result<SentReceipt> {
        append(&mut self.session, &self.folder, raw, timestamp).await?;
        // The tagged APPEND OK is the commit point. Later lookup/logout errors
        // must never turn it into an instruction to upload again.
        Ok(SentReceipt {
            folder: self.folder.clone(),
            remote_id: None,
        })
    }
}

pub(super) fn choose_folder(names: &[Name], configured: &str) -> anyhow::Result<String> {
    let selectable = |name: &&Name| !name.attributes().contains(&NameAttribute::NoSelect);
    if !configured.is_empty() {
        return names.iter().filter(selectable).find(|n| n.name() == configured)
            .map(|n| n.name().to_owned()).context("The configured Sent folder is unavailable. Choose an existing folder in account settings.");
    }
    let sent: Vec<_> = names
        .iter()
        .filter(selectable)
        .filter(|n| n.attributes().contains(&NameAttribute::Sent))
        .collect();
    anyhow::ensure!(
        sent.len() <= 1,
        "The server has several Sent folders. Choose one in account settings."
    );
    if let Some(name) = sent.first() {
        return Ok(name.name().to_owned());
    }
    let common: Vec<_> = names
        .iter()
        .filter(selectable)
        .filter(|n| {
            ["Sent", "Sent Items", "Sent Messages"]
                .iter()
                .any(|s| n.name().eq_ignore_ascii_case(s))
        })
        .collect();
    anyhow::ensure!(
        common.len() == 1,
        "Choose your server's Sent folder in account settings, or select local Sent copies."
    );
    Ok(common[0].name().to_owned())
}

async fn discover<T>(
    session: &mut async_imap::Session<T>,
    configured: &str,
) -> anyhow::Result<String>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    anyhow::ensure!(
        configured.len() <= 1024 && !configured.contains(['\r', '\n', '\0']),
        "Invalid Sent folder."
    );
    let capabilities = session.capabilities().await?;
    let pattern = if capabilities.has_str("SPECIAL-USE") {
        "\"*\" RETURN (SPECIAL-USE)"
    } else {
        "*"
    };
    let mut names = session.list(None, Some(pattern)).await?;
    let mut folders = Vec::new();
    while let Some(name) = names.try_next().await? {
        anyhow::ensure!(
            folders.len() < 4096 && name.name().len() <= 1024,
            "The server returned too many or oversized mailbox names."
        );
        folders.push(name);
    }
    choose_folder(&folders, configured)
}

async fn lookup<T>(
    session: &mut async_imap::Session<T>,
    folder: &str,
    message_id: &str,
) -> anyhow::Result<Option<SentReceipt>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    anyhow::ensure!(
        crate::compose::message_ids(message_id) == [message_id]
            && message_id.len() <= 998
            && !message_id.contains(['"', '\\', '\r', '\n', '\0']),
        "Invalid outgoing message identity."
    );
    let mailbox = session.examine(folder).await?;
    let validity = mailbox
        .uid_validity
        .context("The Sent folder has no UIDVALIDITY.")?;
    let uids = session
        .uid_search(format!("HEADER Message-ID \"{message_id}\""))
        .await?;
    anyhow::ensure!(
        uids.len() <= 32,
        "Too many Sent copies match this message. Inspect the folder before retrying."
    );
    if uids.is_empty() {
        return Ok(None);
    }
    let mut ordered: Vec<_> = uids.iter().copied().collect();
    ordered.sort_unstable();
    let ids = ordered
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut fetched = session
        .uid_fetch(ids, "(UID BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])")
        .await?;
    let mut found = None;
    let mut count = 0;
    while let Some(item) = fetched.try_next().await? {
        count += 1;
        anyhow::ensure!(count <= 32, "Too many Sent lookup responses.");
        let uid = item.uid.context("The Sent lookup omitted a message UID.")?;
        anyhow::ensure!(
            uids.contains(&uid),
            "The Sent lookup returned an unexpected UID."
        );
        let header = item
            .header()
            .context("The Sent lookup omitted message headers.")?;
        anyhow::ensure!(
            header.len() <= 64 * 1024,
            "The Sent lookup returned oversized headers."
        );
        let (headers, _) = mailparse::parse_headers(header)?;
        let values = headers.get_all_values("Message-ID");
        if values.len() == 1 && values[0].trim() == message_id {
            found = Some(SentReceipt {
                folder: folder.into(),
                remote_id: Some(format!("{validity}.{uid}")),
            });
        }
    }
    Ok(found)
}

async fn append<T>(
    session: &mut async_imap::Session<T>,
    folder: &str,
    raw: &[u8],
    timestamp: i64,
) -> anyhow::Result<()>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    anyhow::ensure!(
        !raw.is_empty() && raw.len() <= MAX_MESSAGE_BYTES,
        "The Sent copy exceeds the message size limit."
    );
    let date = chrono::DateTime::from_timestamp(timestamp, 0)
        .context("Invalid outgoing date.")?
        .format("\"%d-%b-%Y %H:%M:%S %z\"")
        .to_string();
    session
        .append(folder, Some("(\\Seen)"), Some(&date), raw)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn command(socket: &mut BufReader<tokio::io::DuplexStream>) -> (String, String) {
        let mut line = String::new();
        assert!(socket.read_line(&mut line).await.unwrap() > 0);
        let (tag, command) = line.trim_end().split_once(' ').unwrap();
        (tag.into(), command.into())
    }
    async fn reply(socket: &mut BufReader<tokio::io::DuplexStream>, tag: &str, body: &str) {
        socket
            .get_mut()
            .write_all(format!("{body}{tag} OK done\r\n").as_bytes())
            .await
            .unwrap();
    }
    async fn login(socket: &mut BufReader<tokio::io::DuplexStream>) {
        socket
            .get_mut()
            .write_all(b"* OK fixture\r\n")
            .await
            .unwrap();
        let (tag, cmd) = command(socket).await;
        assert_eq!(cmd, "LOGIN \"alex\" \"fixture\"");
        reply(socket, &tag, "").await;
    }
    #[tokio::test]
    async fn sent_discovery_uses_special_use_and_requires_unambiguous_selectable_folders() {
        for (listing, configured, expected) in [
            (
                "* LIST (\\Sent) \".\" \"INBOX.Sent Mail\"\r\n* LIST (\\Noselect \\Sent) \".\" \"Virtual\"\r\n",
                "",
                Some("INBOX.Sent Mail"),
            ),
            (
                "* LIST (\\Sent) \"/\" \"One\"\r\n* LIST (\\Sent) \"/\" \"Two\"\r\n",
                "",
                None,
            ),
            (
                "* LIST (\\Sent) \"/\" \"One\"\r\n* LIST (\\Sent) \"/\" \"Two\"\r\n",
                "Two",
                Some("Two"),
            ),
            ("* LIST () \"/\" \"Sent\"\r\n", "", Some("Sent")),
            ("* LIST () \"/\" \"INBOX\"\r\n", "Missing", None),
        ] {
            let (client, server) = tokio::io::duplex(4096);
            let server = tokio::spawn(async move {
                let mut server = BufReader::new(server);
                login(&mut server).await;
                let (tag, cmd) = command(&mut server).await;
                assert_eq!(cmd, "CAPABILITY");
                reply(&mut server, &tag, "* CAPABILITY IMAP4rev1 SPECIAL-USE\r\n").await;
                let (tag, cmd) = command(&mut server).await;
                assert_eq!(cmd, "LIST \"\" \"*\" RETURN (SPECIAL-USE)");
                reply(&mut server, &tag, listing).await;
            });
            let mut session = async_imap::Client::new(client)
                .login("alex", "fixture")
                .await
                .unwrap();
            let actual = discover(&mut session, configured).await;
            match expected {
                Some(folder) => assert_eq!(actual.unwrap(), folder),
                None => assert!(actual.is_err()),
            }
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn sent_lookup_checks_exact_headers_and_validity_without_fetching_bodies() {
        for exact in [true, false] {
            let (client, server) = tokio::io::duplex(4096);
            let server = tokio::spawn(async move {
                let mut server = BufReader::new(server);
                login(&mut server).await;
                let (tag, cmd) = command(&mut server).await;
                assert_eq!(cmd, "EXAMINE \"Sent Mail\"");
                reply(
                    &mut server,
                    &tag,
                    "* 1 EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n",
                )
                .await;
                let (tag, cmd) = command(&mut server).await;
                assert_eq!(cmd, "UID SEARCH HEADER Message-ID \"<needle@shep.local>\"");
                reply(&mut server, &tag, "* SEARCH 7\r\n").await;
                let (tag, cmd) = command(&mut server).await;
                assert_eq!(
                    cmd,
                    "UID FETCH 7 (UID BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])"
                );
                let header = if exact {
                    "Message-ID: <needle@shep.local>\r\n\r\n"
                } else {
                    "Message-ID: <needle@shep.local>.different\r\n\r\n"
                };
                reply(
                    &mut server,
                    &tag,
                    &format!(
                        "* 1 FETCH (UID 7 BODY[HEADER.FIELDS (MESSAGE-ID)] {{{}}}\r\n{header})\r\n",
                        header.len()
                    ),
                )
                .await;
            });
            let mut session = async_imap::Client::new(client)
                .login("alex", "fixture")
                .await
                .unwrap();
            let actual = lookup(&mut session, "Sent Mail", "<needle@shep.local>")
                .await
                .unwrap();
            assert_eq!(
                actual,
                exact.then(|| SentReceipt {
                    folder: "Sent Mail".into(),
                    remote_id: Some("42.7".into())
                })
            );
            server.await.unwrap();
        }
    }
    #[tokio::test]
    async fn sent_append_requires_tagged_commit_and_keeps_the_original_bytes() {
        for accepted in [true, false] {
            let (client, server) = tokio::io::duplex(4096);
            let raw =
                b"Message-ID: <stable@shep.local>\r\nSubject: Sent fixture\r\n\r\nBytes: \0\xff";
            let server = tokio::spawn(async move {
                let mut server = BufReader::new(server);
                login(&mut server).await;
                let (tag, cmd) = command(&mut server).await;
                assert!(cmd.starts_with("APPEND \"Sent Mail\" (\\Seen) \""));
                assert!(cmd.ends_with(&format!(" {{{}}}", raw.len())));
                server.get_mut().write_all(b"+ Ready\r\n").await.unwrap();
                let mut data = vec![0; raw.len() + 2];
                server.read_exact(&mut data).await.unwrap();
                assert_eq!(data, [raw.as_slice(), b"\r\n"].concat());
                if accepted {
                    reply(&mut server, &tag, "").await;
                }
            });
            let mut session = async_imap::Client::new(client)
                .login("alex", "fixture")
                .await
                .unwrap();
            let result = append(&mut session, "Sent Mail", raw, 1788692400).await;
            assert_eq!(result.is_ok(), accepted);
            server.await.unwrap();
        }
    }
}
