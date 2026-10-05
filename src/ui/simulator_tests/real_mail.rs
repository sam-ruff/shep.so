//! Widget input through the production engine, checked against an owned server.
use super::{Harness, harness, real_server::RealServer};
use crate::{credentials::Scope, model::*, store::Store};
use anyhow::Context;
use mailparse::MailHeaderMap;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    time::{Duration, Instant},
};

const MAILS: usize = 12;
const ARRIVAL_BUDGET: Duration = Duration::from_secs(5);
const INPUT_P95_BUDGET: Duration = Duration::from_millis(100);

mod stress;
use stress::*;

struct Fixture {
    server: RealServer,
    directory: PathBuf,
    ids: Vec<String>,
}

impl Fixture {
    async fn new(count: usize) -> anyhow::Result<Self> {
        let server = RealServer::start().await?;
        let ids = server.seed(count).await?;
        let artifacts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("artifacts/e2e-iced");
        std::fs::create_dir_all(&artifacts)?;
        let directory = tempfile::Builder::new()
            .prefix("mail-")
            .tempdir_in(artifacts)?
            .keep();
        eprintln!("Real mail evidence: {}", directory.display());
        let fixture = Self {
            server,
            directory,
            ids,
        };
        let store = fixture.store()?;
        store.save_account(fixture.server.account("alice")).await?;
        store
            .put(
                "preferences",
                Preferences {
                    appearance: Appearance::Light,
                    group_conversations: false,
                    close_to_tray: false,
                    mail_check_seconds: 5,
                    notifications: crate::notifications::Settings {
                        popups: false,
                        sound: false,
                        show_details: false,
                    },
                    ..Default::default()
                },
            )
            .await?;
        Ok(fixture)
    }

    fn store(&self) -> anyhow::Result<Store> {
        Store::open(self.directory.join("cache.sqlite"))
    }

    async fn start(&self) -> anyhow::Result<Harness> {
        self.start_size(1440., 920.).await
    }

    async fn start_size(&self, width: f32, height: f32) -> anyhow::Result<Harness> {
        let credentials = self.server.credentials(Scope::default()).await?;
        Ok(Harness::with_backend(
            width,
            height,
            crate::engine::simulator_support::subscription(self.store()?, credentials),
            Some(
                self.directory
                    .join(format!("state-{}.json", uuid::Uuid::new_v4())),
            ),
        )
        .await)
    }

    async fn loaded(&self, h: &mut Harness, count: usize) -> anyhow::Result<()> {
        h.expect("total", count).await;
        startup_finished(h).await;
        h.expect("background_sync", false).await;
        settled(h).await;
        let messages = self.server.messages("alice", "INBOX").await?;
        assert_eq!(messages.len(), count);
        for row in h.state()["mail_rows"].as_array().context("mail rows")? {
            let remote = messages
                .iter()
                .find(|mail| mail.remote_id() == row["remote_id"].as_str().unwrap_or_default())
                .context("every UI row must have its exact IMAP UID")?;
            assert_eq!(row["subject"], remote.subject);
            assert_eq!(row["unread"], !remote.flags.iter().any(|f| f == "\\Seen"));
            assert_eq!(
                row["starred"],
                remote.flags.iter().any(|f| f == "\\Flagged")
            );
        }
        Ok(())
    }

    async fn ids_in(&self, folder: &str) -> anyhow::Result<BTreeSet<String>> {
        let messages = self.server.messages("alice", folder).await?;
        let count = messages.len();
        let ids: BTreeSet<_> = messages.into_iter().map(|mail| mail.message_id).collect();
        assert_eq!(count, ids.len(), "duplicate Message-IDs in {folder}");
        Ok(ids)
    }

    async fn assert_ids(
        &self,
        folder: &str,
        ids: impl IntoIterator<Item = String>,
    ) -> anyhow::Result<()> {
        assert_eq!(
            self.ids_in(folder).await?,
            ids.into_iter().collect(),
            "exact Message-IDs in {folder}"
        );
        Ok(())
    }

    fn capture(&self, h: &Harness, name: &str) -> anyhow::Result<()> {
        h.snapshot().matches_image(self.directory.join(name))?;
        Ok(())
    }
}

async fn startup_finished(h: &mut Harness) {
    h.expect_at_least("harness.successful_mail_checks", 1.)
        .await;
    h.expect("harness.outgoing_repaired", true).await;
}

async fn settled(h: &mut Harness) {
    h.expect_state(
        |s| {
            s["mail_pending"] == 0
                && s["bulk"]["staging"].is_null()
                && s["bulk"]["preparing"] == false
                && s["bulk"]["jobs"].as_array().is_some_and(|jobs| {
                    jobs.iter()
                        .all(|j| j["remaining"] == 0 && j["running"] == 0)
                })
                && s["mail_rows"]
                    .as_array()
                    .is_some_and(|rows| rows.iter().all(|row| row["group_pending"] == false))
                && s["store_truth"]["agrees"] == true
                && s["drawn_rows"]["consistent"] == true
        },
        "all admitted mail actions settled",
    )
    .await;
    for job in h.state()["bulk"]["jobs"]
        .as_array()
        .expect("job observations")
    {
        assert_eq!(job["failed"], 0, "{job}");
        assert_eq!(job["uncertain"], 0, "{job}");
    }
}

async fn search(h: &mut Harness, text: &str, count: usize) {
    h.key("ctrl+k").await;
    h.expect("focused_input", "search").await;
    h.key("ctrl+a").await;
    h.key("BackSpace").await;
    h.type_text(text).await;
    h.expect("query", text).await;
    h.expect("total", count).await;
    if count > 0 {
        h.expect_state(
            |s| !s["selected_id"].is_null() && s["loaded_message_id"] == s["selected_id"],
            "current search selection and reader",
        )
        .await;
    }
    h.expect("store_truth.agrees", true).await;
    h.key("Escape").await;
}

async fn field(h: &mut Harness, name: &'static str, value: &str) {
    h.click_id(name).await;
    h.key("ctrl+a").await;
    assert_eq!(
        h.state()["native_focus"],
        name,
        "editing field must retain widget focus"
    );
    h.type_text(value).await;
    assert_eq!(
        h.state()["compose_fields"][name],
        value,
        "every typed character reached {name}"
    );
}

async fn compose(h: &mut Harness, subject: &str, body: &str) {
    h.key("c").await;
    h.expect("composer.visible", true).await;
    h.expect("focused_input", "to").await;
    h.type_text("bob@shep.test").await;
    field(h, "subject", subject).await;
    h.click_id("compose-body").await;
    h.type_text(body).await;
    h.expect("editor", body).await;
}

async fn select_all(h: &mut Harness, count: usize) {
    h.click_at(574., 156.).await;
    h.expect("mail_selection.mode", true).await;
    h.key("ctrl+a").await;
    h.expect("mail_selection.count", count).await;
    h.expect("mail_selection.pending", false).await;
}

async fn confirm(h: &mut Harness, count: usize) {
    h.expect("dialog", "BulkReview").await;
    h.expect("bulk.review_count", count).await;
    h.key("Return").await;
    h.expect("dialog", Value::Null).await;
}

async fn receive_search_and_keyboard_focus() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    f.assert_ids("INBOX", f.ids.clone()).await?;
    search(&mut h, "0003", 1).await;
    h.expect("selected", "Server message 0003").await;
    h.key("ctrl+k").await;
    h.expect("focused_input", "search").await;
    h.key("ctrl+d").await;
    assert_eq!(
        h.state()["mail_rows"][0]["folder"],
        "INBOX",
        "typing focus must block Delete"
    );
    assert_eq!(h.state()["mail_pending"], 0);
    f.assert_ids("INBOX", f.ids.clone()).await?;
    h.key("ctrl+a").await;
    h.type_text("no-such-mail-xyzzy").await;
    h.expect("total", 0).await;
    search(&mut h, "", MAILS).await;
    f.assert_ids("INBOX", f.ids.clone()).await?;
    f.capture(&h, "received-mail")?;
    h.close().await;
    Ok(())
}

async fn read_on_leave_and_flag_are_saved_on_server() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    search(&mut h, "0003", 1).await;
    h.click_text("Server message 0003").await;
    h.expect("read_candidate", "Server message 0003").await;
    h.key("s").await;
    h.expect("starred", true).await;
    settled(&mut h).await;
    let first = f.server.messages("alice", "INBOX").await?;
    assert!(
        first
            .iter()
            .find(|m| m.message_id == f.ids[3])
            .context("flagged message")?
            .flags
            .contains(&"\\Flagged".into())
    );
    search(&mut h, "0004", 1).await;
    settled(&mut h).await;
    let messages = f.server.messages("alice", "INBOX").await?;
    assert!(
        messages
            .iter()
            .find(|m| m.message_id == f.ids[3])
            .context("read message")?
            .flags
            .contains(&"\\Seen".into())
    );
    search(&mut h, "0003", 1).await;
    h.key("s").await;
    h.expect("starred", false).await;
    settled(&mut h).await;
    let messages = f.server.messages("alice", "INBOX").await?;
    assert!(
        !messages
            .iter()
            .find(|m| m.message_id == f.ids[3])
            .context("unflagged message")?
            .flags
            .contains(&"\\Flagged".into())
    );
    h.close().await;
    Ok(())
}

async fn archive_trash_and_undo_keep_exact_identity() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    for (key, destination) in [("BackSpace", "Archive"), ("ctrl+d", "Trash")] {
        search(&mut h, "0003", 1).await;
        h.key(key).await;
        h.expect("mail_rows.0.folder", destination).await;
        settled(&mut h).await;
        f.assert_ids(destination, [f.ids[3].clone()]).await?;
        assert!(!f.ids_in("INBOX").await?.contains(&f.ids[3]));
        h.click_text("Undo").await;
        h.expect("total", 1).await;
        h.expect("mail_rows.0.folder", "INBOX").await;
        settled(&mut h).await;
        f.assert_ids(destination, []).await?;
        f.assert_ids("INBOX", f.ids.clone()).await?;
    }
    h.close().await;
    Ok(())
}

async fn move_dialog_cancel_and_commit() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    search(&mut h, "0003", 1).await;
    h.key("m").await;
    h.expect("focused_input", "folder-search").await;
    h.type_text("Projects").await;
    h.key("Escape").await;
    f.assert_ids("INBOX", f.ids.clone()).await?;
    h.key("m").await;
    h.expect("focused_input", "folder-search").await;
    h.type_text("Projects").await;
    h.expect("move_enter_destination", "Projects").await;
    h.key("Return").await;
    h.expect("dialog", Value::Null).await;
    h.expect("mail_rows.0.folder", "Projects").await;
    settled(&mut h).await;
    f.assert_ids("Projects", [f.ids[3].clone()]).await?;
    h.close().await;
    let mut h = f.start().await?;
    h.expect("total", MAILS - 1).await;
    h.click_text("Projects").await;
    h.expect("selected", "Server message 0003").await;
    settled(&mut h).await;
    f.assert_ids("Projects", [f.ids[3].clone()]).await?;
    h.close().await;
    Ok(())
}

async fn bulk_cancel_and_cross_page_archive_undo() -> anyhow::Result<()> {
    let f = Fixture::new(65).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, 65).await?;
    assert_eq!(h.state()["mail_rows"].as_array().context("page")?.len(), 50);
    select_all(&mut h, 65).await;
    h.key("ctrl+d").await;
    h.expect("dialog", "BulkReview").await;
    h.key("Escape").await;
    h.expect("total", 65).await;
    f.assert_ids("INBOX", f.ids.clone()).await?;
    h.key("BackSpace").await;
    confirm(&mut h, 65).await;
    h.expect("total", 0).await;
    settled(&mut h).await;
    f.assert_ids("Archive", f.ids.clone()).await?;
    f.assert_ids("INBOX", []).await?;
    h.click_at(1358., 36.).await;
    h.expect("dialog", "BulkHistory").await;
    h.expect("bulk.history_loading", false).await;
    h.click_text("Archive · 65 messages").await;
    h.expect_ne("bulk.selected_job", Value::Null).await;
    h.click_text("Undo").await;
    h.key("Escape").await;
    h.expect("total", 65).await;
    settled(&mut h).await;
    f.assert_ids("Archive", []).await?;
    f.assert_ids("INBOX", f.ids.clone()).await?;
    h.close().await;
    Ok(())
}

async fn bulk_flag_changes_only_reviewed_search_results() -> anyhow::Result<()> {
    let f = Fixture::new(22).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, 22).await?;
    search(&mut h, "Deterministic", 21).await;
    select_all(&mut h, 21).await;
    h.key("s").await;
    confirm(&mut h, 21).await;
    settled(&mut h).await;
    let messages = f.server.messages("alice", "INBOX").await?;
    let flagged: BTreeSet<_> = messages
        .into_iter()
        .filter(|m| m.flags.contains(&"\\Flagged".into()))
        .map(|m| m.message_id)
        .collect();
    assert_eq!(flagged, f.ids[1..].iter().cloned().collect());
    f.assert_ids("INBOX", f.ids.clone()).await?;
    h.close().await;
    Ok(())
}

async fn draft_survives_screens_and_restart() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    compose(
        &mut h,
        "Persistent draft",
        "Keep every word while navigating.",
    )
    .await;
    h.key("Escape").await;
    h.expect("draft_count", 1).await;
    for key in ["ctrl+2", "ctrl+comma", "ctrl+1"] {
        h.key(key).await;
    }
    h.click_text("Persistent draft").await;
    h.expect("compose_fields.to", "bob@shep.test").await;
    h.expect("editor", "Keep every word while navigating.")
        .await;
    h.close().await;
    let mut h = f.start().await?;
    h.expect("draft_count", 1).await;
    h.click_text("Persistent draft").await;
    h.expect("editor", "Keep every word while navigating.")
        .await;
    h.expect("compose_fields.subject", "Persistent draft").await;
    assert!(f.server.messages("bob", "INBOX").await?.is_empty());
    h.close().await;
    Ok(())
}

async fn smtp_delivery_and_sent_copy_survive_restart_without_resend() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    compose(
        &mut h,
        "Real SMTP submission",
        "Delivered through the production transport.",
    )
    .await;
    h.click_text("Send").await;
    h.expect("composer.visible", false).await;
    h.expect_contains("notice", "Message sent").await;
    h.expect("outgoing_pending", 0).await;
    h.expect("draft_count", 0).await;
    let received = f.server.messages("bob", "INBOX").await?;
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].subject, "Real SMTP submission");
    let parsed = mailparse::parse_mail(&received[0].raw)?;
    assert!(
        parsed
            .get_body()?
            .contains("Delivered through the production transport.")
    );
    let sent = f.server.messages("alice", "Sent").await?;
    assert_eq!(sent.len(), 1);
    assert_eq!(received[0].message_id, sent[0].message_id);
    h.close().await;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    h.expect("outgoing_pending", 0).await;
    assert_eq!(f.server.messages("bob", "INBOX").await?.len(), 1);
    assert_eq!(f.server.messages("alice", "Sent").await?.len(), 1);
    h.close().await;
    Ok(())
}

async fn reply_keeps_thread_identity_on_server() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    search(&mut h, "0003", 1).await;
    h.key("r").await;
    h.expect("composer.visible", true).await;
    h.expect("compose_fields.subject", "Re: Server message 0003")
        .await;
    h.expect("draft_in_reply_to", f.ids[3].clone()).await;
    field(&mut h, "to", "bob@shep.test").await;
    h.click_id("compose-body").await;
    h.type_text("Here is the reply.").await;
    h.click_text("Send").await;
    h.expect("composer.visible", false).await;
    h.expect_contains("notice", "Message sent").await;
    h.expect("outgoing_pending", 0).await;
    let received = f.server.messages("bob", "INBOX").await?;
    assert_eq!(received.len(), 1);
    assert_ne!(received[0].message_id, f.ids[3]);
    let parsed = mailparse::parse_mail(&received[0].raw)?;
    assert_eq!(
        parsed.headers.get_first_value("In-Reply-To"),
        Some(f.ids[3].clone())
    );
    assert!(
        parsed
            .headers
            .get_first_value("References")
            .context("reply references")?
            .contains(&f.ids[3])
    );
    h.close().await;
    Ok(())
}

async fn forwarding_preserves_attachment_bytes() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    search(&mut h, "0000", 1).await;
    h.key("f").await;
    h.expect("composer.visible", true).await;
    h.expect("compose_fields.subject", "Fwd: Server message 0000")
        .await;
    h.expect("draft_attachments.0.name", "evidence.txt").await;
    h.expect("draft_in_reply_to", Value::Null).await;
    h.expect("compose_fields.to", "").await;
    field(&mut h, "to", "bob@shep.test").await;
    h.click_text("Send").await;
    h.expect("composer.visible", false).await;
    h.expect_contains("notice", "Message sent").await;
    h.expect("outgoing_pending", 0).await;
    let received = f.server.messages("bob", "INBOX").await?;
    assert_eq!(received.len(), 1);
    let original = f.server.messages("alice", "INBOX").await?;
    let original = original
        .iter()
        .find(|m| m.message_id == f.ids[0])
        .context("forward source")?;
    assert_ne!(received[0].message_id, original.message_id);
    assert_eq!(received[0].subject, "Fwd: Server message 0000");
    let source = mailparse::parse_mail(&original.raw)?;
    let target = mailparse::parse_mail(&received[0].raw)?;
    assert_eq!(target.headers.get_first_value("In-Reply-To"), None);
    fn attachment(mail: &mailparse::ParsedMail<'_>) -> Option<Vec<u8>> {
        if mail
            .get_content_disposition()
            .params
            .get("filename")
            .is_some_and(|n| n == "evidence.txt")
        {
            return mail.get_body_raw().ok();
        }
        mail.subparts.iter().find_map(attachment)
    }
    assert!(attachment(&source).is_some());
    assert_eq!(attachment(&source), attachment(&target));
    h.close().await;
    Ok(())
}

macro_rules! real_scenarios {
    ($($name:ident),+ $(,)?) => { $(
        #[test]
        #[ignore = "owned Docker mail server; run python3 scripts/test_real_mail.py"]
        fn $name() {
            harness::run(|| async {
                let start = Instant::now();
                super::$name().await.expect(stringify!($name));
                eprintln!("{} finished in {:?}", stringify!($name), start.elapsed());
            });
        }
    )+ };
}

mod scenarios {
    use super::*;
    real_scenarios!(
        receive_search_and_keyboard_focus,
        read_on_leave_and_flag_are_saved_on_server,
        archive_trash_and_undo_keep_exact_identity,
        move_dialog_cancel_and_commit,
        bulk_cancel_and_cross_page_archive_undo,
        bulk_flag_changes_only_reviewed_search_results,
        draft_survives_screens_and_restart,
        smtp_delivery_and_sent_copy_survive_restart_without_resend,
        reply_keeps_thread_identity_on_server,
        forwarding_preserves_attachment_bytes,
        real_authentication_and_pop3_download,
        create_move_and_delete_folder_through_controls,
        rapid_flags_preserve_the_last_choice,
        rapid_archive_trash_and_undo_preserve_membership,
        repeated_send_clicks_deliver_once,
        repeated_refresh_and_screen_switches_preserve_draft,
        search_and_reader_survive_rapid_screen_switches,
        held_server_keeps_navigation_and_draft_saving_responsive,
        slow_large_download_keeps_cached_mail_usable,
        initial_backlog_keeps_first_screen_and_composer_usable,
        first_download_preserves_active_composer_and_focus,
        new_mail_arrives_within_five_seconds_without_refresh,
        new_mail_does_not_replace_active_editor,
        compact_dark_controls_use_the_same_real_server,
    );
}
