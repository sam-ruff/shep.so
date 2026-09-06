use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
const CERT: &[u8] = include_bytes!("../../../tests/fixtures/tls-cert.pem");
const KEY: &[u8] = include_bytes!("../../../tests/fixtures/tls-private.pem");

async fn fixture(
    service: &'static str,
    starttls: bool,
) -> (PinnedMail, tokio::task::JoinHandle<()>, Arc<AtomicBool>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let peer = listener.local_addr().unwrap();
    let identity = native_tls::Identity::from_pkcs8(CERT, KEY).unwrap();
    let acceptor =
        tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::new(identity).unwrap());
    let authenticated = Arc::new(AtomicBool::new(false));
    let observed = authenticated.clone();
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let tcp = if starttls {
            let mut plain = BufReader::new(tcp);
            plain
                .get_mut()
                .write_all(match service {
                    "imap" => b"* OK fixture ready\r\n",
                    "pop3" => b"+OK fixture ready\r\n",
                    _ => b"220 fixture ready\r\n",
                })
                .await
                .unwrap();
            let mut line = String::new();
            plain.read_line(&mut line).await.unwrap();
            match service {
                "imap" => {
                    let (tag, command) = line.trim_end().split_once(' ').unwrap();
                    assert_eq!(command, "STARTTLS");
                    plain
                        .get_mut()
                        .write_all(format!("{tag} OK start TLS\r\n").as_bytes())
                        .await
                        .unwrap();
                }
                "pop3" => {
                    assert_eq!(line, "STLS\r\n");
                    plain
                        .get_mut()
                        .write_all(b"+OK start TLS\r\n")
                        .await
                        .unwrap();
                }
                _ => {
                    assert!(line.starts_with("EHLO "));
                    plain
                        .get_mut()
                        .write_all(b"250-fixture\r\n250 STARTTLS\r\n")
                        .await
                        .unwrap();
                    line.clear();
                    plain.read_line(&mut line).await.unwrap();
                    assert_eq!(line, "STARTTLS\r\n");
                    plain
                        .get_mut()
                        .write_all(b"220 start TLS\r\n")
                        .await
                        .unwrap();
                }
            }
            assert!(plain.buffer().is_empty());
            plain.into_inner()
        } else {
            tcp
        };
        // Incorrect hostname/certificate ends here or at the first encrypted
        // read. Neither path may receive authentication credentials.
        let Ok(tls) = acceptor.accept(tcp).await else {
            return;
        };
        let mut stream = BufReader::new(tls);
        if !starttls {
            let _ = stream
                .get_mut()
                .write_all(match service {
                    "imap" => b"* OK fixture ready\r\n",
                    "pop3" => b"+OK fixture ready\r\n",
                    _ => b"220 fixture ready\r\n",
                })
                .await;
        }
        loop {
            let mut line = String::new();
            if !matches!(stream.read_line(&mut line).await,Ok(n) if n>0) {
                break;
            }
            let reply = match service {
                "imap" => {
                    let (tag, command) = line.trim_end().split_once(' ').unwrap();
                    if command.starts_with("LOGIN ") {
                        observed.store(true, Ordering::SeqCst);
                    }
                    if command == "CAPABILITY" {
                        format!("* CAPABILITY IMAP4rev1 SPECIAL-USE\r\n{tag} OK capability\r\n")
                    } else if command.starts_with("LIST ") {
                        assert_eq!(command, "LIST \"\" \"*\" RETURN (SPECIAL-USE)");
                        format!("* LIST (\\Sent) \"/\" \"Sent Mail\"\r\n{tag} OK listed\r\n")
                    } else if command.starts_with("UID SEARCH ") {
                        assert_eq!(
                            command,
                            "UID SEARCH HEADER Message-ID \"<gateway-sent@shep.so>\""
                        );
                        format!("* SEARCH\r\n{tag} OK searched\r\n")
                    } else if command.starts_with("APPEND ") {
                        let expected=b"Message-ID: <gateway-sent@shep.so>\r\nSubject: Pinned Sent\r\n\r\nOriginal bytes: \0\xff";
                        assert!(command.starts_with("APPEND \"Sent Mail\" (\\Seen) \""));
                        assert!(command.ends_with(&format!(" {{{}}}", expected.len())));
                        stream.get_mut().write_all(b"+ Ready\r\n").await.unwrap();
                        let mut bytes = vec![0; expected.len() + 2];
                        stream.read_exact(&mut bytes).await.unwrap();
                        assert_eq!(bytes, [expected.as_slice(), b"\r\n"].concat());
                        format!("{tag} OK [APPENDUID 42 7] committed\r\n")
                    } else if command.starts_with("EXAMINE ") {
                        format!(
                            "* 1 EXISTS\r\n* OK [UIDVALIDITY 42] fixture\r\n{tag} OK examined\r\n"
                        )
                    } else if command == "LOGOUT" {
                        let _ = stream
                            .get_mut()
                            .write_all(format!("* BYE fixture\r\n{tag} OK logout\r\n").as_bytes())
                            .await;
                        break;
                    } else {
                        format!("{tag} OK accepted\r\n")
                    }
                }
                "pop3" => {
                    if line.starts_with("PASS ") {
                        observed.store(true, Ordering::SeqCst);
                    }
                    assert!(!line.starts_with("DELE "));
                    if line == "QUIT\r\n" {
                        let _ = stream.get_mut().write_all(b"+OK bye\r\n").await;
                        break;
                    }
                    "+OK accepted\r\n".into()
                }
                _ => {
                    if line.starts_with("EHLO ") {
                        "250-fixture\r\n250 AUTH PLAIN\r\n".into()
                    } else if line.starts_with("AUTH PLAIN ") {
                        observed.store(true, Ordering::SeqCst);
                        "235 authenticated\r\n".into()
                    } else if line == "QUIT\r\n" {
                        let _ = stream.get_mut().write_all(b"221 bye\r\n").await;
                        break;
                    } else {
                        assert_eq!(line, "NOOP\r\n");
                        "250 OK\r\n".into()
                    }
                }
            };
            if stream.get_mut().write_all(reply.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    let mut client = PinnedMail::new(peer, peer);
    client.incoming.test_ca = Some(native_tls::Certificate::from_pem(CERT).unwrap());
    client.smtp_test_ca =
        Some(lettre::transport::smtp::client::Certificate::from_pem(CERT).unwrap());
    (client, server, authenticated)
}
fn account(service: &str, starttls: bool, wrong_host: bool) -> Account {
    let host = if wrong_host {
        "wrong.example.test"
    } else {
        "localhost"
    };
    let mut account:Account=serde_json::from_value(serde_json::json!({"id":"fixture","name":"Fixture","email":"fixture@example.test","protocol":if service=="pop3"{"Pop3"}else{"Imap"},"host":host,"port":993,"username":"fixture-user","smtp_host":host,"smtp_port":465,"smtp_auth":"Plain"})).unwrap();
    let security = if starttls {
        ConnectionSecurity::StartTls
    } else {
        ConnectionSecurity::Tls
    };
    account.incoming_security = security;
    account.smtp_security = Some(security);
    account
}
#[tokio::test]
async fn pinned_imap_and_pop3_preserve_tls_hostname_validation_and_upgrade_before_auth() {
    for service in ["imap", "pop3"] {
        for starttls in [false, true] {
            for wrong_host in [false, true] {
                let (client, server, authenticated) = fixture(service, starttls).await;
                let result = client
                    .probe_incoming(
                        &account(service, starttls, wrong_host),
                        &SecretString::from("fixture-password"),
                    )
                    .await;
                assert_eq!(result.is_ok(), !wrong_host);
                tokio::time::timeout(Duration::from_secs(5), server)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(authenticated.load(Ordering::SeqCst), !wrong_host);
            }
        }
    }
}
#[tokio::test]
async fn pinned_smtp_uses_required_tls_with_original_hostname_and_no_unauthenticated_relay() {
    for starttls in [false, true] {
        for wrong_host in [false, true] {
            let (client, server, authenticated) = fixture("smtp", starttls).await;
            let mut account = account("smtp", starttls, wrong_host);
            let result = client
                .probe_smtp(&account, &SecretString::from("fixture-password"))
                .await;
            assert_eq!(result.is_ok(), !wrong_host);
            tokio::time::timeout(Duration::from_secs(5), server)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(authenticated.load(Ordering::SeqCst), !wrong_host);
            account.smtp_auth = SmtpAuth::None;
            assert!(
                client
                    .probe_smtp(&account, &SecretString::from(""))
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn pinned_sent_discovery_lookup_and_append_keep_tls_identity_and_original_bytes() {
    for starttls in [false, true] {
        for wrong_host in [false, true] {
            let (client, server, authenticated) = fixture("imap", starttls).await;
            let result = client
                .sent(
                    &account("imap", starttls, wrong_host),
                    &SecretString::from("fixture-password"),
                )
                .await;
            assert_eq!(result.is_ok(), !wrong_host);
            if let Ok(mut sent) = result {
                assert_eq!(sent.folder, "Sent Mail");
                assert!(sent.find("<gateway-sent@shep.so>").await.unwrap().is_none());
                let raw=b"Message-ID: <gateway-sent@shep.so>\r\nSubject: Pinned Sent\r\n\r\nOriginal bytes: \0\xff";
                assert_eq!(
                    sent.append(raw, 1788688800).await.unwrap().folder,
                    "Sent Mail"
                );
            }
            tokio::time::timeout(Duration::from_secs(5), server)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(authenticated.load(Ordering::SeqCst), !wrong_host);
        }
    }
}
