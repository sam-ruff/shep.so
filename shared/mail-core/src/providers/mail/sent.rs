//! Sent mailbox discovery and exact Message-ID lookup. SMTP delivery is separate.
use super::*;
use async_imap::{
    imap_proto::{
        AttributeValue, MailboxDatum, MessageSection, RequestId, Response, SectionPath, Status,
    },
    types::{Name, NameAttribute},
};
use async_trait::async_trait;
use mailparse::MailHeaderMap;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        Self::from_session(imap(account, secret).await?, &account.sent_folder).await
    }
    pub(super) async fn from_session(
        mut session: async_imap::Session<Tls>,
        configured: &str,
    ) -> anyhow::Result<Self> {
        let folder = discover(&mut session, configured).await?;
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

/// Shared object-scoped connection for desktop, device and gateway transports.
#[async_trait]
pub trait SentConnection: Send {
    fn folder(&self) -> &str;
    async fn find(&mut self, id: &str) -> anyhow::Result<Option<SentReceipt>>;
    async fn append(&mut self, raw: &[u8], timestamp: i64) -> anyhow::Result<SentReceipt>;
}
#[async_trait]
impl SentConnection for SentMailbox {
    fn folder(&self) -> &str {
        &self.folder
    }
    async fn find(&mut self, id: &str) -> anyhow::Result<Option<SentReceipt>> {
        self.find(id).await
    }
    async fn append(&mut self, raw: &[u8], timestamp: i64) -> anyhow::Result<SentReceipt> {
        self.append(raw, timestamp).await
    }
}

pub fn same_incoming(current: &Account, saved: &Account) -> bool {
    current.id == saved.id
        && current.protocol == saved.protocol
        && current.host == saved.host
        && current.port == saved.port
        && current.username == saved.username
        && current.email == saved.email
        && current.incoming_security == saved.incoming_security
        && current.incoming_auth == saved.incoming_auth
}

fn choose(names: &[(String, bool, bool)], configured: &str) -> anyhow::Result<String> {
    let selectable = names.iter().filter(|(_, selectable, _)| *selectable);
    if !configured.is_empty() {
        return selectable.clone().find(|(name, _, _)| name == configured).map(|(name, _, _)|name.clone())
            .context("The configured Sent folder is unavailable. Choose an existing folder in account settings.");
    }
    let sent: Vec<_> = selectable.clone().filter(|(_, _, sent)| *sent).collect();
    anyhow::ensure!(
        sent.len() <= 1,
        "The server has several Sent folders. Choose one in account settings."
    );
    if let Some(name) = sent.first() {
        return Ok(name.0.clone());
    }
    let common: Vec<_> = selectable
        .filter(|(name, _, _)| {
            ["Sent", "Sent Items", "Sent Messages"]
                .iter()
                .any(|s| name.eq_ignore_ascii_case(s))
        })
        .collect();
    anyhow::ensure!(
        common.len() == 1,
        "Choose your server's Sent folder in account settings, or select local Sent copies."
    );
    Ok(common[0].0.clone())
}
pub(super) fn choose_folder(names: &[Name], configured: &str) -> anyhow::Result<String> {
    choose(
        &names
            .iter()
            .map(|n| {
                (
                    n.name().to_owned(),
                    !n.attributes().contains(&NameAttribute::NoSelect),
                    n.attributes().contains(&NameAttribute::Sent),
                )
            })
            .collect::<Vec<_>>(),
        configured,
    )
}
fn complete(response: &Response<'_>, tag: &RequestId) -> anyhow::Result<bool> {
    match response {
        Response::Done {
            tag: actual,
            status,
            ..
        } => {
            anyhow::ensure!(
                actual == tag && *status == Status::Ok,
                "The server did not complete the Sent lookup. Keep the local copy and retry checking Sent."
            );
            Ok(true)
        }
        Response::Data {
            status: Status::Bye,
            ..
        } => anyhow::bail!(
            "The server disconnected while checking Sent. Keep the local copy and retry."
        ),
        _ => Ok(false),
    }
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
        "\"*\""
    };
    let tag = session.run_command(format!("LIST \"\" {pattern}")).await?;
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for _ in 0..8192 {
        let response = session
            .read_response()
            .await?
            .context("The server disconnected while listing Sent folders.")?;
        if let Response::MailboxData(MailboxDatum::List {
            name_attributes,
            name,
            ..
        }) = response.parsed()
        {
            anyhow::ensure!(
                names.len() < 4096
                    && name.len() <= 1024
                    && !name.contains(['\r', '\n', '\0'])
                    && seen.insert(name.to_string()),
                "The server returned incomplete, duplicate or oversized folder information. Retry checking Sent."
            );
            names.push((
                name.to_string(),
                !name_attributes.contains(&NameAttribute::NoSelect),
                name_attributes.contains(&NameAttribute::Sent),
            ));
        }
        if complete(response.parsed(), &tag)? {
            return choose(&names, configured);
        }
    }
    anyhow::bail!("The server returned too many responses while listing Sent folders.")
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
        .filter(|v| *v != 0)
        .context("The Sent folder has no valid UIDVALIDITY.")?;
    let tag = session
        .run_command(format!("UID SEARCH HEADER Message-ID \"{message_id}\""))
        .await?;
    let mut uids = BTreeSet::new();
    let mut finished = false;
    let mut saw_search = false;
    for _ in 0..8192 {
        let response = session
            .read_response()
            .await?
            .context("The server disconnected during Sent lookup.")?;
        if let Response::MailboxData(MailboxDatum::Search(ids)) = response.parsed() {
            anyhow::ensure!(
                !saw_search && ids.len() <= 32 && !ids.contains(&0),
                "The Sent lookup returned invalid or incomplete message identities."
            );
            saw_search = true;
            uids.extend(ids);
            anyhow::ensure!(
                uids.len() == ids.len(),
                "The Sent lookup returned duplicate message identities."
            );
        }
        if complete(response.parsed(), &tag)? {
            finished = true;
            break;
        }
    }
    anyhow::ensure!(
        finished && saw_search,
        "The Sent search did not return a complete result. Retry checking Sent."
    );
    if uids.is_empty() {
        return Ok(None);
    }
    let ids = uids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let tag = session
        .run_command(format!(
            "UID FETCH {ids} (UID BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])"
        ))
        .await?;
    let mut seen = BTreeSet::new();
    let mut found = None;
    for _ in 0..8192 {
        let response = session
            .read_response()
            .await?
            .context("The server disconnected during Sent lookup.")?;
        if let Response::Fetch(_, attributes) = response.parsed() {
            let mut uid = None;
            let mut header = None;
            for value in attributes {
                match value {
                    AttributeValue::Uid(id) => {
                        anyhow::ensure!(
                            uid.replace(*id).is_none(),
                            "The Sent lookup returned conflicting message identities."
                        );
                    }
                    AttributeValue::BodySection {
                        section: Some(SectionPath::Full(MessageSection::Header)),
                        index: None,
                        data: Some(data),
                    } => {
                        anyhow::ensure!(
                            header.replace(data.as_ref()).is_none() && data.len() <= 64 * 1024,
                            "The Sent lookup returned conflicting or oversized headers."
                        );
                    }
                    _ => {}
                }
            }
            // Unsolicited flag updates do not contribute lookup evidence.
            if let Some(header) = header {
                let uid = uid.context("The Sent lookup omitted a message UID.")?;
                anyhow::ensure!(
                    uids.contains(&uid) && seen.insert(uid),
                    "The Sent lookup returned an unexpected or duplicate UID."
                );
                let (headers, _) = mailparse::parse_headers(header)?;
                let values = headers.get_all_values("Message-ID");
                anyhow::ensure!(
                    values.len() <= 1,
                    "The Sent lookup returned conflicting Message-ID headers. Check the provider folder before uploading another copy."
                );
                if values.len() == 1 && values[0].trim() == message_id {
                    found = Some(SentReceipt {
                        folder: folder.into(),
                        remote_id: Some(format!("{validity}.{uid}")),
                    });
                }
            }
        }
        if complete(response.parsed(), &tag)? {
            anyhow::ensure!(
                seen == uids,
                "The Sent lookup omitted messages. Refresh and check again before saving another copy."
            );
            return Ok(found);
        }
    }
    anyhow::bail!("The server returned too many responses during Sent lookup.")
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
    #[tokio::test]
    async fn sent_recovery_refuses_rejected_incomplete_or_conflicting_lookup_evidence() {
        for fault in [
            "list_no",
            "search_no",
            "search_missing",
            "fetch_no",
            "fetch_missing",
            "fetch_duplicate",
            "uid_zero",
            "header_duplicate",
            "disconnect",
        ] {
            let (client, server) = tokio::io::duplex(8192);
            let server = tokio::spawn(async move {
                let mut server = BufReader::new(server);
                login(&mut server).await;
                if fault == "list_no" {
                    let (tag, _) = command(&mut server).await;
                    reply(&mut server, &tag, "* CAPABILITY IMAP4rev1 SPECIAL-USE\r\n").await;
                    let (tag, _) = command(&mut server).await;
                    server
                        .get_mut()
                        .write_all(
                            format!("* LIST (\\Sent) \"/\" \"Sent\"\r\n{tag} NO denied\r\n")
                                .as_bytes(),
                        )
                        .await
                        .unwrap();
                    return;
                }
                let (tag, _) = command(&mut server).await;
                reply(
                    &mut server,
                    &tag,
                    if fault == "uid_zero" {
                        "* OK [UIDVALIDITY 0] invalid\r\n"
                    } else {
                        "* OK [UIDVALIDITY 42] valid\r\n"
                    },
                )
                .await;
                if fault == "uid_zero" {
                    return;
                }
                let (tag, _) = command(&mut server).await;
                if fault == "search_no" {
                    server
                        .get_mut()
                        .write_all(format!("* SEARCH\r\n{tag} NO denied\r\n").as_bytes())
                        .await
                        .unwrap();
                    return;
                }
                if fault == "search_missing" {
                    reply(&mut server, &tag, "").await;
                    return;
                }
                reply(&mut server, &tag, "* SEARCH 7\r\n").await;
                let (tag, _) = command(&mut server).await;
                if fault == "fetch_missing" {
                    reply(&mut server, &tag, "").await;
                    return;
                }
                let header = if fault == "header_duplicate" {
                    "Message-ID: <needle@shep.local>\r\nMessage-ID: <other@shep.local>\r\n\r\n"
                } else {
                    "Message-ID: <needle@shep.local>\r\n\r\n"
                };
                let fetched = format!(
                    "* 1 FETCH (UID 7 BODY[HEADER.FIELDS (MESSAGE-ID)] {{{}}}\r\n{header})\r\n",
                    header.len()
                );
                server
                    .get_mut()
                    .write_all(fetched.as_bytes())
                    .await
                    .unwrap();
                if fault == "fetch_duplicate" {
                    server
                        .get_mut()
                        .write_all(fetched.as_bytes())
                        .await
                        .unwrap();
                }
                if fault == "disconnect" {
                    return;
                }
                server
                    .get_mut()
                    .write_all(
                        format!(
                            "{tag} {} done\r\n",
                            if fault == "fetch_no" { "NO" } else { "OK" }
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            });
            let mut session = async_imap::Client::new(client)
                .login("alex", "fixture")
                .await
                .unwrap();
            if fault == "list_no" {
                assert!(discover(&mut session, "").await.is_err());
            } else {
                let result = lookup(&mut session, "Sent", "<needle@shep.local>").await;
                assert!(result.is_err(), "{fault} must not establish lookup success");
            }
            server.await.unwrap();
        }
    }
}
