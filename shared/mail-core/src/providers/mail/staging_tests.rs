use super::*;

#[tokio::test]
async fn imap_receives_small_mail_before_staging_large_source_and_protected_profiles_fail_explicitly()
-> anyhow::Result<()> {
    for mode in [ReceiveMode::Staged, ReceiveMode::Protected] {
        let raw = b"From: sender@example.test\r\nSubject: Large source\r\nContent-Type: multipart/mixed; boundary=part\r\n\r\n--part\r\nContent-Type: text/plain\r\n\r\nReadable body.\r\n--part\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=big.bin\r\n\r\n";
        let size = raw.len() + 26 * 1024 * 1024 + b"\r\n--part--\r\n".len();
        let (client, server) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            let small = b"Subject: Small message\r\n\r\nSmall body.";
            for stage in 0..7 {
                let mut line = String::new();
                server.read_line(&mut line).await?;
                let (tag, command) = line.trim_end().split_once(' ').context("command")?;
                let data = match stage {
                    0 => String::new(),
                    1 => "* CAPABILITY IMAP4rev1\r\n".into(),
                    2 => "* LIST () \"/\" \"INBOX\"\r\n".into(),
                    3 => "* 2 EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n".into(),
                    4 => "* SEARCH 7 8\r\n".into(),
                    5 => format!(
                        "* 1 FETCH (UID 8 FLAGS () RFC822.SIZE {size})\r\n* 2 FETCH (UID 7 FLAGS () RFC822.SIZE {})\r\n",
                        small.len()
                    ),
                    _ => {
                        assert_eq!(command, "UID FETCH 7 (UID FLAGS BODY.PEEK[])");
                        format!(
                            "* 2 FETCH (UID 7 FLAGS () BODY[] {{{}}}\r\n{})\r\n",
                            small.len(),
                            String::from_utf8_lossy(small)
                        )
                    }
                };
                server
                    .get_mut()
                    .write_all(format!("{data}{tag} OK done\r\n").as_bytes())
                    .await?;
            }
            if mode == ReceiveMode::Protected {
                let mut line = String::new();
                assert_eq!(server.read_line(&mut line).await?, 0);
                return Ok::<_, anyhow::Error>(());
            }
            let mut line = String::new();
            server.read_line(&mut line).await?;
            let (tag, command) = line.trim_end().split_once(' ').context("select")?;
            assert_eq!(command, "SELECT \"INBOX\"");
            server
                .get_mut()
                .write_all(
                    format!("* 2 EXISTS\r\n* OK [UIDVALIDITY 42] valid\r\n{tag} OK selected\r\n")
                        .as_bytes(),
                )
                .await?;
            let mut offset = 0;
            while offset < size {
                line.clear();
                server.read_line(&mut line).await?;
                let (tag, command) = line.trim_end().split_once(' ').context("fetch")?;
                let count = (streaming::CHUNK_BYTES as usize).min(size - offset);
                assert_eq!(
                    command,
                    format!("UID FETCH 8 (UID BODY.PEEK[]<{offset}.{count}>)")
                );
                let mut bytes = vec![b'x'; count];
                for (i, byte) in bytes.iter_mut().enumerate() {
                    let position = offset + i;
                    if position < raw.len() {
                        *byte = raw[position];
                    }
                    if position >= size - 12 {
                        *byte = b"\r\n--part--\r\n"[position - (size - 12)];
                    }
                }
                server
                    .get_mut()
                    .write_all(
                        format!("* 1 FETCH (UID 8 BODY[]<{offset}> {{{count}}}\r\n").as_bytes(),
                    )
                    .await?;
                server.get_mut().write_all(&bytes).await?;
                server
                    .get_mut()
                    .write_all(format!(")\r\n{tag} OK fetched\r\n").as_bytes())
                    .await?;
                offset += count;
            }
            line.clear();
            server.read_line(&mut line).await?;
            let (tag, command) = line.trim_end().split_once(' ').context("logout")?;
            assert_eq!(command, "LOGOUT");
            server
                .get_mut()
                .write_all(format!("* BYE goodbye\r\n{tag} OK logout\r\n").as_bytes())
                .await?;
            Ok(())
        });
        let account: Account = serde_json::from_value(
            serde_json::json!({"id":"fixture","name":"Fixture","email":"fixture@example.test","protocol":"Imap","host":"localhost","port":993,"username":"fixture","smtp_host":"localhost","smtp_port":465}),
        )?;
        let session = async_imap::Client::new(client)
            .login("fixture", "secret")
            .await
            .map_err(|(error, _)| error)?;
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let consume = async move {
            let mut received = Vec::new();
            while let Some(item) = rx.recv().await {
                match item {
                    MailSyncItem::Message(mail) => received.push(mail.summary.subject),
                    MailSyncItem::StagedMessage(mail) => {
                        assert_eq!(mail.bytes(), size as u64);
                        assert_eq!(mail.text, "Readable body.");
                        assert_eq!(mail.summary.attachment_count, 1);
                        received.push(mail.summary.subject);
                    }
                    MailSyncItem::SkippedLarge => {
                        panic!("Native staged sync must not skip large mail")
                    }
                    _ => {}
                }
            }
            received
        };
        let known = HashSet::new();
        let (result, received) = tokio::join!(
            sync_imap_session_mode(session, &account, &known, tx, None, mode),
            consume
        );
        if mode == ReceiveMode::Protected {
            assert!(
                result
                    .expect_err("protected profile must report unsupported staging")
                    .to_string()
                    .contains("encrypted staging")
            );
            assert_eq!(received, ["Small message"]);
        } else {
            result?;
            assert_eq!(received, ["Small message", "Large source"]);
        }
        server.await??;
    }
    Ok(())
}
