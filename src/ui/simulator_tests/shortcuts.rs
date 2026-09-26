//! Shortcut remapping, conflict and disabling scenarios. Each one mirrors the
//! named native scenario in `scripts/e2e.py`, using the same coordinates.
use super::*;
use serde_json::Value::Null;

async fn open_shortcuts(h: &mut Harness) {
    h.key("ctrl+comma").await;
    h.expect("tab", "Preferences").await;
    h.click_at(645., 156.).await;
    h.expect("settings_tab", "Shortcuts").await;
}

/// `test_remapping_persists_and_works`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remapping_persists_and_works() {
    let mut h = Harness::start().await;
    open_shortcuts(&mut h).await;
    h.click_at(946., 350.).await;
    h.key("alt+m").await;
    h.expect("shortcuts.Move", "Alt+M").await;
    h.expect("preferences_saved", true).await;
    h.key("ctrl+1").await;
    h.expect("tab", "Mail").await;
    h.key("m").await;
    h.expect("dialog", Null).await;
    h.key("alt+m").await;
    h.expect("dialog", "Move").await;
    h.key("Escape").await;
    h.expect("dialog", Null).await;
}

/// `test_secondary_shortcut_remap_conflict_and_disable`, full-size part.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn secondary_shortcut_remap_conflict_and_disable() {
    let mut h = Harness::start().await;
    open_shortcuts(&mut h).await;
    h.click_at(1100., 350.).await;
    h.key("Delete").await;
    h.expect_contains("notice", "assigned more than once").await;
    h.key("alt+m").await;
    h.expect("shortcut_secondary.Move", "Alt+M").await;
    h.expect("preferences_saved", true).await;
    h.key("ctrl+1").await;
    h.expect("tab", "Mail").await;
    h.key("alt+m").await;
    h.expect("dialog", "Move").await;
    h.expect("focused_input", "folder-search").await;
    h.key("Escape").await;
    h.expect("dialog", Null).await;
    h.key("m").await;
    h.expect("dialog", "Move").await;
    h.expect("focused_input", "folder-search").await;
    h.key("Escape").await;
    h.expect("dialog", Null).await;
    open_shortcuts(&mut h).await;
    h.click_at(1157., 353.).await;
    h.expect("shortcut_secondary.Move", "").await;
    h.expect("preferences_saved", true).await;
}

/// `test_shortcut_clear_primary_secondary_and_cancel_capture`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shortcut_clear_primary_secondary_and_cancel_capture() {
    let mut h = Harness::start().await;
    open_shortcuts(&mut h).await;
    h.click_at(988., 350.).await;
    h.expect("shortcuts.Move", "").await;
    h.expect("preferences_saved", true).await;
    h.click_at(1070., 350.).await;
    h.key("alt+m").await;
    h.expect("shortcut_secondary.Move", "Alt+M").await;
    h.click_at(1157., 350.).await;
    h.expect("shortcut_secondary.Move", "").await;
    h.expect("preferences_saved", true).await;
    // Starting a capture and clearing instead cancels it; a later key is ignored.
    h.click_at(905., 350.).await;
    h.click_at(988., 350.).await;
    h.key("q").await;
    h.expect("shortcuts.Move", "").await;
    h.key("ctrl+1").await;
    h.expect("tab", "Mail").await;
    h.key("m").await;
    h.expect("dialog", Null).await;
    h.key("ctrl+comma").await;
    h.expect("tab", "Preferences").await;
    h.click_at(905., 350.).await;
    h.key("m").await;
    h.expect("shortcuts.Move", "M").await;
    h.expect("preferences_saved", true).await;
    h.key("ctrl+1").await;
    h.key("m").await;
    h.expect("dialog", "Move").await;
    h.key("Escape").await;
}
