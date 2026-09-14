use super::*;
use lanes::SLOW_LANE_BYTES;

fn message(subject: &str, padding: usize) -> String {
    format!(
        "From: Fixture <sender@example.test>\r\nSubject: {subject}\r\n\r\n{}",
        "x".repeat(padding)
    )
}

fn metadata(uid: u32, body: &str) -> String {
    format!(
        "* 1 FETCH (UID {uid} FLAGS () RFC822.SIZE {})\r\n",
        body.len()
    )
}

fn body(uid: u32, body: &str) -> String {
    format!(
        "* 1 FETCH (UID {uid} FLAGS () BODY[] {{{}}}\r\n{body})\r\n",
        body.len()
    )
}

fn account() -> Account {
    serde_json::from_value(serde_json::json!({"id":"test", "name":"Test", "email":"test@example.test", "protocol":"Imap", "host":"localhost", "port":993, "username":"test", "smtp_host":"localhost", "smtp_port":465})).unwrap()
}

fn describe(item: &MailSyncItem) -> Option<String> {
    match item {
        MailSyncItem::InboxSyncStarted { .. } => Some("start".into()),
        MailSyncItem::InboxSyncFinished { .. } => Some("finish".into()),
        MailSyncItem::Message(mail) => Some(mail.summary.subject.clone()),
        MailSyncItem::Reconcile { folder, .. } => Some(format!("reconcile:{folder}")),
        _ => None,
    }
}

/// Inbox holds three small bodies (two of equal size) and one over the slow
/// threshold; Archive holds one small body. Small bodies go smallest first
/// with the newest breaking ties, the slow body follows every folder's small
/// mail, and Inbox only finishes after its slow body. A rejected slow fetch
/// keeps the earlier messages and the complete listings that were sent.
#[tokio::test]
async fn imap_sync_orders_small_bodies_first_and_slow_bodies_after_every_folder() {
    for fail_slow in [false, true] {
        let six = message("S6", 10);
        let seven = message("S7", 10);
        let nine = message("S9", 40);
        let eight = message("S8", SLOW_LANE_BYTES);
        let three = message("S3", 5);
        assert!(eight.len() > SLOW_LANE_BYTES);
        let script: Vec<(String, String)> = vec![
            ("LOGIN".into(), String::new()),
            ("CAPABILITY".into(), "* CAPABILITY IMAP4rev1\r\n".into()),
            (
                "LIST \"\" *".into(),
                "* LIST () \"/\" \"INBOX\"\r\n* LIST () \"/\" \"Archive\"\r\n".into(),
            ),
            (
                "SELECT \"INBOX\"".into(),
                "* 4 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n".into(),
            ),
            ("UID SEARCH ALL".into(), "* SEARCH 6 7 8 9\r\n".into()),
            (
                "UID FETCH 9,8,7,6 (UID FLAGS RFC822.SIZE)".into(),
                [
                    metadata(9, &nine),
                    metadata(8, &eight),
                    metadata(7, &seven),
                    metadata(6, &six),
                ]
                .concat(),
            ),
            (
                "UID FETCH 7,6,9 (UID FLAGS BODY.PEEK[])".into(),
                [body(7, &seven), body(6, &six), body(9, &nine)].concat(),
            ),
            (
                "SELECT \"Archive\"".into(),
                "* 1 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n".into(),
            ),
            ("UID SEARCH ALL".into(), "* SEARCH 3\r\n".into()),
            (
                "UID FETCH 3 (UID FLAGS RFC822.SIZE)".into(),
                metadata(3, &three),
            ),
            (
                "UID FETCH 3 (UID FLAGS BODY.PEEK[])".into(),
                body(3, &three),
            ),
            (
                "SELECT \"INBOX\"".into(),
                "* 4 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n".into(),
            ),
            (
                "UID FETCH 8 (UID FLAGS BODY.PEEK[])".into(),
                body(8, &eight),
            ),
            ("LOGOUT".into(), "* BYE goodbye\r\n".into()),
        ];
        let (client, server) = tokio::io::duplex(8192);
        let server = tokio::spawn(async move {
            let mut server = BufReader::new(server);
            for (expected, response) in script {
                let mut line = String::new();
                server.read_line(&mut line).await.unwrap();
                let (tag, command) = line.trim_end().split_once(' ').unwrap();
                if expected == "LOGIN" {
                    assert!(command.starts_with("LOGIN "));
                } else {
                    assert_eq!(command, expected);
                }
                if fail_slow && expected.starts_with("UID FETCH 8 ") {
                    server
                        .get_mut()
                        .write_all(format!("{tag} NO fixture failure\r\n").as_bytes())
                        .await
                        .unwrap();
                    break;
                }
                server
                    .get_mut()
                    .write_all(format!("{response}{tag} OK done\r\n").as_bytes())
                    .await
                    .unwrap();
            }
        });
        let session = async_imap::Client::new(client)
            .login("test", "secret")
            .await
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let consume = async move {
            let mut items = Vec::new();
            while let Some(item) = rx.recv().await {
                items.push(item);
            }
            items
        };
        let account = account();
        let known = HashSet::new();
        let (result, items) = tokio::join!(
            tokio::time::timeout(
                Duration::from_secs(10),
                sync_imap_session(session, &account, &known, tx, None),
            ),
            consume
        );
        let result = result.unwrap();
        assert_eq!(result.is_ok(), !fail_slow, "{result:?}");
        let order: Vec<_> = items.iter().filter_map(describe).collect();
        let mut expected = vec![
            "start",
            "S7",
            "S6",
            "S9",
            "reconcile:INBOX",
            "S3",
            "reconcile:Archive",
        ];
        if !fail_slow {
            expected.extend(["S8", "finish"]);
        }
        assert_eq!(order, expected, "fail_slow={fail_slow}");
        let listing = items.iter().find_map(|item| match item {
            MailSyncItem::Reconcile {
                folder, live_ids, ..
            } if folder == "INBOX" => Some(live_ids.clone()),
            _ => None,
        });
        // The store deletes cached rows missing from the listing, so the
        // deferred (and possibly never received) body must stay listed.
        assert_eq!(
            listing.unwrap(),
            HashSet::from([
                "test:INBOX:12.6".to_string(),
                "test:INBOX:12.7".to_string(),
                "test:INBOX:12.8".to_string(),
                "test:INBOX:12.9".to_string(),
            ])
        );
        if !fail_slow {
            let large = items
                .iter()
                .find_map(|item| match item {
                    MailSyncItem::Message(mail) if mail.summary.subject == "S8" => Some(mail),
                    _ => None,
                })
                .unwrap();
            assert_eq!(large.raw, eight.as_bytes());
        }
        server.await.unwrap();
    }
}
