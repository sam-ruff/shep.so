//! Download exact IMAP source into caller-owned staging without accumulating it.
use anyhow::{Context, Result, ensure};
use async_imap::imap_proto::{AttributeValue, RequestId, Response, Status};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

pub const CHUNK_BYTES: u32 = 256 * 1024;

pub async fn pop_body(
    source: &mut (impl tokio::io::AsyncBufRead + Unpin),
    expected: u64,
    destination: &mut (impl AsyncWrite + Unpin),
    progress: Option<&tokio::sync::mpsc::Sender<crate::model::MailSyncItem>>,
) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};
    let mut total = 0;
    let mut line_start = true;
    loop {
        let mut bytes = Vec::with_capacity(8192);
        let mut limited = (&mut *source).take(8192);
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            limited.read_until(b'\n', &mut bytes),
        )
        .await
        .context("POP3 download timed out.")??;
        ensure!(
            !bytes.is_empty(),
            "The POP3 server disconnected during download."
        );
        if line_start && (bytes == b".\r\n" || bytes == b".\n") {
            ensure!(
                total == expected,
                "The POP3 message size changed during download."
            );
            destination.flush().await?;
            return Ok(());
        }
        let skip = usize::from(line_start && bytes.starts_with(b".."));
        line_start = bytes.last() == Some(&b'\n');
        total += (bytes.len() - skip) as u64;
        ensure!(
            total <= expected,
            "The POP3 server returned more message data than advertised."
        );
        destination.write_all(&bytes[skip..]).await?;
        if total % u64::from(CHUNK_BYTES) < 8192 {
            heartbeat(progress).await?;
        }
    }
}

/// The caller must discard staging on error and publish it only after success.
pub(super) async fn download_reporting<T>(
    session: &mut async_imap::Session<T>,
    uid: u32,
    size: u32,
    destination: &mut (impl AsyncWrite + Unpin),
    progress: Option<&tokio::sync::mpsc::Sender<crate::model::MailSyncItem>>,
) -> Result<()>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    ensure!(uid != 0 && size != 0, "Invalid message identity or size.");
    let mut offset = 0;
    while offset < size {
        let count = CHUNK_BYTES.min(size - offset);
        let bytes = tokio::time::timeout(
            std::time::Duration::from_secs(480),
            chunk(session, uid, offset, count),
        )
        .await
        .context("The message download stopped responding. Refresh and retry.")??;
        destination.write_all(&bytes).await?;
        offset += count;
        heartbeat(progress).await?;
    }
    destination.flush().await?;
    Ok(())
}

async fn heartbeat(
    progress: Option<&tokio::sync::mpsc::Sender<crate::model::MailSyncItem>>,
) -> Result<()> {
    #[cfg(not(feature = "staged-receive"))]
    let _ = progress;
    #[cfg(feature = "staged-receive")]
    if let Some(progress) = progress {
        progress
            .send(crate::model::MailSyncItem::DownloadProgress)
            .await
            .context("Sync was cancelled")?;
    }
    Ok(())
}

async fn chunk<T>(
    session: &mut async_imap::Session<T>,
    uid: u32,
    offset: u32,
    count: u32,
) -> Result<Vec<u8>>
where
    T: AsyncRead + AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let tag = session
        .run_command(format!(
            "UID FETCH {uid} (UID BODY.PEEK[]<{offset}.{count}>)"
        ))
        .await?;
    let mut bytes = None;
    loop {
        let response = session
            .read_response()
            .await?
            .context("The mail server disconnected during the download.")?;
        if let Response::Fetch(_, attributes) = response.parsed() {
            let mut fetched_uid = None;
            let mut body = None;
            for attribute in attributes {
                match attribute {
                    AttributeValue::Uid(value) => {
                        ensure!(fetched_uid.is_none(), "Conflicting download identities.");
                        fetched_uid = Some(*value);
                    }
                    AttributeValue::BodySection {
                        section,
                        index,
                        data,
                    } => {
                        ensure!(
                            section.is_none() && *index == Some(offset),
                            "The server returned a different message section."
                        );
                        ensure!(body.is_none(), "The server repeated a download section.");
                        let data = data.as_ref().context("The download section is missing.")?;
                        ensure!(
                            data.len() == count as usize,
                            "The server returned an incomplete or oversized download section."
                        );
                        body = Some(data.as_ref());
                    }
                    _ => {}
                }
            }
            if let Some(body) = body {
                ensure!(
                    fetched_uid == Some(uid),
                    "The downloaded message identity changed."
                );
                ensure!(bytes.is_none(), "The server repeated a download section.");
                bytes = Some(body.to_vec());
            }
        }
        if complete(response.parsed(), &tag)? {
            return bytes.context("The message disappeared during download. Refresh and retry.");
        }
    }
}

fn complete(response: &Response<'_>, tag: &RequestId) -> Result<bool> {
    match response {
        Response::Done {
            tag: actual,
            status,
            ..
        } => {
            ensure!(
                actual == tag && *status == Status::Ok,
                "The mail server rejected the download. Refresh and retry."
            );
            Ok(true)
        }
        Response::Data {
            status: Status::Bye,
            ..
        } => {
            anyhow::bail!("The mail server disconnected during the download.")
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[tokio::test]
    async fn pop_stream_preserves_long_binary_lines_dot_stuffing_and_checks_completeness()
    -> Result<()> {
        let (client, mut server) = tokio::io::duplex(8192);
        let expected = 26 * 1024 * 1024 + 5;
        let sender = tokio::spawn(async move {
            let block = [0xff; 8192];
            for _ in 0..26 * 1024 * 1024 / block.len() {
                server.write_all(&block).await?;
            }
            server.write_all(b"\r\n..\r\n.\r\n").await?;
            Ok::<_, anyhow::Error>(())
        });
        let mut source = BufReader::new(client);
        let mut sink = tokio::io::sink();
        pop_body(&mut source, expected as u64, &mut sink, None).await?;
        sender.await??;
        for bytes in [b"body\r\n.\r\n".as_slice(), b"body\r\n".as_slice()] {
            let mut source = BufReader::new(bytes);
            assert!(pop_body(&mut source, 7, &mut sink, None).await.is_err());
        }
        Ok(())
    }

    #[tokio::test]
    async fn downloads_more_than_25_mib_without_retaining_the_complete_message() -> Result<()> {
        use sha2::{Digest, Sha256};
        let size = 26 * 1024 * 1024 + 13;
        let (client, server) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            let mut line = String::new();
            server.read_line(&mut line).await?;
            let (tag, _) = line.trim_end().split_once(' ').context("login")?;
            server
                .get_mut()
                .write_all(format!("{tag} OK logged in\r\n").as_bytes())
                .await?;
            let mut offset = 0;
            let mut digest = Sha256::new();
            while offset < size {
                line.clear();
                server.read_line(&mut line).await?;
                let (tag, command) = line.trim_end().split_once(' ').context("fetch")?;
                let count = CHUNK_BYTES.min(size - offset);
                assert_eq!(
                    command,
                    format!("UID FETCH 7 (UID BODY.PEEK[]<{offset}.{count}>)")
                );
                let bytes = vec![(offset / CHUNK_BYTES) as u8; count as usize];
                digest.update(&bytes);
                server
                    .get_mut()
                    .write_all(
                        format!("* 1 FETCH (UID 7 BODY[]<{offset}> {{{count}}}\r\n").as_bytes(),
                    )
                    .await?;
                server.get_mut().write_all(&bytes).await?;
                server
                    .get_mut()
                    .write_all(format!(")\r\n{tag} OK fetched\r\n").as_bytes())
                    .await?;
                offset += count;
            }
            Ok::<_, anyhow::Error>(digest.finalize())
        });
        let mut session = async_imap::Client::new(client)
            .login("fixture", "secret")
            .await
            .map_err(|(error, _)| error)?;
        let (mut sink, source) = tokio::io::duplex(8192);
        let reader = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut source = source;
            let mut buffer = [0; 8192];
            let mut digest = Sha256::new();
            let mut total = 0;
            loop {
                let read = source.read(&mut buffer).await?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
                total += read;
            }
            Ok::<_, anyhow::Error>((total, digest.finalize()))
        });
        download_reporting(&mut session, 7, size, &mut sink, None).await?;
        drop(sink);
        let (total, digest) = reader.await??;
        assert_eq!(total, size as usize);
        assert_eq!(digest, server.await??);
        Ok(())
    }

    #[tokio::test]
    async fn rejects_wrong_identity_offset_size_and_final_status_before_writing() -> Result<()> {
        for reply in [
            "* 1 FETCH (UID 8 BODY[]<0> {4}\r\nbody)\r\n{tag} OK done\r\n",
            "* 1 FETCH (UID 7 BODY[]<1> {4}\r\nbody)\r\n{tag} OK done\r\n",
            "* 1 FETCH (UID 7 BODY[]<0> {3}\r\nbod)\r\n{tag} OK done\r\n",
            "* 1 FETCH (UID 7 BODY[]<0> {5}\r\nbodys)\r\n{tag} OK done\r\n",
            "* 1 FETCH (UID 7 BODY[]<0> {4}\r\nbody)\r\n{tag} NO failed\r\n",
            "* 1 FETCH (UID 7 BODY[]<0> {4}\r\nbody)\r\n",
            "{tag} OK done\r\n",
        ] {
            let (client, server) = tokio::io::duplex(8192);
            let server = tokio::spawn(async move {
                let mut server = BufReader::new(server);
                let mut line = String::new();
                server.read_line(&mut line).await?;
                let (tag, _) = line.trim_end().split_once(' ').context("login")?;
                server
                    .get_mut()
                    .write_all(format!("{tag} OK logged in\r\n").as_bytes())
                    .await?;
                line.clear();
                server.read_line(&mut line).await?;
                let (tag, _) = line.trim_end().split_once(' ').context("fetch")?;
                server
                    .get_mut()
                    .write_all(reply.replace("{tag}", tag).as_bytes())
                    .await?;
                Ok::<_, anyhow::Error>(())
            });
            let mut session = async_imap::Client::new(client)
                .login("fixture", "secret")
                .await
                .map_err(|(error, _)| error)?;
            let mut sink = Vec::new();
            assert!(
                download_reporting(&mut session, 7, 4, &mut sink, None)
                    .await
                    .is_err()
            );
            assert!(sink.is_empty());
            server.await??;
        }
        Ok(())
    }
}
