use super::*;
use mailparse::MailHeaderMap;

#[tokio::test]
async fn smtp_wire_keeps_bcc_in_envelope_and_distinguishes_rejection_from_lost_ack() {
    for outcome in ["accepted", "rejected", "lost-ack"] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = BufReader::new(socket);
            socket
                .get_mut()
                .write_all(b"220 local.test ESMTP\r\n")
                .await
                .unwrap();
            let mut recipients = Vec::new();
            let mut data = Vec::new();
            loop {
                let mut command = String::new();
                assert!(socket.read_line(&mut command).await.unwrap() > 0);
                let response = if command.starts_with("EHLO ") {
                    "250-local.test\r\n250 8BITMIME\r\n"
                } else if command.starts_with("MAIL FROM:") {
                    "250 Sender accepted\r\n"
                } else if command.starts_with("RCPT TO:") {
                    recipients.push(command.trim().to_owned());
                    if outcome == "rejected" {
                        socket
                            .get_mut()
                            .write_all(b"550 Recipient rejected\r\n")
                            .await
                            .unwrap();
                        return (recipients, data);
                    }
                    "250 Recipient accepted\r\n"
                } else if command == "DATA\r\n" {
                    socket
                        .get_mut()
                        .write_all(b"354 Send message\r\n")
                        .await
                        .unwrap();
                    loop {
                        let mut line = Vec::new();
                        assert!(socket.read_until(b'\n', &mut line).await.unwrap() > 0);
                        if line == b".\r\n" {
                            break;
                        }
                        if line.starts_with(b"..") {
                            line.remove(0);
                        }
                        data.extend(line);
                    }
                    if outcome == "accepted" {
                        socket.get_mut().write_all(b"250 Queued\r\n").await.unwrap();
                    }
                    return (recipients, data);
                } else {
                    panic!("Unexpected SMTP command: {command}");
                };
                socket
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
            }
        });
        let account: Account = serde_json::from_value(serde_json::json!({"id":"a", "name":"Work", "email":"sender@example.com", "protocol":"Imap", "host":"imap.example.com", "port":993, "username":"sender", "smtp_host":"smtp.example.com", "smtp_port":465})).unwrap();
        let bytes = vec![0, 255, 1, 13, 10];
        let attachment = DraftAttachment {
            content_id: None,
            id: "file".into(),
            name: "notes.bin".into(),
            media_type: "application/octet-stream".into(),
            size: bytes.len(),
        };
        let draft = Draft {
            to: "friend@example.com".into(),
            cc: "copy@example.com".into(),
            bcc: "hidden@example.com".into(),
            subject: "Wire contract".into(),
            body: "First line\n.Second line".into(),
            attachments: vec![attachment.clone()],
            ..Default::default()
        };
        let message = crate::compose::build(
            &account,
            &draft,
            vec![crate::compose::FilePart {
                attachment,
                bytes: bytes.clone(),
            }],
        )
        .unwrap();
        // Only this object-scoped loopback transport uses plaintext. Production
        // always constructs its TLS/STARTTLS transport through smtp_transport.
        let transport =
            lettre::AsyncSmtpTransport::<lettre::Tokio1Executor>::builder_dangerous("127.0.0.1")
                .port(port)
                .timeout(Some(Duration::from_secs(5)))
                .build();
        let result = deliver(transport, message).await;
        let (recipients, raw) = tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        match outcome {
            "accepted" => {
                let sent = result.unwrap();
                // Lettre terminates DATA with a CRLF before the dot line.
                assert_eq!([sent.as_slice(), b"\r\n"].concat(), raw);
                assert_eq!(
                    recipients,
                    [
                        "RCPT TO:<friend@example.com>",
                        "RCPT TO:<copy@example.com>",
                        "RCPT TO:<hidden@example.com>"
                    ]
                );
                let parsed = mailparse::parse_mail(&raw).unwrap();
                assert!(parsed.headers.get_first_value("Bcc").is_none());
                assert_eq!(parsed.subparts[1].get_body_raw().unwrap(), bytes);
                assert!(
                    parsed.subparts[0]
                        .get_body()
                        .unwrap()
                        .contains(".Second line")
                );
            }
            "rejected" => assert!(result.unwrap_err().to_string().contains("rejected")),
            _ => assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("could not confirm delivery")
            ),
        }
    }
}
