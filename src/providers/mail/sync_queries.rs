//! Sync must observe tagged OK. async-imap's collection helpers stop at the
//! matching tag without checking its status, accepting NO/BAD as empty data.
use super::*;
use async_imap::imap_proto::{AttributeValue, MailboxDatum, RequestId, Response, Status};

fn completed(reply: &Response<'_>, tag: &RequestId) -> anyhow::Result<bool> {
    match reply {
        Response::Done {
            tag: actual,
            status,
            ..
        } => {
            anyhow::ensure!(
                actual == tag && *status == Status::Ok,
                "The mail server rejected the sync request. Try Refresh again."
            );
            Ok(true)
        }
        Response::Data {
            status: Status::Bye,
            ..
        } => anyhow::bail!("The mail server disconnected during sync. Try Refresh again."),
        _ => Ok(false),
    }
}

pub(super) async fn search<T>(session: &mut async_imap::Session<T>) -> anyhow::Result<HashSet<u32>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let tag = session.run_command("UID SEARCH ALL").await?;
    let mut uids = HashSet::new();
    loop {
        let response = session
            .read_response()
            .await?
            .context("The mail server disconnected before completing sync. Try Refresh again.")?;
        if let Response::MailboxData(MailboxDatum::Search(found)) = response.parsed() {
            anyhow::ensure!(
                !found.contains(&0),
                "The mail server returned an invalid message identity."
            );
            uids.extend(found);
        }
        if completed(response.parsed(), &tag)? {
            return Ok(uids);
        }
    }
}

pub(super) struct Fetch {
    pub uid: Option<u32>,
    pub size: Option<u32>,
    pub unread: bool,
    pub starred: bool,
    pub body: Option<Vec<u8>>,
}
pub(super) async fn fetch<T>(
    session: &mut async_imap::Session<T>,
    set: &str,
    attributes: &str,
) -> anyhow::Result<Vec<Fetch>>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let tag = session
        .run_command(format!("UID FETCH {set} {attributes}"))
        .await?;
    let mut results = Vec::new();
    loop {
        let response = session
            .read_response()
            .await?
            .context("The mail server disconnected before completing sync. Try Refresh again.")?;
        if let Response::Fetch(_, attributes) = response.parsed() {
            let mut fetch = Fetch {
                uid: None,
                size: None,
                unread: true,
                starred: false,
                body: None,
            };
            for attribute in attributes {
                match attribute {
                    AttributeValue::Uid(uid) => {
                        anyhow::ensure!(*uid != 0, "Invalid message identity during sync.");
                        fetch.uid = Some(*uid);
                    }
                    AttributeValue::Rfc822Size(size) => fetch.size = Some(*size),
                    AttributeValue::Flags(flags) => {
                        fetch.unread =
                            !flags.iter().any(|flag| flag.eq_ignore_ascii_case("\\Seen"));
                        fetch.starred = flags
                            .iter()
                            .any(|flag| flag.eq_ignore_ascii_case("\\Flagged"));
                    }
                    AttributeValue::BodySection {
                        section: None,
                        index: None,
                        data: Some(data),
                    } => {
                        anyhow::ensure!(
                            data.len() <= MAX_MESSAGE_BYTES,
                            "The server returned a message exceeding 25 MiB."
                        );
                        anyhow::ensure!(
                            fetch.body.is_none(),
                            "The mail server returned conflicting message bodies."
                        );
                        fetch.body = Some(data.to_vec());
                    }
                    _ => {}
                }
            }
            results.push(fetch);
        }
        if completed(response.parsed(), &tag)? {
            return Ok(results);
        }
    }
}
