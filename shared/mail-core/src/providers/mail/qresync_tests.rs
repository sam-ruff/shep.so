//! Scripted IMAP transcripts for QRESYNC expunge discovery and its fallbacks.
use super::condstore::{FolderState, Resume};
use super::condstore_tests::{body, flags, known, ok, rejected, run, saved, subjects};
use super::*;

const QRESYNC: &str = "* CAPABILITY IMAP4rev1 ENABLE CONDSTORE QRESYNC\r\n";
const SELECT: &str = "SELECT \"INBOX\" (CONDSTORE)";
const CHANGES: &str = "UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 90 VANISHED)";

fn enabled() -> super::condstore_tests::Step {
    ok("ENABLE QRESYNC", "* ENABLED QRESYNC\r\n")
}

fn selected(exists: u32, validity: u32, modseq: u64) -> String {
    format!(
        "* {exists} EXISTS\r\n* OK [UIDVALIDITY {validity}] valid\r\n* OK [HIGHESTMODSEQ {modseq}] modseq\r\n"
    )
}

fn vanished(items: &[MailSyncItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            MailSyncItem::Vanished {
                account,
                folder,
                ids,
            } => {
                assert_eq!((account.as_str(), folder.as_str()), ("test", "INBOX"));
                Some(ids.clone())
            }
            _ => None,
        })
        .flatten()
        .collect()
}

fn listing(items: &[MailSyncItem]) -> Option<HashSet<String>> {
    items.iter().find_map(|item| match item {
        MailSyncItem::Reconcile { live_ids, .. } => Some(live_ids.clone()),
        _ => None,
    })
}

/// Saved states, each published after the folder's listing or removals.
fn states(items: &[MailSyncItem]) -> Vec<Option<FolderState>> {
    let settled = items.iter().rposition(|item| {
        matches!(
            item,
            MailSyncItem::Reconcile { .. } | MailSyncItem::Vanished { .. } | MailSyncItem::Flags(_)
        )
    });
    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            MailSyncItem::FolderState { state, .. } => {
                assert!(settled.is_none_or(|settled| settled < index));
                Some(*state)
            }
            _ => None,
        })
        .collect()
}

const STATE: FolderState = FolderState {
    validity: 12,
    modseq: 120,
};

#[tokio::test]
async fn vanished_mail_is_removed_and_new_mail_fetched_without_listing_every_uid() {
    let (result, items) = run(
        QRESYNC,
        vec![
            enabled(),
            ok(SELECT, &selected(3, 12, 120)),
            ok(
                CHANGES,
                "* VANISHED (EARLIER) 1:5\r\n* 1 FETCH (UID 6 MODSEQ (100) FLAGS (\\Seen))\r\n* 3 FETCH (UID 8 MODSEQ (110) FLAGS ())\r\n",
            ),
            ok(
                "UID FETCH 8 (UID FLAGS RFC822.SIZE)",
                "* 3 FETCH (UID 8 FLAGS () RFC822.SIZE 70)\r\n",
            ),
            ok("UID FETCH 8 (UID FLAGS BODY.PEEK[])", &body(8, "New")),
        ],
        known(12, &[5, 6, 7]),
        saved(12, 90),
    )
    .await;
    result.unwrap();
    assert_eq!(
        flags(&items),
        vec![("test:INBOX:12.6".into(), false, false)]
    );
    assert_eq!(subjects(&items), vec!["New"]);
    assert_eq!(vanished(&items), vec!["test:INBOX:12.5".to_string()]);
    assert_eq!(listing(&items), None, "no complete listing was sent");
    assert_eq!(states(&items), vec![Some(STATE)]);
}

#[tokio::test]
async fn an_unchanged_folder_of_the_same_size_sends_nothing_after_select() {
    let (result, items) = run(
        QRESYNC,
        vec![enabled(), ok(SELECT, &selected(2, 12, 120))],
        known(12, &[6, 7]),
        saved(12, 120),
    )
    .await;
    result.unwrap();
    assert!(flags(&items).is_empty());
    assert!(vanished(&items).is_empty());
    assert_eq!(listing(&items), None);
    assert!(
        states(&items).is_empty(),
        "the saved state is still current"
    );
}

#[tokio::test]
async fn a_rejected_request_after_partial_vanished_data_uses_the_complete_listing() {
    let (result, items) = run(
        QRESYNC,
        vec![
            enabled(),
            ok(SELECT, &selected(2, 12, 120)),
            rejected(
                CHANGES,
                "* VANISHED (EARLIER) 6\r\n* 1 FETCH (UID 7 FLAGS (\\Seen))\r\n",
            ),
            ok("UID SEARCH ALL", "* SEARCH 6 7\r\n"),
            ok(
                "UID FETCH 7,6 (UID FLAGS RFC822.SIZE)",
                "* 2 FETCH (UID 7 FLAGS () RFC822.SIZE 70)\r\n* 1 FETCH (UID 6 FLAGS (\\Flagged) RFC822.SIZE 70)\r\n",
            ),
        ],
        known(12, &[6, 7]),
        saved(12, 90),
    )
    .await;
    result.unwrap();
    assert!(vanished(&items).is_empty(), "partial VANISHED is discarded");
    assert_eq!(
        flags(&items),
        vec![
            ("test:INBOX:12.7".into(), true, false),
            ("test:INBOX:12.6".into(), true, true),
        ],
        "every flag comes from the complete listing"
    );
    assert_eq!(listing(&items), Some(known(12, &[6, 7])));
    assert_eq!(states(&items), vec![Some(STATE)]);
}

#[tokio::test]
async fn a_size_disagreement_lists_every_uid_and_finds_uncached_mail() {
    let (result, items) = run(
        QRESYNC,
        vec![
            enabled(),
            ok(SELECT, &selected(3, 12, 120)),
            ok(
                CHANGES,
                "* 2 FETCH (UID 6 MODSEQ (100) FLAGS (\\Flagged))\r\n",
            ),
            ok("UID SEARCH ALL", "* SEARCH 5 6 7\r\n"),
            ok(
                "UID FETCH 5 (UID FLAGS RFC822.SIZE)",
                "* 1 FETCH (UID 5 FLAGS () RFC822.SIZE 70)\r\n",
            ),
            ok("UID FETCH 5 (UID FLAGS BODY.PEEK[])", &body(5, "Missed")),
        ],
        known(12, &[6, 7]),
        saved(12, 90),
    )
    .await;
    result.unwrap();
    assert_eq!(flags(&items), vec![("test:INBOX:12.6".into(), true, true)]);
    assert_eq!(subjects(&items), vec!["Missed"]);
    assert_eq!(listing(&items), Some(known(12, &[5, 6, 7])));
    assert!(vanished(&items).is_empty());
    assert_eq!(states(&items), vec![Some(STATE)]);
}

#[tokio::test]
async fn a_live_vanished_during_the_refresh_uses_the_complete_listing() {
    let (result, items) = run(
        QRESYNC,
        vec![
            enabled(),
            ok(SELECT, &selected(2, 12, 120)),
            ok(
                CHANGES,
                "* 1 FETCH (UID 6 MODSEQ (100) FLAGS (\\Seen))\r\n* VANISHED 7\r\n",
            ),
            ok("UID SEARCH ALL", "* SEARCH 6\r\n"),
        ],
        known(12, &[6, 7]),
        saved(12, 90),
    )
    .await;
    result.unwrap();
    assert_eq!(
        flags(&items),
        vec![("test:INBOX:12.6".into(), false, false)]
    );
    assert_eq!(listing(&items), Some(known(12, &[6])));
    assert!(vanished(&items).is_empty());
    assert_eq!(states(&items), vec![Some(STATE)]);
}

#[tokio::test]
async fn a_new_uidvalidity_lists_the_whole_folder() {
    let (result, items) = run(
        QRESYNC,
        vec![
            enabled(),
            ok(SELECT, &selected(1, 12, 120)),
            ok("UID SEARCH ALL", "* SEARCH 6\r\n"),
            ok(
                "UID FETCH 6 (UID FLAGS RFC822.SIZE)",
                "* 1 FETCH (UID 6 FLAGS () RFC822.SIZE 70)\r\n",
            ),
            ok(
                "UID FETCH 6 (UID FLAGS BODY.PEEK[])",
                &body(6, "Renumbered"),
            ),
        ],
        known(11, &[6]),
        saved(11, 90),
    )
    .await;
    result.unwrap();
    assert_eq!(subjects(&items), vec!["Renumbered"]);
    assert_eq!(listing(&items), Some(known(12, &[6])));
    assert_eq!(states(&items), vec![Some(STATE)]);
}

#[tokio::test]
async fn a_refused_enable_keeps_condstore_flag_refresh() {
    let (result, items) = run(
        QRESYNC,
        vec![
            rejected("ENABLE QRESYNC", ""),
            ok(SELECT, &selected(2, 12, 120)),
            ok("UID SEARCH ALL", "* SEARCH 6 7\r\n"),
            ok(
                "UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 90)",
                "* 1 FETCH (UID 6 MODSEQ (100) FLAGS (\\Seen))\r\n",
            ),
        ],
        known(12, &[6, 7]),
        saved(12, 90),
    )
    .await;
    result.unwrap();
    assert_eq!(
        flags(&items),
        vec![("test:INBOX:12.6".into(), false, false)]
    );
    assert_eq!(listing(&items), Some(known(12, &[6, 7])));
    assert_eq!(states(&items), vec![Some(STATE)]);
}

#[tokio::test]
async fn without_a_saved_state_qresync_is_not_enabled() {
    let (result, items) = run(
        QRESYNC,
        vec![
            ok(SELECT, &selected(1, 12, 120)),
            ok("UID SEARCH ALL", "* SEARCH 6\r\n"),
            ok(
                "UID FETCH 6 (UID FLAGS RFC822.SIZE)",
                "* 1 FETCH (UID 6 FLAGS () RFC822.SIZE 70)\r\n",
            ),
        ],
        known(12, &[6]),
        Resume::new(),
    )
    .await;
    result.unwrap();
    assert_eq!(listing(&items), Some(known(12, &[6])));
    assert_eq!(states(&items), vec![Some(STATE)]);
}
