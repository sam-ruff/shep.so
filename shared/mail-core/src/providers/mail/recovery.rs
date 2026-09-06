//! Identify a moved copy before Undo; never guess from subject or sender.
use super::*;
use crate::mail_actions::{Fingerprint, MoveReceipt};
use async_imap::imap_proto::{AttributeValue, MailboxDatum, RequestId, Response, Status};
use std::collections::BTreeSet;

pub async fn resolve(
    account: &Account,
    secret: &SecretString,
    receipt: &MoveReceipt,
) -> anyhow::Result<StoredMail> {
    anyhow::ensure!(
        account.protocol == Protocol::Imap && account.id == receipt.account,
        "The move belongs to a different account."
    );
    let fingerprint = receipt
        .fingerprint
        .as_ref()
        .context("This move has no server identity proof. Refresh the folder.")?;
    let mut session = imap(account, secret).await?;
    let result = resolve_session(
        &mut session,
        &account.id,
        &receipt.folder,
        fingerprint,
        receipt.current.as_ref().map(|mail| mail.remote_id.as_str()),
    )
    .await;
    let _ = session.logout().await;
    result
}

fn completed(reply: &Response<'_>, tag: &RequestId) -> anyhow::Result<bool> {
    match reply {
        Response::Done {
            tag: actual,
            status,
            ..
        } => {
            anyhow::ensure!(
                actual == tag && *status == Status::Ok,
                "The server could not complete the message lookup. Refresh and retry Undo."
            );
            Ok(true)
        }
        Response::Data {
            status: Status::Bye,
            ..
        } => anyhow::bail!("The server disconnected during message lookup. Retry Undo."),
        _ => Ok(false),
    }
}

async fn candidates<T>(
    session: &mut async_imap::Session<T>,
    fingerprint: &Fingerprint,
) -> anyhow::Result<BTreeSet<u32>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let mut search = if fingerprint.bytes == 0 {
        "SMALLER 1".to_owned()
    } else {
        format!(
            "LARGER {} SMALLER {}",
            fingerprint.bytes - 1,
            fingerprint.bytes.saturating_add(1)
        )
    };
    if let Some(id) = &fingerprint.message_id {
        search.push_str(&format!(
            " HEADER Message-ID {}",
            super::receipts::quoted(id)?
        ));
    }
    let tag = session.run_command(format!("UID SEARCH {search}")).await?;
    let mut uids = BTreeSet::new();
    loop {
        let response = session
            .read_response()
            .await?
            .context("The server disconnected during message lookup. Retry Undo.")?;
        if let Response::MailboxData(MailboxDatum::Search(found)) = response.parsed() {
            anyhow::ensure!(
                !found.contains(&0),
                "The server returned an invalid message identity."
            );
            uids.extend(found);
        }
        if completed(response.parsed(), &tag)? {
            return Ok(uids);
        }
    }
}

async fn fetch<T>(
    session: &mut async_imap::Session<T>,
    uid: u32,
    fingerprint: &Fingerprint,
) -> anyhow::Result<Option<(Vec<u8>, bool, bool)>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let tag = session
        .run_command(format!(
            "UID FETCH {uid} (UID FLAGS RFC822.SIZE BODY.PEEK[])"
        ))
        .await?;
    let mut body = None;
    let mut flags = None;
    let mut size = None;
    let mut saw_body = false;
    let mut seen = false;
    loop {
        let response = session
            .read_response()
            .await?
            .context("The server disconnected during message lookup. Retry Undo.")?;
        if let Response::Fetch(_, attributes) = response.parsed()
            && attributes
                .iter()
                .any(|a| matches!(a, AttributeValue::Uid(found) if *found == uid))
        {
            seen = true;
            for attribute in attributes {
                match attribute {
                    AttributeValue::BodySection {
                        section: None,
                        index: None,
                        data: Some(data),
                    } => {
                        anyhow::ensure!(
                            !saw_body,
                            "The server returned conflicting message bodies. Refresh and retry Undo."
                        );
                        saw_body = true;
                        if fingerprint.matches(data) {
                            body = Some(data.to_vec());
                        }
                    }
                    AttributeValue::Flags(values) => {
                        anyhow::ensure!(
                            !values.iter().any(|v| v.eq_ignore_ascii_case("\\Deleted")),
                            "This message is being deleted elsewhere. Refresh before moving it."
                        );
                        flags = Some((
                            !values.iter().any(|v| v.eq_ignore_ascii_case("\\Seen")),
                            values.iter().any(|v| v.eq_ignore_ascii_case("\\Flagged")),
                        ));
                    }
                    AttributeValue::Rfc822Size(bytes) => size = Some(u64::from(*bytes)),
                    _ => {}
                }
            }
        }
        if completed(response.parsed(), &tag)? {
            break;
        }
    }
    anyhow::ensure!(
        seen && saw_body && flags.is_some() && size.is_some(),
        "This message moved or was removed elsewhere. Refresh the folder before undoing."
    );
    let (unread, starred) = flags.unwrap();
    Ok(body
        .filter(|_| size == Some(fingerprint.bytes))
        .map(|body| (body, unread, starred)))
}

pub(super) async fn resolve_session<T>(
    session: &mut async_imap::Session<T>,
    account: &str,
    folder: &str,
    fingerprint: &Fingerprint,
    known: Option<&str>,
) -> anyhow::Result<StoredMail>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let validity = session
        .examine(folder)
        .await?
        .uid_validity
        .filter(|id| *id != 0)
        .context("The folder has no stable identity. Refresh before undoing.")?;
    let known = known
        .and_then(|remote| remote.split_once('.'))
        .and_then(|(old, uid)| Some((old.parse::<u32>().ok()?, uid.parse::<u32>().ok()?)))
        .filter(|(old, uid)| *old == validity && *uid != 0)
        .map(|(_, uid)| uid);
    let uids = match known {
        Some(uid) => BTreeSet::from([uid]),
        None => candidates(session, fingerprint).await?,
    };
    let mut found = None;
    for uid in uids {
        if let Some((raw, unread, starred)) = fetch(session, uid, fingerprint).await? {
            anyhow::ensure!(
                found.is_none(),
                "Several identical copies match this move. Open the destination folder and choose the copy to move back."
            );
            found = Some((uid, raw, unread, starred));
        }
    }
    let (uid, raw, unread, starred) = found.context("The moved message could not be found unchanged in its destination. Refresh before undoing.")?;
    let (account, folder) = (account.to_owned(), folder.to_owned());
    tokio::task::spawn_blocking(move || {
        parse_mail(
            &account,
            &format!("{validity}.{uid}"),
            &folder,
            raw,
            unread,
            starred,
        )
    })
    .await?
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn line(socket: &mut BufReader<tokio::io::DuplexStream>) -> (String, String) {
        let mut line = String::new();
        assert!(socket.read_line(&mut line).await.unwrap() > 0);
        let (tag, command) = line.trim_end().split_once(' ').unwrap();
        (tag.into(), command.into())
    }
    const RAW: &[u8] = b"Message-ID: <move@example.test>\r\nSubject: Keep this\r\n\r\nExact body";
    async fn server(
        socket: tokio::io::DuplexStream,
        known: bool,
        bodies: Vec<&'static [u8]>,
        reject_fetch: bool,
    ) {
        let mut socket = BufReader::new(socket);
        let (tag, _) = line(&mut socket).await;
        socket
            .get_mut()
            .write_all(format!("{tag} OK login\r\n").as_bytes())
            .await
            .unwrap();
        let (tag, command) = line(&mut socket).await;
        assert_eq!(command, "EXAMINE \"Archive\"");
        socket
            .get_mut()
            .write_all(format!("* OK [UIDVALIDITY 91] valid\r\n{tag} OK selected\r\n").as_bytes())
            .await
            .unwrap();
        if !known {
            let (tag, command) = line(&mut socket).await;
            assert_eq!(
                command,
                format!(
                    "UID SEARCH LARGER {} SMALLER {} HEADER Message-ID \"<move@example.test>\"",
                    RAW.len() - 1,
                    RAW.len() + 1
                )
            );
            let uids = (1..=bodies.len())
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            socket
                .get_mut()
                .write_all(format!("* SEARCH {uids}\r\n{tag} OK found\r\n").as_bytes())
                .await
                .unwrap();
        }
        for (index, body) in bodies.into_iter().enumerate() {
            let uid = index + 1;
            let (tag, command) = line(&mut socket).await;
            assert_eq!(
                command,
                format!("UID FETCH {uid} (UID FLAGS RFC822.SIZE BODY.PEEK[])")
            );
            let prefix = format!(
                "* {uid} FETCH (UID {uid} FLAGS (\\Seen \\Flagged) RFC822.SIZE {} BODY[] {{{}}}\r\n",
                body.len(),
                body.len()
            );
            socket.get_mut().write_all(prefix.as_bytes()).await.unwrap();
            socket.get_mut().write_all(body).await.unwrap();
            socket
                .get_mut()
                .write_all(
                    format!(
                        ")\r\n{tag} {} done\r\n",
                        if reject_fetch { "NO" } else { "OK" }
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
    }
    #[tokio::test]
    async fn changed_uidvalidity_uses_exact_lookup_instead_of_the_old_uid() {
        let (client, socket) = tokio::io::duplex(8192);
        let task = tokio::spawn(server(socket, false, vec![RAW], false));
        let mut session = async_imap::Client::new(client)
            .login("fixture", "secret")
            .await
            .unwrap();
        let result = resolve_session(
            &mut session,
            "work",
            "Archive",
            &Fingerprint::of(RAW),
            Some("90.17"),
        )
        .await
        .unwrap();
        assert_eq!(result.summary.remote_id, "91.1");
        assert_eq!(result.raw, RAW);
        task.await.unwrap();
    }
    #[tokio::test]
    async fn recovery_checks_raw_bytes_and_tagged_status_before_choosing_a_copy() {
        for (known, bodies, rejected, success) in [
            (true, vec![RAW], false, true),
            (false, vec![b"Different".as_slice(), RAW], false, true),
            (false, vec![RAW, RAW], false, false),
            (true, vec![b"Different".as_slice()], false, false),
            (true, vec![RAW], true, false),
            (false, vec![], false, false),
        ] {
            let (client, socket) = tokio::io::duplex(8192);
            let task = tokio::spawn(server(socket, known, bodies, rejected));
            let mut session = async_imap::Client::new(client)
                .login("fixture", "secret")
                .await
                .unwrap();
            let result = resolve_session(
                &mut session,
                "work",
                "Archive",
                &Fingerprint::of(RAW),
                known.then_some("91.1"),
            )
            .await;
            assert_eq!(result.is_ok(), success, "{result:?}");
            if let Ok(message) = result {
                assert_eq!(message.raw, RAW);
                assert!(!message.summary.unread);
                assert!(message.summary.starred);
                assert_eq!(message.summary.folder, "Archive");
                assert!(message.summary.remote_id.starts_with("91."));
            }
            task.await.unwrap();
        }
    }
}
