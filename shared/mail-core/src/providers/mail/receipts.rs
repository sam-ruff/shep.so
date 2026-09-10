//! Preserve acknowledged destination identities for moving messages back safely.
use super::*;
use async_imap::imap_proto::{RequestId, Response, ResponseCode, Status, UidSetMember};

/// An APPEND tagged rejection means its atomic upload was not applied. MOVE
/// deliberately does not use this error: RFC 6851 allows partial effects on NO.
#[derive(Debug)]
pub struct UploadRejected;
impl std::fmt::Display for UploadRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("The server rejected the upload. The original is retained.")
    }
}
impl std::error::Error for UploadRejected {}

pub fn quoted(value: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        !value.is_empty() && !value.contains(['\r', '\n', '\0']),
        "Choose a valid folder."
    );
    Ok(format!(
        "\"{}\"",
        value.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn single_uid(set: &[UidSetMember]) -> Option<u32> {
    let uid = match set {
        [UidSetMember::Uid(uid)] => *uid,
        [UidSetMember::UidRange(range)] if range.start() == range.end() => *range.start(),
        _ => return None,
    };
    (uid != 0).then_some(uid)
}

#[derive(Clone, Copy)]
enum Kind {
    Move(u32),
    Append,
}

impl Kind {
    fn identity(self, code: &ResponseCode<'_>) -> Option<Option<String>> {
        let (validity, destination) = match (self, code) {
            (Self::Move(uid), ResponseCode::CopyUid(validity, source, destination)) => {
                if single_uid(source) != Some(uid) {
                    return Some(None);
                }
                (*validity, single_uid(destination))
            }
            (Self::Append, ResponseCode::AppendUid(validity, destination)) => {
                (*validity, single_uid(destination))
            }
            _ => return None,
        };
        Some(
            destination
                .filter(|_| validity != 0)
                .map(|uid| format!("{validity}.{uid}")),
        )
    }
}

async fn completion<T>(
    session: &mut async_imap::Session<T>,
    tag: &RequestId,
    kind: Kind,
) -> anyhow::Result<Option<String>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let mut receipt = None;
    loop {
        let reply = session.read_response().await?.context(
            "The server disconnected before confirming the operation. Refresh before retrying.",
        )?;
        let (status, code) = match reply.parsed() {
            Response::Done {
                tag: received,
                status,
                code,
                ..
            } => {
                anyhow::ensure!(
                    received == tag,
                    "The server returned an unexpected operation response. Refresh before retrying."
                );
                (status, code)
            }
            Response::Data { status, code, .. } => (status, code),
            _ => continue,
        };
        if *status == Status::Ok
            && let Some(mapping) = code.as_ref().and_then(|code| kind.identity(code))
        {
            // Missing/malformed/conflicting mappings do not negate a tagged OK.
            // They require exact-message recovery before Undo can issue a MOVE.
            receipt = Some(match receipt {
                None => mapping,
                Some(previous) if previous == mapping => previous,
                Some(_) => None,
            });
        }
        anyhow::ensure!(
            *status != Status::Bye,
            "The server disconnected before confirming the operation. Refresh before retrying."
        );
        if matches!(reply.parsed(), Response::Done { .. }) {
            if matches!(kind, Kind::Append) && matches!(status, Status::No | Status::Bad) {
                return Err(UploadRejected.into());
            }
            anyhow::ensure!(
                *status == Status::Ok,
                "The server did not confirm this operation. Refresh before retrying."
            );
            return Ok(receipt.flatten());
        }
    }
}

pub async fn move_message<T>(
    session: &mut async_imap::Session<T>,
    uid: &str,
    folder: &str,
) -> anyhow::Result<Option<String>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let uid: u32 = uid.parse()?;
    anyhow::ensure!(
        uid != 0,
        "Invalid message identity. Refresh before moving it."
    );
    let tag = session
        .run_command(format!("UID MOVE {uid} {}", quoted(folder)?))
        .await?;
    completion(session, &tag, Kind::Move(uid)).await
}

pub async fn append_message<T>(
    session: &mut async_imap::Session<T>,
    mail: &Mail,
    folder: &str,
    raw: &[u8],
) -> anyhow::Result<Option<String>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let flags = match (mail.unread, mail.starred) {
        (true, false) => "()",
        (false, false) => "(\\Seen)",
        (true, true) => "(\\Flagged)",
        (false, true) => "(\\Seen \\Flagged)",
    };
    let date = chrono::DateTime::from_timestamp(mail.timestamp, 0)
        .context("Invalid message date.")?
        .format("\"%d-%b-%Y %H:%M:%S %z\"")
        .to_string();
    let tag = session
        .run_command(format!(
            "APPEND {} {flags} {date} {{{}}}",
            quoted(folder)?,
            raw.len()
        ))
        .await?;
    loop {
        let reply = session.read_response().await?.context(
            "The server disconnected before accepting the message. The source is retained.",
        )?;
        match reply.parsed() {
            Response::Continue { .. } => break,
            Response::Done {
                tag: received,
                status: Status::No | Status::Bad,
                ..
            } if received == &tag => return Err(UploadRejected.into()),
            Response::Done { .. }
            | Response::Data {
                status: Status::Bye,
                ..
            } => anyhow::bail!("The server did not accept the upload. The source is retained."),
            _ => {}
        }
    }
    session.as_mut().write_all(raw).await?;
    session.as_mut().write_all(b"\r\n").await?;
    session.as_mut().flush().await?;
    completion(session, &tag, Kind::Append).await
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
    async fn login(socket: &mut BufReader<tokio::io::DuplexStream>) {
        let (tag, command) = line(socket).await;
        assert_eq!(command, "LOGIN \"fixture\" \"secret\"");
        socket
            .get_mut()
            .write_all(format!("{tag} OK logged in\r\n").as_bytes())
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn move_receipts_keep_only_the_acknowledged_single_destination_uid() {
        for (reply, expected, accepted) in [
            ("{tag} OK [COPYUID 91 7 38] done\r\n", Some("91.38"), true),
            (
                "* OK [COPYUID 91 7:7 38:38] moved\r\n* 2 EXPUNGE\r\n{tag} OK done\r\n",
                Some("91.38"),
                true,
            ),
            (
                "* OK [COPYUID 91 7 38] moved\r\n{tag} OK [COPYUID 91 7 38] done\r\n",
                Some("91.38"),
                true,
            ),
            ("{tag} OK done\r\n", None, true),
            (
                "* OK [COPYUID 91 7 38] moved\r\n{tag} OK [COPYUID 92 7 40] done\r\n",
                None,
                true,
            ),
            ("{tag} OK [COPYUID 91 8 38] done\r\n", None, true),
            ("{tag} OK [COPYUID 91 7 38:40] done\r\n", None, true),
            ("{tag} NO rejected\r\n", None, false),
            ("* OK [COPYUID 91 7 38] moved\r\n", None, false),
            ("other OK [COPYUID 91 7 38] done\r\n", None, false),
        ] {
            let (client, server) = tokio::io::duplex(4096);
            let task = tokio::spawn(async move {
                let mut socket = BufReader::new(server);
                login(&mut socket).await;
                let (tag, command) = line(&mut socket).await;
                assert_eq!(command, "UID MOVE 7 \"A. Keep \\\"folder\\\"\"");
                socket
                    .get_mut()
                    .write_all(reply.replace("{tag}", &tag).as_bytes())
                    .await
                    .unwrap();
            });
            let mut session = async_imap::Client::new(client)
                .login("fixture", "secret")
                .await
                .unwrap();
            let result = move_message(&mut session, "7", "A. Keep \"folder\"").await;
            assert_eq!(result.is_ok(), accepted, "{reply}: {result:?}");
            if let Err(error) = &result {
                assert!(
                    error.downcast_ref::<UploadRejected>().is_none(),
                    "A MOVE error must not be treated as an atomic APPEND rejection"
                );
            }
            if accepted {
                assert_eq!(result.unwrap().as_deref(), expected);
            }
            task.await.unwrap();
        }
    }
    #[tokio::test]
    async fn append_receipt_preserves_binary_content_flags_and_commit_without_logout() {
        for (reply, expected, accepted) in [
            ("{tag} OK [APPENDUID 93 39] done\r\n", Some("93.39"), true),
            ("{tag} OK done\r\n", None, true),
            ("{tag} NO rejected\r\n", None, false),
            ("", None, false),
        ] {
            let raw = b"Message-ID: <fixture@example.test>\r\n\r\nBinary \0\xff";
            let (client, server) = tokio::io::duplex(4096);
            let task = tokio::spawn(async move {
                let mut socket = BufReader::new(server);
                login(&mut socket).await;
                let (tag, command) = line(&mut socket).await;
                assert!(command.starts_with("APPEND \"Keep\" (\\Seen \\Flagged) \""));
                assert!(command.ends_with(&format!(" {{{}}}", raw.len())));
                socket
                    .get_mut()
                    .write_all(b"* 5 EXISTS\r\n+ Ready\r\n")
                    .await
                    .unwrap();
                let mut bytes = vec![0; raw.len() + 2];
                socket.read_exact(&mut bytes).await.unwrap();
                assert_eq!(bytes, [raw.as_slice(), b"\r\n"].concat());
                socket
                    .get_mut()
                    .write_all(reply.replace("{tag}", &tag).as_bytes())
                    .await
                    .unwrap();
            });
            let mut session = async_imap::Client::new(client)
                .login("fixture", "secret")
                .await
                .unwrap();
            let mail = parse_mail("work", "1.7", "INBOX", raw.to_vec(), false, true).unwrap();
            let result = append_message(&mut session, &mail.summary, "Keep", raw).await;
            assert_eq!(result.is_ok(), accepted);
            if let Err(error) = &result {
                assert_eq!(
                    error.downcast_ref::<UploadRejected>().is_some(),
                    reply.contains(" NO ")
                );
            }
            if accepted {
                assert_eq!(result.unwrap().as_deref(), expected);
            }
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn append_requires_its_own_tagged_rejection_before_treating_upload_as_not_applied() {
        for (reply, rejected) in [
            ("{tag} NO quota\r\n", true),
            ("{tag} BAD invalid\r\n", true),
            ("other NO quota\r\n", false),
            ("* BYE closed\r\n", false),
            ("", false),
        ] {
            let (client, server) = tokio::io::duplex(4096);
            let task = tokio::spawn(async move {
                let mut socket = BufReader::new(server);
                login(&mut socket).await;
                let (tag, command) = line(&mut socket).await;
                assert!(command.starts_with("APPEND "));
                socket
                    .get_mut()
                    .write_all(reply.replace("{tag}", &tag).as_bytes())
                    .await
                    .unwrap();
            });
            let mut session = async_imap::Client::new(client)
                .login("fixture", "secret")
                .await
                .unwrap();
            let mail = parse_mail(
                "work",
                "1.7",
                "INBOX",
                b"Subject: Copy\r\n\r\nBody".to_vec(),
                true,
                false,
            )
            .unwrap();
            let error = append_message(&mut session, &mail.summary, "Keep", &mail.raw)
                .await
                .unwrap_err();
            assert_eq!(
                error.downcast_ref::<UploadRejected>().is_some(),
                rejected,
                "{reply}"
            );
            task.await.unwrap();
        }
    }
}
