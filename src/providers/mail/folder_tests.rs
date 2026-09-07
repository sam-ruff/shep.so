use super::*;
use crate::{folders::NameEncoding, store::Store};

#[tokio::test]
async fn listing_preserves_hierarchy_and_never_selects_container_names() {
    let (client, server) = tokio::io::duplex(8192);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        let names = [
            "INBOX",
            "Projects/Design/&ZeVnLIqe-",
            "Shared.Team.Plans",
            "Flat/Name.With.Dots",
        ];
        let mut commands = vec![
            "LOGIN \"test\" \"secret\"".to_string(),
            "CAPABILITY".into(),
            "LIST \"\" *".into(),
        ];
        for name in names {
            commands.extend([format!("SELECT \"{name}\""), "UID SEARCH ALL".into()]);
        }
        commands.push("LOGOUT".into());
        for command in commands {
            let mut line = String::new();
            server.read_line(&mut line).await.unwrap();
            let (tag, actual) = line.trim_end().split_once(' ').unwrap();
            assert_eq!(actual, command, "a container must never reach SELECT");
            let response = match command.as_str() {
                "CAPABILITY" => "* CAPABILITY IMAP4rev1\r\n",
                "LIST \"\" *" => concat!(
                    "* LIST () \"/\" \"INBOX\"\r\n",
                    "* LIST (\\Noselect) \"/\" \"Projects/\"\r\n",
                    "* LIST () \"/\" \"Projects/Design/&ZeVnLIqe-\"\r\n",
                    "* LIST (\\Noselect) \".\" \"Shared\"\r\n",
                    "* LIST () \".\" \"Shared.Team.Plans\"\r\n",
                    "* LIST (\\Noinferiors) NIL \"Flat/Name.With.Dots\"\r\n",
                    "* LIST (\\NonExistent) \"/\" \"Gone\"\r\n"
                ),
                "UID SEARCH ALL" => "* SEARCH\r\n",
                "LOGOUT" => "* BYE goodbye\r\n",
                value if value.starts_with("SELECT ") => {
                    "* 0 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n"
                }
                _ => "",
            };
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
    let account:Account=serde_json::from_value(serde_json::json!({"id":"test","name":"Test","email":"test@example.test","protocol":"Imap","host":"localhost","port":993,"username":"test","smtp_host":"localhost","smtp_port":465})).unwrap();
    let store = Store::memory().unwrap();
    store.save_account(account.clone()).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(16);
    let names = tokio::time::timeout(
        Duration::from_secs(5),
        sync_imap_session(session, &account, &HashSet::new(), tx, None),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        names,
        [
            "INBOX",
            "Projects/Design/&ZeVnLIqe-",
            "Shared.Team.Plans",
            "Flat/Name.With.Dots"
        ]
    );
    while let Some(item) = rx.recv().await {
        store.apply_sync(item).await.unwrap();
    }
    let workspace = store.workspace().await.unwrap();
    assert_eq!(workspace.account_folders["test"], names);
    let tree = &workspace.folder_trees["test"];
    assert!(!tree.node("Projects").unwrap().mailbox.selectable);
    assert_eq!(tree.node("Projects").unwrap().mailbox.name, "Projects/");
    assert_eq!(
        tree.node("Projects/Design/&ZeVnLIqe-").unwrap().label,
        "日本語"
    );
    assert_eq!(
        tree.node("Projects/Design/&ZeVnLIqe-")
            .unwrap()
            .mailbox
            .encoding,
        NameEncoding::ImapUtf7
    );
    assert_eq!(tree.node("Flat/Name.With.Dots").unwrap().parent, None);
    assert_eq!(tree.node("Shared.Team").unwrap().label, "Team");
    assert!(!tree.node("Gone").unwrap().mailbox.selectable);
    server.await.unwrap();
}
