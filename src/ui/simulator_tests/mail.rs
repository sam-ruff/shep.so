//! Mail list, Move dialog, drafts and dropdown scenarios. Each one mirrors the
//! named native scenario in `scripts/e2e.py`, using the same coordinates.
use super::*;
use serde_json::Value::Null;

/// `test_move_mouse_and_keyboard_and_typing_protection`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn move_mouse_and_keyboard_and_typing_protection() {
    let mut h = Harness::start().await;
    h.key("m").await;
    h.expect("dialog", "Move").await;
    h.key("Escape").await;
    h.expect("dialog", Null).await;
    h.key("ctrl+k").await;
    h.expect("focused_input", "search").await;
    h.type_text("m").await;
    h.expect("query", "m").await;
    h.expect("dialog", Null).await;
    h.key("ctrl+a").await;
    h.key("BackSpace").await;
    h.expect("query", "").await;
    h.expect("total", 120).await;
    h.expect("selected", "A little more room to think").await;
    h.key("Escape").await;
    h.key("m").await;
    h.expect("dialog", "Move").await;
    h.click_at(600., 374.).await;
    h.expect("dialog", Null).await;
    h.expect("total", 119).await;
    h.click_at(85., 398.).await;
    h.expect("folder", "Archive").await;
    h.expect("total", 1).await;
}

/// `test_delete_archive_defaults_and_mail_returns_to_inbox`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_archive_defaults_and_mail_returns_to_inbox() {
    let mut h = Harness::start().await;
    h.expect("shortcuts.Delete", "Mod+D").await;
    h.expect("shortcuts.Archive", "Backspace").await;
    h.expect("shortcut_secondary.Archive", "Delete").await;
    h.key("BackSpace").await;
    h.expect("total", 119).await;
    h.key("Delete").await;
    h.expect("total", 118).await;
    h.key("ctrl+d").await;
    h.expect("total", 117).await;
    h.click_at(85., 398.).await;
    h.expect("folder", "Archive").await;
    h.expect("total", 2).await;
    h.click_at(85., 115.).await;
    h.expect("folder", "INBOX").await;
    h.expect("total", 117).await;
    h.key("ctrl+k").await;
    h.expect("focused_input", "search").await;
    h.type_text("invoice").await;
    h.expect("query", "invoice").await;
    h.expect("total", 1).await;
    // Search focus keeps Ctrl+D as a text chord, not a mail shortcut.
    h.key("ctrl+d").await;
    h.expect("folder", "INBOX").await;
    h.expect("query", "invoice").await;
    h.expect("total", 1).await;
    h.key("ctrl+a").await;
    h.key("BackSpace").await;
    h.expect("total", 117).await;
    h.key("Escape").await;
    h.click_at(85., 438.).await;
    h.expect("folder", "Trash").await;
    h.expect("total", 1).await;
    h.click_at(85., 115.).await;
    h.expect("folder", "INBOX").await;
    h.expect("total", 117).await;
    h.key("ctrl+comma").await;
    h.expect("tab", "Preferences").await;
    h.click_at(286., 737.).await;
    h.expect("unified", false).await;
    h.key("ctrl+1").await;
    h.expect("tab", "Mail").await;
    h.click_at(85., 358.).await;
    h.expect("folder", "Archive").await;
    h.click_at(85., 115.).await;
    h.expect("folder", "INBOX").await;
    h.expect("account", "preview-work").await;
}

/// `test_search_mouse_focus_blocks_default_and_remapped_delete_chords`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_mouse_focus_blocks_default_and_remapped_delete_chords() {
    let mut h = Harness::start().await;
    h.click_at(415., 154.).await;
    h.type_text("invoice").await;
    h.expect("total", 1).await;
    h.key("ctrl+d").await;
    h.key("ctrl+a").await;
    h.key("BackSpace").await;
    h.expect("total", 120).await;
    h.expect("action_toast", Null).await;
    h.key("Escape").await;
    h.key("ctrl+comma").await;
    h.expect("tab", "Preferences").await;
    h.click_at(645., 156.).await;
    h.expect("settings_tab", "Shortcuts").await;
    h.hover(1200., 700.).await;
    h.scroll(30).await;
    h.click_at(920., 480.).await;
    h.key("alt+d").await;
    h.expect("shortcuts.Delete", "Alt+D").await;
    h.expect("preferences_saved", true).await;
    h.key("ctrl+1").await;
    h.expect("tab", "Mail").await;
    h.click_at(415., 154.).await;
    h.type_text("invoice").await;
    h.expect("total", 1).await;
    h.key("alt+d").await;
    h.key("ctrl+a").await;
    h.key("BackSpace").await;
    h.expect("total", 120).await;
    h.expect("action_toast", Null).await;
    h.key("Escape").await;
    h.key("alt+d").await;
    h.expect("total", 119).await;
    h.expect("action_toast.label", "Deleted 1 message").await;
}

/// `test_drafts_collapse_context_cancel_and_discard`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drafts_collapse_context_cancel_and_discard() {
    let mut h = Harness::start().await;
    h.key("c").await;
    h.expect("composer.visible", true).await;
    h.click_at(850., 279.).await;
    h.type_text("First draft to keep").await;
    h.expect("draft_count", 1).await;
    h.key("Escape").await;
    h.expect("composer.visible", false).await;
    h.expect("dialog", Null).await;
    h.key("c").await;
    h.expect("composer.visible", true).await;
    h.click_at(850., 279.).await;
    h.type_text("Second draft to discard").await;
    h.expect("draft_count", 2).await;
    h.key("Escape").await;
    h.expect("composer.visible", false).await;
    h.expect("dialog", Null).await;
    h.click_at(98., 516.).await;
    h.expect("drafts_collapsed", true).await;
    h.expect("saved_drafts_collapsed", true).await;
    h.key("ctrl+2").await;
    h.expect("tab", "Calendar").await;
    h.key("ctrl+1").await;
    h.expect("tab", "Mail").await;
    h.expect("drafts_collapsed", true).await;
    h.click_at(98., 516.).await;
    h.expect("drafts_collapsed", false).await;
    h.expect("saved_drafts_collapsed", false).await;
    // A background refresh must not dismiss the draft context menu.
    h.click_at(1400., 36.).await;
    h.expect_contains("busy", "sync").await;
    h.right_click_at(100., 592.).await;
    h.expect_ne("draft_context", Null).await;
    h.expect("busy", serde_json::json!([])).await;
    h.expect_ne("draft_context", Null).await;
    h.key("Down").await;
    h.key("Return").await;
    h.expect("dialog", "DiscardDraft").await;
    h.key("n").await;
    h.expect("dialog", Null).await;
    h.expect("draft_count", 2).await;
    h.right_click_at(100., 592.).await;
    h.expect_ne("draft_context", Null).await;
    h.click_at(190., 649.).await;
    h.expect("dialog", "DiscardDraft").await;
    h.key("Return").await;
    h.expect("dialog", Null).await;
    h.expect("draft_count", 1).await;
    h.expect("draft_rows.0.1", "First draft to keep").await;
    h.click_at(98., 555.).await;
    h.expect("composer.visible", true).await;
    h.expect("compose_fields.subject", "First draft to keep")
        .await;
}

/// `test_dropdown_escape_in_mail_keeps_find_and_blocks_mail_shortcuts`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropdown_escape_in_mail_keeps_find_and_blocks_mail_shortcuts() {
    let mut h = Harness::start().await;
    h.expect("reader_text_ready", true).await;
    h.expect("mail_rows.0.starred", true).await;
    let state = h.state();
    let first = state["mail_rows"][0]["id"].clone();
    let second = state["mail_rows"][1]["id"].clone();
    h.expect("selected_id", first.clone()).await;
    h.key("ctrl+f").await;
    h.expect("find_open", true).await;
    h.expect("focused_input", "find-message").await;
    h.click_at(350., 100.).await;
    // Mail shortcuts and Escape belong to the open menu, not the reader.
    for key in ["s", "ctrl+d", "Delete", "Escape"] {
        h.key(key).await;
    }
    h.expect("find_open", true).await;
    h.expect("full_reader", false).await;
    h.expect("filter", "All").await;
    h.expect("mail_rows.0.starred", true).await;
    h.expect("selected_id", first).await;
    h.expect("total", 120).await;
    h.expect("mail_pending", 0).await;
    // The Attachments row covered the second message; that row now takes the click.
    h.click_at(300., 265.).await;
    h.expect("selected_id", second).await;
    h.expect("filter", "All").await;
    h.expect("find_open", true).await;
    h.expect("total", 120).await;
    h.key("Escape").await;
    h.expect("find_open", false).await;
}

/// `test_dropdown_escape_keeps_composer_and_frees_the_covered_field`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropdown_escape_keeps_composer_and_frees_the_covered_field() {
    let mut h = Harness::start().await;
    h.click_at(110., 218.).await;
    h.expect("composer.visible", true).await;
    h.expect("focused_input", "to").await;
    h.type_text("dropdown@example.com").await;
    h.click_at(1040., 278.).await;
    h.type_text("Plans").await;
    h.expect("compose_fields.subject", "Plans").await;
    h.click_at(1040., 182.).await;
    h.key("Escape").await;
    h.expect("composer.visible", true).await;
    // The first account row covered the To field, which now takes the click.
    h.click_at(900., 229.).await;
    h.type_text(".uk").await;
    h.expect("compose_fields.to", "dropdown@example.com.uk")
        .await;
    h.expect("compose_fields.subject", "Plans").await;
    h.expect("composer.visible", true).await;
    h.key("Escape").await;
    h.expect("composer.visible", false).await;
}
