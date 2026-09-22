//! Scripted IMAP transcripts for CONDSTORE flag refresh and its fallbacks.
use super::condstore::{FolderState, Resume};
use super::*;

const CONDSTORE: &str = "* CAPABILITY IMAP4rev1 CONDSTORE\r\n";
const PLAIN: &str = "* CAPABILITY IMAP4rev1\r\n";
const LIST: &str = "* LIST () \"/\" \"INBOX\"\r\n";

/// One expected command, its untagged reply and whether it completes OK.
struct Step {
    command: String,
    reply: String,
    ok: bool,
}
fn ok(command: &str, reply: &str) -> Step {
    Step {
        command: command.into(),
        reply: reply.into(),
        ok: true,
    }
}
fn rejected(command: &str, reply: &str) -> Step {
    Step {
        ok: false,
        ..ok(command, reply)
    }
}

fn account() -> Account {
    serde_json::from_value(serde_json::json!({"id":"test", "name":"Test", "email":"test@example.test", "protocol":"Imap", "host":"localhost", "port":993, "username":"test", "smtp_host":"localhost", "smtp_port":465})).unwrap()
}

fn known(validity: u32, uids: &[u32]) -> HashSet<String> {
    uids.iter()
        .map(|uid| format!("test:INBOX:{validity}.{uid}"))
        .collect()
}

fn saved(validity: u32, modseq: u64) -> Resume {
    Resume::from([("INBOX".to_string(), FolderState { validity, modseq })])
}

fn body(uid: u32, subject: &str) -> String {
    let raw = format!("From: Fixture <sender@example.test>\r\nSubject: {subject}\r\n\r\nHello");
    format!(
        "* 1 FETCH (UID {uid} FLAGS () BODY[] {{{}}}\r\n{raw})\r\n",
        raw.len()
    )
}

/// Runs a sync against the script and returns its result and the items it
/// published. Every scripted command must arrive in order.
async fn run(
    capabilities: &str,
    steps: Vec<Step>,
    known: HashSet<String>,
    resume: Resume,
) -> (anyhow::Result<Vec<String>>, Vec<MailSyncItem>) {
    let mut script = vec![ok("CAPABILITY", capabilities), ok("LIST \"\" *", LIST)];
    script.extend(steps);
    script.push(ok("LOGOUT", "* BYE goodbye\r\n"));
    let (client, server) = tokio::io::duplex(8192);
    let server = tokio::spawn(async move {
        let mut server = BufReader::new(server);
        let mut line = String::new();
        server.read_line(&mut line).await.unwrap();
        let (tag, _) = line.split_once(' ').unwrap();
        let tag = tag.to_owned();
        server
            .get_mut()
            .write_all(format!("{tag} OK login\r\n").as_bytes())
            .await
            .unwrap();
        for step in script {
            line.clear();
            if server.read_line(&mut line).await.unwrap() == 0 {
                return;
            }
            let (tag, command) = line.trim_end().split_once(' ').unwrap();
            assert_eq!(command, step.command);
            let status = if step.ok { "OK" } else { "NO fixture refusal" };
            server
                .get_mut()
                .write_all(format!("{}{tag} {status}\r\n", step.reply).as_bytes())
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
    let (result, items) = tokio::join!(
        tokio::time::timeout(
            Duration::from_secs(10),
            sync_imap_session_mode(
                session,
                &account,
                &known,
                tx,
                None,
                ReceiveMode::Staged,
                Some(&resume),
            ),
        ),
        consume
    );
    server.await.unwrap();
    (result.unwrap(), items)
}

/// Every flag update in publication order.
fn flags(items: &[MailSyncItem]) -> Vec<(String, bool, bool)> {
    items
        .iter()
        .filter_map(|item| match item {
            MailSyncItem::Flags(flags) => Some(flags.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

/// The folder state updates, which must follow the folder's listing.
fn states(items: &[MailSyncItem]) -> Vec<Option<FolderState>> {
    let reconcile = items
        .iter()
        .position(|item| matches!(item, MailSyncItem::Reconcile { .. }));
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            MailSyncItem::FolderState {
                account,
                folder,
                state,
            } => {
                assert_eq!((account.as_str(), folder.as_str()), ("test", "INBOX"));
                assert!(reconcile.is_some_and(|reconcile| reconcile < index));
                Some(*state)
            }
            _ => None,
        })
        .collect()
}

fn subjects(items: &[MailSyncItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            MailSyncItem::Message(mail) => Some(mail.summary.subject.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn changed_flags_replace_the_full_listing_and_only_new_mail_is_fetched() {
    let (result, items) = run(
        CONDSTORE,
        vec![
            ok(
                "SELECT \"INBOX\" (CONDSTORE)",
                "* 3 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n* OK [HIGHESTMODSEQ 120] modseq\r\n",
            ),
            ok("UID SEARCH ALL", "* SEARCH 6 7 8\r\n"),
            ok(
                "UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 90)",
                "* 1 FETCH (UID 6 MODSEQ (100) FLAGS (\\Seen \\Flagged))\r\n* 3 FETCH (UID 8 MODSEQ (110) FLAGS ())\r\n",
            ),
            ok(
                "UID FETCH 8 (UID FLAGS RFC822.SIZE)",
                "* 3 FETCH (UID 8 FLAGS () RFC822.SIZE 70)\r\n",
            ),
            ok("UID FETCH 8 (UID FLAGS BODY.PEEK[])", &body(8, "New")),
        ],
        known(12, &[6, 7]),
        saved(12, 90),
    )
    .await;
    result.unwrap();
    assert_eq!(
        flags(&items),
        vec![("test:INBOX:12.6".into(), false, true)],
        "only the changed cached message is refreshed"
    );
    assert_eq!(subjects(&items), vec!["New"]);
    assert_eq!(
        states(&items),
        vec![Some(FolderState {
            validity: 12,
            modseq: 120
        })]
    );
}

#[tokio::test]
async fn an_unchanged_folder_skips_the_flag_fetch() {
    let (result, items) = run(
        CONDSTORE,
        vec![
            ok(
                "SELECT \"INBOX\" (CONDSTORE)",
                "* 2 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n* OK [HIGHESTMODSEQ 120] modseq\r\n",
            ),
            ok("UID SEARCH ALL", "* SEARCH 6 7\r\n"),
        ],
        known(12, &[6, 7]),
        saved(12, 120),
    )
    .await;
    result.unwrap();
    assert!(flags(&items).is_empty());
    assert!(states(&items).is_empty(), "the saved state is unchanged");
}

#[tokio::test]
async fn the_first_condstore_check_lists_every_flag_and_saves_its_state() {
    let (result, items) = run(
        CONDSTORE,
        vec![
            ok(
                "SELECT \"INBOX\" (CONDSTORE)",
                "* 2 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n* OK [HIGHESTMODSEQ 90] modseq\r\n",
            ),
            ok("UID SEARCH ALL", "* SEARCH 6 7\r\n"),
            ok(
                "UID FETCH 7,6 (UID FLAGS RFC822.SIZE)",
                "* 2 FETCH (UID 7 FLAGS () RFC822.SIZE 70)\r\n* 1 FETCH (UID 6 FLAGS (\\Seen) RFC822.SIZE 70)\r\n",
            ),
        ],
        known(12, &[6, 7]),
        Resume::new(),
    )
    .await;
    result.unwrap();
    assert_eq!(flags(&items).len(), 2);
    assert_eq!(
        states(&items),
        vec![Some(FolderState {
            validity: 12,
            modseq: 90
        })]
    );
}

#[tokio::test]
async fn a_server_without_condstore_uses_the_full_path_and_forgets_saved_state() {
    let (result, items) = run(
        PLAIN,
        vec![
            ok(
                "SELECT \"INBOX\"",
                "* 2 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n",
            ),
            ok("UID SEARCH ALL", "* SEARCH 6 7\r\n"),
            ok(
                "UID FETCH 7,6 (UID FLAGS RFC822.SIZE)",
                "* 2 FETCH (UID 7 FLAGS () RFC822.SIZE 70)\r\n* 1 FETCH (UID 6 FLAGS () RFC822.SIZE 70)\r\n",
            ),
        ],
        known(12, &[6, 7]),
        saved(12, 90),
    )
    .await;
    result.unwrap();
    assert_eq!(flags(&items).len(), 2);
    assert_eq!(states(&items), vec![None]);
}

#[tokio::test]
async fn nomodseq_uses_the_full_path_and_forgets_saved_state() {
    let (result, items) = run(
        CONDSTORE,
        vec![
            ok(
                "SELECT \"INBOX\" (CONDSTORE)",
                "* 2 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n* OK [NOMODSEQ] no persistent mod-sequences\r\n",
            ),
            ok("UID SEARCH ALL", "* SEARCH 6 7\r\n"),
            ok(
                "UID FETCH 7,6 (UID FLAGS RFC822.SIZE)",
                "* 2 FETCH (UID 7 FLAGS () RFC822.SIZE 70)\r\n* 1 FETCH (UID 6 FLAGS () RFC822.SIZE 70)\r\n",
            ),
        ],
        known(12, &[6, 7]),
        saved(12, 90),
    )
    .await;
    result.unwrap();
    assert_eq!(flags(&items).len(), 2);
    assert_eq!(states(&items), vec![None]);
}

#[tokio::test]
async fn a_new_uidvalidity_refetches_the_folder_and_replaces_the_state() {
    let (result, items) = run(
        CONDSTORE,
        vec![
            ok(
                "SELECT \"INBOX\" (CONDSTORE)",
                "* 1 EXISTS\r\n* OK [UIDVALIDITY 13] valid\r\n* OK [HIGHESTMODSEQ 5] modseq\r\n",
            ),
            ok("UID SEARCH ALL", "* SEARCH 1\r\n"),
            ok(
                "UID FETCH 1 (UID FLAGS RFC822.SIZE)",
                "* 1 FETCH (UID 1 FLAGS () RFC822.SIZE 70)\r\n",
            ),
            ok(
                "UID FETCH 1 (UID FLAGS BODY.PEEK[])",
                &body(1, "Renumbered"),
            ),
        ],
        known(12, &[6, 7]),
        saved(12, 90),
    )
    .await;
    result.unwrap();
    assert_eq!(subjects(&items), vec!["Renumbered"]);
    assert_eq!(
        states(&items),
        vec![Some(FolderState {
            validity: 13,
            modseq: 5
        })]
    );
}

#[tokio::test]
async fn a_rejected_changedsince_after_partial_data_falls_back_to_every_flag() {
    let (result, items) = run(
        CONDSTORE,
        vec![
            ok(
                "SELECT \"INBOX\" (CONDSTORE)",
                "* 2 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n* OK [HIGHESTMODSEQ 120] modseq\r\n",
            ),
            ok("UID SEARCH ALL", "* SEARCH 6 7\r\n"),
            rejected(
                "UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 90)",
                "* 1 FETCH (UID 6 MODSEQ (100) FLAGS (\\Seen))\r\n",
            ),
            ok(
                "UID FETCH 7,6 (UID FLAGS RFC822.SIZE)",
                "* 2 FETCH (UID 7 FLAGS (\\Flagged) RFC822.SIZE 70)\r\n* 1 FETCH (UID 6 FLAGS () RFC822.SIZE 70)\r\n",
            ),
        ],
        known(12, &[6, 7]),
        saved(12, 90),
    )
    .await;
    result.unwrap();
    assert_eq!(
        flags(&items),
        vec![
            ("test:INBOX:12.7".into(), true, true),
            ("test:INBOX:12.6".into(), true, false),
        ],
        "the partial CHANGEDSINCE data is never used"
    );
    assert_eq!(
        states(&items),
        vec![Some(FolderState {
            validity: 12,
            modseq: 120
        })]
    );
}

#[tokio::test]
async fn a_failed_check_never_saves_the_folder_state() {
    let (result, items) = run(
        CONDSTORE,
        vec![
            ok(
                "SELECT \"INBOX\" (CONDSTORE)",
                "* 2 EXISTS\r\n* OK [UIDVALIDITY 12] valid\r\n* OK [HIGHESTMODSEQ 120] modseq\r\n",
            ),
            ok("UID SEARCH ALL", "* SEARCH 6 7\r\n"),
            rejected("UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 90)", ""),
            rejected("UID FETCH 7,6 (UID FLAGS RFC822.SIZE)", ""),
        ],
        known(12, &[6, 7]),
        saved(12, 90),
    )
    .await;
    assert!(result.is_err());
    assert!(flags(&items).is_empty());
    assert!(states(&items).is_empty());
}
