//! Preferences, settings search and appearance scenarios. Each one mirrors the
//! named native scenario in `scripts/e2e.py`, using the same coordinates.
use super::*;
use serde_json::Value::Null;

async fn open_preferences(h: &mut Harness) {
    h.key("ctrl+comma").await;
    h.expect("tab", "Preferences").await;
}

/// Types `query` into the settings search, like the native `search_setting`.
async fn search_setting(h: &mut Harness, query: &str, section: &str, control: Option<&str>) {
    h.click_at(1150., 88.).await;
    h.key("ctrl+a").await;
    h.type_text(query).await;
    h.expect("settings_search", query).await;
    h.expect("settings_matches.0", section).await;
    h.expect("settings_match_controls.0", control).await;
}

/// `test_preferences_search_and_tooltip_options`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preferences_search_and_tooltip_options() {
    let mut h = Harness::start().await;
    open_preferences(&mut h).await;
    h.click_at(1150., 88.).await;
    h.type_text("tooltip").await;
    h.expect("settings_matches", serde_json::json!(["Tooltips"]))
        .await;
    h.click_at(500., 289.).await;
    h.expect("settings_group", "Tooltips").await;
    h.click_at(288., 342.).await;
    h.expect("tooltips", false).await;
    h.expect("preferences_saved", true).await;
    h.key("ctrl+1").await;
    h.expect("tab", "Mail").await;
    open_preferences(&mut h).await;
    h.click_at(288., 342.).await;
    h.expect("tooltips", true).await;
    h.click_at(288., 379.).await;
    h.expect("shortcut_tooltips", false).await;
    h.expect("preferences_saved", true).await;
    h.key("ctrl+1").await;
    h.expect("tab", "Mail").await;
    open_preferences(&mut h).await;
    h.click_at(288., 379.).await;
    h.expect("shortcut_tooltips", true).await;
    h.expect("preferences_saved", true).await;
}

/// The compact dark half of `test_preferences_search_and_tooltip_options`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preferences_search_compact_dark() {
    let mut h = Harness::with_size(900., 640.).await;
    open_preferences(&mut h).await;
    h.click_at(563., 366.).await;
    h.expect("dark", true).await;
    h.click_at(650., 88.).await;
    h.type_text("font").await;
    h.expect(
        "settings_matches",
        serde_json::json!(["Reading and layout"]),
    )
    .await;
    h.click_at(450., 289.).await;
    h.expect("settings_group", "Reading and layout").await;
}

/// `test_preferences_catalogue_ranking_and_cross_tab_navigation`, full-size light.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preferences_catalogue_ranking_and_cross_tab_navigation() {
    let mut h = Harness::start().await;
    open_preferences(&mut h).await;
    for (query, section) in [
        ("apperance", "Appearance"),
        ("dark mode", "Appearance"),
        ("mail check interval", "Mail & performance"),
        ("reply history", "Reading and layout"),
        ("synced passwords", "Profiles and sync"),
        ("retention", "Backups"),
        ("sftp fingerprint", "Backups"),
        ("S3 region", "Backups"),
        ("FTP security", "Backups"),
        ("SMTP username", "Your accounts"),
    ] {
        h.click_at(1150., 88.).await;
        h.key("ctrl+a").await;
        h.type_text(query).await;
        h.expect("settings_matches.0", section).await;
        h.expect("dark", false).await;
    }
    h.click_at(1150., 88.).await;
    h.key("ctrl+a").await;
    h.type_text("appearance").await;
    h.expect("settings_matches.0", "Appearance").await;
    h.expect_contains("settings_matches", "Profiles and sync")
        .await;
    h.click_at(450., 289.).await;
    h.expect("settings_group", "Appearance").await;
    h.expect("settings_search", "").await;
    h.click_at(1150., 88.).await;
    h.type_text("qzxvjkwp").await;
    h.expect("settings_matches", serde_json::json!([])).await;
}

/// `test_settings_search_reveals_and_focuses_individual_controls`, without the
/// outline pixel checks, which stay native.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_search_reveals_and_focuses_individual_controls() {
    let mut h = Harness::start().await;
    open_preferences(&mut h).await;
    search_setting(
        &mut h,
        "check for new mail",
        "Mail & performance",
        Some("Check for new mail"),
    )
    .await;
    h.click_at(480., 289.).await;
    h.expect("settings_group", "Mail & performance").await;
    h.expect("settings_search", "").await;
    h.expect("settings_reveal.state", "revealed").await;
    h.expect("settings_reveal.focused", true).await;
    h.expect_ne("settings_reveal.outline", Null).await;
    h.key("ctrl+a").await;
    h.type_text("30").await;
    h.click_at(1352., 88.).await;
    h.expect("mail_check_seconds", 30).await;
    h.expect("preferences_saved", true).await;
    search_setting(
        &mut h,
        "print message",
        "Keyboard shortcuts",
        Some("Print message"),
    )
    .await;
    h.expect("settings_reveal", Null).await;
    h.click_at(480., 289.).await;
    h.expect("settings_group", "Keyboard shortcuts").await;
    h.expect("settings_tab", "Shortcuts").await;
    h.expect("settings_reveal.state", "revealed").await;
    h.expect("settings_reveal.focused", false).await;
    h.expect_that("settings_reveal.top", |top| {
        top.as_f64()
            .is_some_and(|top| (200. ..=880.).contains(&top))
    })
    .await;
    search_setting(
        &mut h,
        "clear image exceptions",
        "Privacy",
        Some("Clear image exceptions"),
    )
    .await;
    h.click_at(480., 289.).await;
    h.expect("settings_tab", "Privacy").await;
    h.expect("settings_reveal.state", "revealed").await;
    search_setting(
        &mut h,
        "shared profile",
        "Profiles and sync",
        Some("Check for shared profiles after Google sign-in"),
    )
    .await;
    h.click_at(480., 289.).await;
    h.expect("settings_tab", "Accounts").await;
    h.expect("settings_reveal.state", "revealed").await;
    search_setting(&mut h, "backups", "Backups", None).await;
    h.click_at(480., 289.).await;
    h.expect("settings_group", "Backups").await;
    h.expect("settings_reveal", Null).await;
    h.click_at(1150., 88.).await;
    h.type_text("qzxvjkwp").await;
    h.expect("settings_matches", serde_json::json!([])).await;
    h.expect("settings_match_controls", serde_json::json!([]))
        .await;
}

/// `test_preferences_and_resize_keep_latest_changes`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn preferences_and_resize_keep_latest_changes() {
    let mut h = Harness::start().await;
    open_preferences(&mut h).await;
    h.click_at(690., 366.).await;
    h.expect("dark", true).await;
    h.click_at(286., 737.).await;
    h.expect("unified", false).await;
    h.click_at(286., 773.).await;
    h.expect("cross_account_moves", true).await;
    h.key("ctrl+1").await;
    h.expect("tab", "Mail").await;
    h.drag((616., 500.), (785., 500.)).await;
    h.expect_at_least("reader_split", 0.44).await;
    open_preferences(&mut h).await;
    h.click_at(399., 366.).await;
    h.expect("dark", false).await;
    h.expect("preferences_saved", true).await;
    h.expect("saved_appearance", "Light").await;
    h.expect_at_least("saved_reader_split", 0.44).await;
    h.expect("unified", false).await;
    h.expect("cross_account_moves", true).await;
    h.key("ctrl+1").await;
    h.expect("tab", "Mail").await;
    h.expect_at_least("reader_split", 0.44).await;
}

/// Appearance changes repaint the window and switching back restores identical
/// pixels. Compares compact `Simulator` snapshots from this run only.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn appearance_round_trip_restores_identical_pixels() {
    let directory = tempfile::tempdir().expect("snapshot directory");
    let light = directory.path().join("preferences-light");
    let mut h = Harness::with_size(900., 640.).await;
    open_preferences(&mut h).await;
    // Start from the saved state; its subtitle differs from a fresh window.
    h.click_text("Dark").await;
    h.expect("dark", true).await;
    h.click_text("Light").await;
    h.expect("dark", false).await;
    h.expect("preferences_saved", true).await;
    assert!(
        h.snapshot()
            .matches_hash(&light)
            .expect("first light snapshot")
    );
    h.click_text("Dark").await;
    h.expect("dark", true).await;
    h.expect("preferences_saved", true).await;
    assert!(!h.snapshot().matches_hash(&light).expect("dark snapshot"));
    h.click_text("Light").await;
    h.expect("dark", false).await;
    h.expect("preferences_saved", true).await;
    assert!(
        h.snapshot()
            .matches_hash(&light)
            .expect("second light snapshot")
    );
}
