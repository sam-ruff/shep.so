use super::*;

fn account(protocol: Protocol) -> Account {
    serde_json::from_value(serde_json::json!({"id":"fixture", "name":"Fixture", "email":"fixture@example.test", "protocol":protocol, "host":"localhost", "port":993, "username":"fixture", "smtp_host":"localhost", "smtp_port":465})).unwrap()
}
fn transitions(items: &[MailSyncItem]) -> Vec<&'static str> {
    items
        .iter()
        .filter_map(|item| match item {
            MailSyncItem::InboxSyncStarted { account, epoch } => {
                assert_eq!(account, "fixture");
                assert!(epoch == "imap:12" || epoch == "pop3");
                Some("start")
            }
            MailSyncItem::Message(_) => Some("message"),
            MailSyncItem::InboxSyncFinished { .. } => Some("finish"),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn imap_notification_baseline_requires_complete_inbox_but_not_successful_logout() {
    for fail in [
        "",
        "SEARCH",
        "FETCH",
        "BODY",
        "LOGOUT",
        "SEARCH_BAD",
        "SEARCH_EOF",
        "SEARCH_TAG",
        "BODY_AFTER_DATA",
    ] {
        let (client, server) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            let raw = "From: Fixture <sender@example.test>\r\nMessage-ID: <one@example.test>\r\nSubject: New\r\n\r\nBody";
            for expected in [
                "LOGIN",
                "CAPABILITY",
                "LIST",
                "SELECT",
                "UID SEARCH",
                "UID FETCH",
                "UID FETCH",
                "LOGOUT",
            ] {
                let mut line = String::new();
                server.read_line(&mut line).await.unwrap();
                let (tag, command) = line.trim_end().split_once(' ').unwrap();
                assert!(
                    command.starts_with(expected),
                    "{command} expected {expected}"
                );
                if !fail.is_empty()
                    && (command.contains(fail)
                        || (command.starts_with("UID SEARCH") && fail.starts_with("SEARCH_")))
                {
                    if fail == "SEARCH_EOF" {
                        break;
                    }
                    let status = if fail == "SEARCH_BAD" { "BAD" } else { "NO" };
                    let tag = if fail == "SEARCH_TAG" {
                        "unrelated"
                    } else {
                        tag
                    };
                    server
                        .get_mut()
                        .write_all(format!("{tag} {status} fixture failure\r\n").as_bytes())
                        .await
                        .unwrap();
                    break;
                }
                let response = if expected == "CAPABILITY" {
                    "* CAPABILITY IMAP4rev1\r\n".into()
                } else if expected == "LIST" {
                    "* LIST () \"/\" \"INBOX\"\r\n".into()
                } else if expected == "SELECT" {
                    "* 1 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n".into()
                } else if expected == "UID SEARCH" {
                    "* SEARCH 1\r\n".into()
                } else if command.contains("RFC822.SIZE") {
                    format!("* 1 FETCH (UID 1 FLAGS () RFC822.SIZE {})\r\n", raw.len())
                } else if expected == "UID FETCH" {
                    format!(
                        "* 1 FETCH (UID 1 FLAGS () BODY[] {{{}}}\r\n{raw})\r\n",
                        raw.len()
                    )
                } else {
                    String::new()
                };
                let completion = if fail == "BODY_AFTER_DATA" && command.contains("BODY") {
                    "NO incomplete"
                } else {
                    "OK done"
                };
                server
                    .get_mut()
                    .write_all(format!("{response}{tag} {completion}\r\n").as_bytes())
                    .await
                    .unwrap();
                if completion == "NO incomplete" {
                    break;
                }
            }
        });
        let session = async_imap::Client::new(client)
            .login("fixture", "secret")
            .await
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let result =
            sync_imap_session(session, &account(Protocol::Imap), &HashSet::new(), tx, None).await;
        assert_eq!(result.is_ok(), fail.is_empty(), "{fail}");
        let mut items = Vec::new();
        while let Some(item) = rx.recv().await {
            items.push(item);
        }
        assert_eq!(
            transitions(&items),
            if !fail.is_empty() && fail != "LOGOUT" {
                vec!["start"]
            } else {
                vec!["start", "message", "finish"]
            },
            "{fail}"
        );
        assert_eq!(
            items
                .iter()
                .any(|item| matches!(item, MailSyncItem::Reconcile { .. })),
            fail.is_empty() || fail == "LOGOUT",
            "An unconfirmed listing must never remove cached messages: {fail}"
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn pop3_notification_baseline_orders_downloads_and_stays_quiet_after_partial_import() {
    for (empty, fail) in [(true, ""), (false, ""), (false, "RETR"), (false, "QUIT")] {
        let (client, server) = tokio::io::duplex(4096);
        let server = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            let raw = "From: Fixture <sender@example.test>\r\nSubject: New\r\n\r\nBody\r\n";
            let commands = if empty {
                vec!["UIDL", "QUIT"]
            } else {
                vec!["UIDL", "LIST 1", "RETR 1", "QUIT"]
            };
            for command in commands {
                let mut line = String::new();
                server.read_line(&mut line).await.unwrap();
                assert_eq!(line.trim_end(), command);
                if !fail.is_empty() && command.starts_with(fail) {
                    server
                        .get_mut()
                        .write_all(b"-ERR fixture failure\r\n")
                        .await
                        .unwrap();
                    break;
                }
                let response = match command {
                    "UIDL" if empty => "+OK\r\n.\r\n".into(),
                    "UIDL" => "+OK\r\n1 stable-uid\r\n.\r\n".into(),
                    "LIST 1" => format!("+OK 1 {}\r\n", raw.len()),
                    "RETR 1" => format!("+OK\r\n{raw}.\r\n"),
                    _ => "+OK\r\n".into(),
                };
                server
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
            }
        });
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let result = sync_pop_session(
            PopConnection(BufReader::new(client)),
            &account(Protocol::Pop3),
            &HashSet::new(),
            tx,
        )
        .await;
        assert_eq!(result.is_ok(), fail.is_empty());
        let mut items = Vec::new();
        while let Some(item) = rx.recv().await {
            items.push(item);
        }
        assert_eq!(
            transitions(&items),
            if empty {
                vec!["start", "finish"]
            } else if fail == "RETR" {
                vec!["start"]
            } else {
                vec!["start", "message", "finish"]
            }
        );
        server.await.unwrap();
    }
}
