use super::*;

fn p95(samples: &mut [Duration]) -> Duration {
    assert!(!samples.is_empty());
    samples.sort();
    samples[(samples.len() * 95).div_ceil(100) - 1]
}

fn assert_snappy(f: &Fixture, samples: &mut [Duration], name: &str) -> anyhow::Result<()> {
    let measured = p95(samples);
    std::fs::write(
        f.directory.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&json!({
            "measurement": "headless input through widget, App update and layout; not presented pixels",
            "samples_ms": samples.iter().map(Duration::as_secs_f64).map(|s| s * 1000.).collect::<Vec<_>>(),
            "p95_ms": measured.as_secs_f64() * 1000., "budget_ms": INPUT_P95_BUDGET.as_millis(),
        }))?,
    )?;
    assert!(
        measured <= INPUT_P95_BUDGET,
        "{name}: input p95 {measured:?} exceeds {INPUT_P95_BUDGET:?}"
    );
    Ok(())
}

pub(super) async fn real_authentication_and_pop3_download() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    assert!(f.server.rejects_wrong_password().await?);
    f.store()?
        .save_account(f.server.pop3_account("alice"))
        .await?;
    let mut h = f.start().await?;
    h.expect("total", MAILS).await;
    startup_finished(&mut h).await;
    h.expect("background_sync", false).await;
    settled(&mut h).await;
    search(&mut h, "0003", 1).await;
    h.expect("selected", "Server message 0003").await;
    h.key("ctrl+r").await;
    h.expect("refreshing", false).await;
    search(&mut h, "", MAILS).await;
    f.assert_ids("INBOX", f.ids.clone()).await?;
    h.close().await;
    let mut h = f.start().await?;
    h.expect("total", MAILS).await;
    startup_finished(&mut h).await;
    h.expect("background_sync", false).await;
    settled(&mut h).await;
    h.close().await;
    Ok(())
}

pub(super) async fn create_move_and_delete_folder_through_controls() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    h.click_text("New folder").await;
    h.expect("dialog", "FolderCreation").await;
    h.expect("focused_input", "new-folder-name").await;
    h.click_id("new-folder-name").await;
    h.key("Return").await;
    h.expect_contains("folder_creation.error", "Enter a folder name")
        .await;
    h.type_text("Receipts").await;
    h.key("Return").await;
    h.expect("dialog", Value::Null).await;
    h.expect_contains("sidebar_labels", "Receipts").await;
    h.expect("folder_creation.saved", json!([])).await;
    assert!(
        f.server
            .folders("alice")
            .await?
            .contains(&"Receipts".into())
    );
    let row = h.text_center("Receipts");
    h.right_click_at(row.x, row.y).await;
    h.click_text("Move folder…").await;
    h.expect("focused_input", "folder-parent-search").await;
    h.type_text("Projects").await;
    h.key("Return").await;
    h.expect("folder_changes.review.folders", 1).await;
    h.click_text_nth("Move folder", 1).await;
    h.expect("dialog", Value::Null).await;
    h.expect("folder_changes.jobs.0.status", "Completed").await;
    let folders = f.server.folders("alice").await?;
    assert!(!folders.contains(&"Receipts".into()));
    assert!(folders.contains(&"Projects.Receipts".into()));
    // Delete another visible, empty folder, after first cancelling its review.
    let row = h.text_center("Archive");
    h.right_click_at(row.x, row.y).await;
    h.click_text("Delete folder…").await;
    h.expect("folder_changes.review.folders", 1).await;
    h.click_text("Cancel").await;
    assert!(f.server.folders("alice").await?.contains(&"Archive".into()));
    let row = h.text_center("Archive");
    h.right_click_at(row.x, row.y).await;
    h.click_text("Delete folder…").await;
    h.expect("folder_changes.review.folders", 1).await;
    h.click_text("Delete folder").await;
    h.expect("dialog", Value::Null).await;
    h.expect("folder_changes.jobs.0.source", "Archive").await;
    h.expect("folder_changes.jobs.0.status", "Completed").await;
    assert!(!f.server.folders("alice").await?.contains(&"Archive".into()));
    f.assert_ids("INBOX", f.ids.clone()).await?;
    h.close().await;
    Ok(())
}

pub(super) async fn rapid_flags_preserve_the_last_choice() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    search(&mut h, "0003", 1).await;
    let mut timings = Vec::new();
    for index in 0..21 {
        let start = Instant::now();
        if index % 2 == 0 {
            h.click_at(784., 100.).await;
        } else {
            h.key("s").await;
        }
        timings.push(start.elapsed());
        assert_eq!(
            h.state()["starred"],
            index % 2 == 0,
            "every flag input took effect"
        );
    }
    h.expect("starred", true).await;
    settled(&mut h).await;
    let mail = f.server.messages("alice", "INBOX").await?;
    assert!(
        mail.iter()
            .find(|m| m.message_id == f.ids[3])
            .context("spam target")?
            .flags
            .contains(&"\\Flagged".into())
    );
    f.assert_ids("INBOX", f.ids.clone()).await?;
    assert_snappy(&f, &mut timings, "rapid-flags")?;
    h.close().await;
    Ok(())
}

pub(super) async fn rapid_archive_trash_and_undo_preserve_membership() -> anyhow::Result<()> {
    let f = Fixture::new(24).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, 24).await?;
    for (key, destination) in [("BackSpace", "Archive"), ("ctrl+d", "Trash")] {
        let rows = h.state()["mail_rows"]
            .as_array()
            .context("initial rows")?
            .clone();
        h.click_text(rows[0]["subject"].as_str().context("first subject")?)
            .await;
        h.expect("loaded_message_id", rows[0]["id"].clone()).await;
        for index in 0..12 {
            h.key(key).await;
            assert_eq!(
                h.state()["total"],
                23 - index,
                "each rapid action targets a different row"
            );
            assert_eq!(h.state()["selected_id"], rows[index + 1]["id"]);
        }
        assert_eq!(h.state()["action_toast"]["count"], 12);
        h.click_text("Undo").await;
        h.expect("total", 24).await;
        settled(&mut h).await;
        f.assert_ids("INBOX", f.ids.clone()).await?;
        f.assert_ids(destination, []).await?;
    }
    h.close().await;
    let mut h = f.start().await?;
    f.loaded(&mut h, 24).await?;
    h.close().await;
    Ok(())
}

pub(super) async fn repeated_send_clicks_deliver_once() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    compose(
        &mut h,
        "One submission",
        "A repeated press must keep one outgoing identity.",
    )
    .await;
    let send = h.text_center("Send");
    for _ in 0..20 {
        h.click(send).await;
    }
    h.expect("composer.visible", false).await;
    h.expect_contains("notice", "Message sent").await;
    h.expect("outgoing_pending", 0).await;
    h.expect("draft_count", 0).await;
    let received = f.server.messages("bob", "INBOX").await?;
    assert_eq!(received.len(), 1, "repeated Send delivered more than once");
    assert_eq!(received[0].subject, "One submission");
    h.close().await;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    assert_eq!(f.server.messages("bob", "INBOX").await?.len(), 1);
    h.close().await;
    Ok(())
}

pub(super) async fn repeated_refresh_and_screen_switches_preserve_draft() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    compose(&mut h, "Keep this editor", "Saved before navigation.").await;
    h.key("Escape").await;
    h.expect("draft_count", 1).await;
    for _ in 0..12 {
        h.click_at(1400., 36.).await;
        h.key("ctrl+2").await;
        assert_eq!(h.state()["tab"], "Calendar");
        h.key("ctrl+comma").await;
        assert_eq!(h.state()["tab"], "Preferences");
        h.key("ctrl+1").await;
        assert_eq!(h.state()["tab"], "Mail");
    }
    h.click_text("Keep this editor").await;
    h.expect("editor", "Saved before navigation.").await;
    h.expect("compose_fields.to", "bob@shep.test").await;
    h.expect("draft_count", 1).await;
    h.key("Escape").await;
    h.expect("refreshing", false).await;
    f.loaded(&mut h, MAILS).await?;
    f.assert_ids("INBOX", f.ids.clone()).await?;
    h.close().await;
    Ok(())
}

pub(super) async fn search_and_reader_survive_rapid_screen_switches() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    search(&mut h, "0003", 1).await;
    let selected = h.state()["selected_id"].clone();
    for _ in 0..12 {
        for (key, tab) in [
            ("ctrl+2", "Calendar"),
            ("ctrl+comma", "Preferences"),
            ("ctrl+1", "Mail"),
        ] {
            h.key(key).await;
            assert_eq!(h.state()["tab"], tab);
        }
        assert_eq!(h.state()["query"], "0003");
        assert_eq!(h.state()["total"], 1);
        assert_eq!(h.state()["selected_id"], selected);
        h.expect("loaded_message_id", selected.clone()).await;
        h.expect("selected", "Server message 0003").await;
    }
    settled(&mut h).await;
    f.assert_ids("INBOX", f.ids.clone()).await?;
    h.close().await;
    Ok(())
}

pub(super) async fn held_server_keeps_navigation_and_draft_saving_responsive() -> anyhow::Result<()>
{
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    f.server.hold_imap();
    h.key("ctrl+r").await;
    h.expect("refreshing", true).await;
    h.expect_state(
        |_| f.server.imap_held_responses() > 0,
        "a real IMAP reply is blocked",
    )
    .await;
    let mut timings = Vec::new();
    for _ in 0..8 {
        for (key, tab) in [
            ("ctrl+2", "Calendar"),
            ("ctrl+comma", "Preferences"),
            ("ctrl+1", "Mail"),
        ] {
            let start = Instant::now();
            h.key(key).await;
            timings.push(start.elapsed());
            assert_eq!(
                h.state()["tab"],
                tab,
                "navigation must finish without server progress"
            );
        }
        h.key("ctrl+r").await;
    }
    compose(
        &mut h,
        "Saved while offline",
        "Local storage does not need IMAP.",
    )
    .await;
    h.key("Escape").await;
    h.expect("draft_count", 1).await;
    h.expect("draft_body", "Local storage does not need IMAP.")
        .await;
    assert!(f.server.imap_held_responses() > 0);
    assert_eq!(h.state()["refreshing"], true);
    assert_snappy(&f, &mut timings, "held-server-navigation")?;
    f.capture(&h, "held-server-draft")?;
    f.server.release_imap();
    h.expect("refreshing", false).await;
    f.loaded(&mut h, MAILS).await?;
    h.close().await;
    let mut h = f.start().await?;
    h.expect("draft_count", 1).await;
    h.click_text("Saved while offline").await;
    h.expect("editor", "Local storage does not need IMAP.")
        .await;
    h.close().await;
    Ok(())
}

pub(super) async fn slow_large_download_keeps_cached_mail_usable() -> anyhow::Result<()> {
    let f = Fixture::new(65).await?;
    let mut h = f.start().await?;
    f.loaded(&mut h, 65).await?;
    let first = h.state()["mail_rows"][0].clone();
    h.click_text(first["subject"].as_str().context("first cached subject")?)
        .await;
    h.expect("loaded_message_id", first["id"].clone()).await;
    f.server.set_imap_delay(Duration::from_millis(20));
    let raw = RealServer::large_message(1000, 3 * 1024 * 1024);
    f.server.deliver("alice", &raw).await?;
    let before = f.server.imap_forwarded_bytes();
    h.key("ctrl+r").await;
    h.expect_state(
        |s| s["refreshing"] == true && f.server.imap_forwarded_bytes() > before + 64 * 1024,
        "large message download is transferring real bytes",
    )
    .await;
    let mut timings = Vec::new();
    for _ in 0..8 {
        for (key, tab) in [
            ("ctrl+2", "Calendar"),
            ("ctrl+comma", "Preferences"),
            ("ctrl+1", "Mail"),
        ] {
            let start = Instant::now();
            h.key(key).await;
            assert_eq!(h.state()["tab"], tab);
            timings.push(start.elapsed());
        }
        for (key, delta) in [("j", 1isize), ("k", -1)] {
            let state = h.state();
            let rows = state["mail_rows"].as_array().context("cached rows")?;
            let selected = rows
                .iter()
                .position(|row| row["id"] == state["selected_id"])
                .context("selected cached row")?;
            let expected =
                rows[selected.checked_add_signed(delta).context("neighbour")?]["id"].clone();
            let start = Instant::now();
            h.key(key).await;
            assert_eq!(h.state()["selected_id"], expected);
            h.expect_within(
                |s| s["loaded_message_id"] == expected,
                "cached reader during download",
                INPUT_P95_BUDGET,
            )
            .await;
            timings.push(start.elapsed());
        }
    }
    assert!(h.state()["refreshing"] == true || h.state()["background_sync"] == true);
    assert_eq!(
        h.state()["total"],
        65,
        "input overlapped the unfinished download"
    );
    assert_snappy(&f, &mut timings, "large-download-input")?;
    f.server.set_imap_delay(Duration::ZERO);
    h.expect("total", 66).await;
    h.expect("refreshing", false).await;
    f.loaded(&mut h, 66).await?;
    assert!(
        h.state()["mail_rows"]
            .as_array()
            .context("metadata page")?
            .len()
            <= 50
    );
    assert!(h.state()["cache_entries"].as_u64().context("body cache")? <= 8);
    h.close().await;
    Ok(())
}

pub(super) async fn initial_backlog_keeps_first_screen_and_composer_usable() -> anyhow::Result<()> {
    let f = Fixture::new(120).await?;
    f.server
        .deliver("alice", &RealServer::large_message(1001, 3 * 1024 * 1024))
        .await?;
    f.server.set_imap_delay(Duration::from_millis(20));
    let mut h = f.start().await?;
    h.expect("background_sync", true).await;
    h.expect_state(
        |s| s["background_sync"] == true && f.server.imap_forwarded_bytes() > 64 * 1024,
        "initial backlog is transferring real mail bytes",
    )
    .await;
    let mut timings = Vec::new();
    for _ in 0..8 {
        for (key, tab) in [
            ("ctrl+2", "Calendar"),
            ("ctrl+comma", "Preferences"),
            ("ctrl+1", "Mail"),
        ] {
            let start = Instant::now();
            h.key(key).await;
            timings.push(start.elapsed());
            assert_eq!(h.state()["tab"], tab);
        }
    }
    compose(
        &mut h,
        "First download draft",
        "Composing need not wait for the Inbox.",
    )
    .await;
    h.key("Escape").await;
    h.expect("draft_count", 1).await;
    assert_eq!(h.state()["background_sync"], true);
    assert!(h.state()["total"].as_u64().context("initial total")? < 121);
    assert_snappy(&f, &mut timings, "initial-backlog-input")?;
    f.server.set_imap_delay(Duration::ZERO);
    f.loaded(&mut h, 121).await?;
    h.click_text("First download draft").await;
    h.expect("editor", "Composing need not wait for the Inbox.")
        .await;
    h.close().await;
    Ok(())
}

async fn arrival_setup() -> anyhow::Result<(Fixture, Harness)> {
    let f = Fixture::new(MAILS).await?;
    let store = f.store()?;
    let mut preferences: Preferences = store.get("preferences").await?;
    // Make the five-second assertion distinguish push from the ordinary poll.
    preferences.mail_check_seconds = 3600;
    store.put("preferences", preferences).await?;
    drop(store);
    let mut h = f.start().await?;
    f.loaded(&mut h, MAILS).await?;
    h.expect_state(
        |_| f.server.imap_connections() >= 2,
        "separate IMAP watcher connected",
    )
    .await;
    Ok((f, h))
}

pub(super) async fn first_download_preserves_active_composer_and_focus() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    f.server.hold_imap();
    let mut h = f.start().await?;
    assert_eq!(h.state()["total"], 0);
    h.expect_state(
        |_| f.server.imap_held_responses() > 0,
        "first mail download is held before any rows arrive",
    )
    .await;
    h.key("c").await;
    h.expect("composer.visible", true).await;
    h.expect("focused_input", "to").await;
    h.type_text("bob@shep.test").await;
    field(&mut h, "subject", "Keep typing during the first download").await;
    let id = h.state()["composer"]["id"].clone();
    assert!(!id.as_str().context("composer identity")?.is_empty());
    f.server.release_imap();
    h.expect("total", MAILS).await;
    assert_eq!(
        h.state()["composer"]["visible"],
        true,
        "first mail page closed the active composer"
    );
    assert_eq!(h.state()["composer"]["id"], id);
    assert_eq!(h.state()["compose_fields"]["to"], "bob@shep.test");
    assert_eq!(h.state()["native_focus"], "subject");
    h.type_text(" without losing focus").await;
    h.expect(
        "compose_fields.subject",
        "Keep typing during the first download without losing focus",
    )
    .await;
    f.capture(&h, "composer-kept-during-first-download")?;
    h.key("Escape").await;
    h.expect("draft_count", 1).await;
    f.loaded(&mut h, MAILS).await?;
    h.close().await;
    let mut h = f.start().await?;
    h.expect("draft_count", 1).await;
    h.click_text("Keep typing during the first download without losing focus")
        .await;
    h.expect("compose_fields.to", "bob@shep.test").await;
    h.expect(
        "compose_fields.subject",
        "Keep typing during the first download without losing focus",
    )
    .await;
    h.close().await;
    Ok(())
}

pub(super) async fn new_mail_arrives_within_five_seconds_without_refresh() -> anyhow::Result<()> {
    let (f, mut h) = arrival_setup().await?;
    let raw = RealServer::large_message(2000, 200);
    f.server.deliver("alice", &raw).await?;
    let start = Instant::now();
    h.expect_within(
        |s| {
            s["total"] == MAILS + 1
                && s["mail_rows"].as_array().is_some_and(|rows| {
                    rows.iter()
                        .any(|row| row["subject"] == "Large server message 2000")
                })
        },
        "new mail visible without Refresh",
        ARRIVAL_BUDGET,
    )
    .await;
    let elapsed = start.elapsed();
    std::fs::write(
        f.directory.join("arrival.json"),
        serde_json::to_vec_pretty(&json!({
            "arrival_ms": elapsed.as_secs_f64() * 1000., "budget_ms": ARRIVAL_BUDGET.as_millis(),
            "poll_seconds": 3600, "server": "GreenMail", "measurement": "SMTP acknowledgement to headless row observation"
        }))?,
    )?;
    f.loaded(&mut h, MAILS + 1).await?;
    h.close().await;
    Ok(())
}

pub(super) async fn new_mail_does_not_replace_active_editor() -> anyhow::Result<()> {
    let (f, mut h) = arrival_setup().await?;
    compose(
        &mut h,
        "Keep my current input",
        "Typing while another email arrives.",
    )
    .await;
    let id = h.state()["composer"]["id"].clone();
    f.server
        .deliver("alice", &RealServer::large_message(2001, 200))
        .await?;
    h.expect_within(
        |s| s["total"] == MAILS + 1,
        "arrival during editing",
        ARRIVAL_BUDGET,
    )
    .await;
    assert_eq!(h.state()["composer"]["id"], id);
    assert_eq!(
        h.state()["compose_fields"]["subject"],
        "Keep my current input"
    );
    assert_eq!(h.state()["editor"], "Typing while another email arrives.");
    h.type_text(" Still my draft.").await;
    h.expect(
        "editor",
        "Typing while another email arrives. Still my draft.",
    )
    .await;
    h.key("Escape").await;
    h.expect("draft_count", 1).await;
    f.loaded(&mut h, MAILS + 1).await?;
    h.close().await;
    Ok(())
}

pub(super) async fn compact_dark_controls_use_the_same_real_server() -> anyhow::Result<()> {
    let f = Fixture::new(MAILS).await?;
    let mut h = f.start_size(900., 640.).await?;
    f.loaded(&mut h, MAILS).await?;
    h.key("ctrl+comma").await;
    h.expect("tab", "Preferences").await;
    h.click_text("Dark").await;
    h.expect("dark", true).await;
    h.expect("preferences_saved", true).await;
    h.key("ctrl+1").await;
    search(&mut h, "0003", 1).await;
    h.key("m").await;
    h.expect("focused_input", "folder-search").await;
    h.type_text("Projects").await;
    h.key("Return").await;
    h.expect("mail_rows.0.folder", "Projects").await;
    settled(&mut h).await;
    f.assert_ids("Projects", [f.ids[3].clone()]).await?;
    h.click_text("Undo").await;
    h.expect("total", 1).await;
    h.expect("mail_rows.0.folder", "INBOX").await;
    settled(&mut h).await;
    f.assert_ids("INBOX", f.ids.clone()).await?;
    f.capture(&h, "compact-dark-mail")?;
    h.close().await;
    Ok(())
}
