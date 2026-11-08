//! Standard editing keys in each kind of text box: undo and redo, clipboard,
//! word deletion and movement, and line or document ends. Mail shortcuts must
//! never act on these keys while a field has focus.
use super::*;
use serde_json::Value::Null;

async fn open_composer(h: &mut Harness) {
    h.click_at(110., 218.).await;
    h.expect("composer.visible", true).await;
    h.expect("focused_input", "to").await;
}

/// The mail list is untouched by keys typed into a field.
async fn expect_mail_untouched(h: &mut Harness) {
    h.expect("dialog", Null).await;
    h.expect("folder", "INBOX").await;
    h.expect("mail_pending", 0).await;
}

/// `test_text_editing_keys_in_fields`, compose body part.
async fn compose_body_editing_keys() {
    let mut h = Harness::start().await;
    open_composer(&mut h).await;
    // Tab reports the field it moved to, so typing waits for real focus.
    h.key("Tab").await;
    h.expect("native_focus", "subject").await;
    h.key("Tab").await;
    h.expect("native_focus", "compose-body").await;
    h.type_text("alpha beta gamma").await;
    h.expect("editor", "alpha beta gamma").await;
    h.key("ctrl+BackSpace").await;
    h.expect("editor", "alpha beta ").await;
    h.key("ctrl+z").await;
    h.expect("editor", "alpha beta gamma").await;
    h.key("ctrl+y").await;
    h.expect("editor", "alpha beta ").await;
    h.key("ctrl+z").await;
    h.expect("editor", "alpha beta gamma").await;
    h.key("ctrl+shift+z").await;
    h.expect("editor", "alpha beta ").await;
    h.key("ctrl+z").await;
    h.expect("editor", "alpha beta gamma").await;
    // Word movement, with Shift extending the selection.
    h.key("ctrl+Home").await;
    h.key("ctrl+Delete").await;
    h.expect("editor", " beta gamma").await;
    h.key("ctrl+End").await;
    h.key("ctrl+shift+Left").await;
    h.type_text("G").await;
    h.expect("editor", " beta G").await;
    // The editor's word motions stop at the end of a word.
    h.key("ctrl+Home").await;
    h.key("ctrl+Right").await;
    h.type_text("s").await;
    h.expect("editor", " betas G").await;
    h.key("ctrl+shift+Left").await;
    h.type_text("B").await;
    h.expect("editor", " B G").await;
    // Line ends and document ends in a multi-line body.
    h.key("ctrl+End").await;
    h.key("Return").await;
    h.type_text("second line").await;
    h.expect("editor", " B G\nsecond line").await;
    h.key("Home").await;
    h.type_text("> ").await;
    h.key("End").await;
    h.type_text(".").await;
    h.expect("editor", " B G\n> second line.").await;
    h.key("ctrl+Home").await;
    h.key("shift+End").await;
    h.type_text("First").await;
    h.key("ctrl+End").await;
    h.type_text("!").await;
    h.expect("editor", "First\n> second line.!").await;
    h.key("ctrl+shift+Home").await;
    h.key("BackSpace").await;
    h.expect("editor", "").await;
    h.key("ctrl+z").await;
    h.expect("editor", "First\n> second line.!").await;
    // Clipboard keys.
    h.key("ctrl+a").await;
    h.key("ctrl+c").await;
    assert_eq!(h.clipboard(), Some("First\n> second line.!"));
    h.key("ctrl+x").await;
    h.expect("editor", "").await;
    h.set_clipboard("pasted");
    h.key("ctrl+v").await;
    h.expect("editor", "pasted").await;
    h.key("ctrl+z").await;
    h.expect("editor", "").await;
    h.key("ctrl+z").await;
    h.expect("editor", "First\n> second line.!").await;
    // Delete and Backspace archive mail outside text; here they only edit.
    h.key("Delete").await;
    h.key("BackSpace").await;
    h.expect("composer.visible", true).await;
    h.expect("total", 120).await;
    expect_mail_untouched(&mut h).await;
}

/// `test_text_editing_keys_in_fields`, subject part.
async fn subject_editing_keys() {
    let mut h = Harness::start().await;
    open_composer(&mut h).await;
    h.click_id("subject").await;
    h.expect("native_focus", "subject").await;
    h.type_text("Quarterly plans draft").await;
    h.key("ctrl+BackSpace").await;
    h.expect("compose_fields.subject", "Quarterly plans ").await;
    h.key("ctrl+z").await;
    h.expect("compose_fields.subject", "Quarterly plans draft")
        .await;
    h.key("ctrl+y").await;
    h.expect("compose_fields.subject", "Quarterly plans ").await;
    h.key("ctrl+shift+z").await;
    h.key("ctrl+z").await;
    h.expect("compose_fields.subject", "Quarterly plans draft")
        .await;
    h.key("Home").await;
    h.key("ctrl+Delete").await;
    h.expect("compose_fields.subject", " plans draft").await;
    h.key("ctrl+Right").await;
    h.key("ctrl+shift+Right").await;
    h.type_text(" notes").await;
    h.expect("compose_fields.subject", " plans notes").await;
    h.key("ctrl+Left").await;
    h.key("ctrl+Left").await;
    h.key("ctrl+shift+Right").await;
    h.type_text("Team").await;
    h.expect("compose_fields.subject", " Team notes").await;
    h.key("End").await;
    h.key("shift+Home").await;
    h.key("ctrl+c").await;
    assert_eq!(h.clipboard(), Some(" Team notes"));
    h.key("ctrl+x").await;
    h.expect("compose_fields.subject", "").await;
    h.key("ctrl+v").await;
    h.key("ctrl+v").await;
    h.expect("compose_fields.subject", " Team notes Team notes")
        .await;
    h.key("ctrl+a").await;
    h.key("Delete").await;
    h.expect("compose_fields.subject", "").await;
    h.expect("total", 120).await;
    expect_mail_untouched(&mut h).await;
}

/// `test_text_editing_keys_in_fields`, mail search part.
async fn search_editing_keys() {
    let mut h = Harness::start().await;
    h.key("ctrl+k").await;
    h.expect("focused_input", "search").await;
    h.type_text("invoice march").await;
    h.expect("query", "invoice march").await;
    h.key("ctrl+BackSpace").await;
    h.expect("query", "invoice ").await;
    h.key("ctrl+z").await;
    h.expect("query", "invoice march").await;
    h.key("ctrl+y").await;
    h.expect("query", "invoice ").await;
    h.key("ctrl+z").await;
    h.key("ctrl+shift+z").await;
    h.expect("query", "invoice ").await;
    h.key("ctrl+z").await;
    h.expect("query", "invoice march").await;
    h.key("Home").await;
    h.key("ctrl+Delete").await;
    h.expect("query", " march").await;
    h.key("ctrl+z").await;
    h.expect("query", "invoice march").await;
    h.key("ctrl+a").await;
    h.key("ctrl+c").await;
    assert_eq!(h.clipboard(), Some("invoice march"));
    h.key("ctrl+x").await;
    h.expect("query", "").await;
    h.key("ctrl+v").await;
    h.expect("query", "invoice march").await;
    h.key("End").await;
    h.key("ctrl+shift+Left").await;
    h.type_text("april").await;
    h.expect("query", "invoice april").await;
    h.key("ctrl+a").await;
    h.key("BackSpace").await;
    h.expect("query", "").await;
    h.key("ctrl+z").await;
    h.expect("query", "invoice april").await;
    h.key("ctrl+a").await;
    h.key("Delete").await;
    h.expect("query", "").await;
    h.expect("total", 120).await;
    expect_mail_untouched(&mut h).await;
}

/// `test_text_editing_keys_in_fields`, settings search part.
async fn settings_search_editing_keys() {
    let mut h = Harness::start().await;
    h.key("ctrl+comma").await;
    h.expect("tab", "Preferences").await;
    h.click_at(1150., 88.).await;
    h.expect("native_focus", "settings-search").await;
    h.type_text("dark theme").await;
    h.expect("settings_search", "dark theme").await;
    h.key("ctrl+BackSpace").await;
    h.expect("settings_search", "dark ").await;
    h.key("ctrl+z").await;
    h.expect("settings_search", "dark theme").await;
    h.key("ctrl+shift+z").await;
    h.expect("settings_search", "dark ").await;
    h.key("ctrl+z").await;
    h.key("ctrl+y").await;
    h.expect("settings_search", "dark ").await;
    h.key("ctrl+a").await;
    h.key("Delete").await;
    h.expect("settings_search", "").await;
    h.key("ctrl+z").await;
    h.expect("settings_search", "dark ").await;
    h.expect("tab", "Preferences").await;
}

/// `test_text_editing_keys_in_fields`, event dialog part. The native scenario
/// opens the dialog by double-clicking a day, which focuses the title.
async fn event_dialog_editing_keys() {
    let mut h = Harness::start().await;
    h.key("ctrl+2").await;
    h.expect("tab", "Calendar").await;
    h.click_text("New event").await;
    h.expect("dialog", "Event").await;
    h.click_id("event-title").await;
    h.expect("native_focus", "event-title").await;
    h.type_text("Team lunch Friday").await;
    h.expect("fields.title", "Team lunch Friday").await;
    h.key("ctrl+BackSpace").await;
    h.expect("fields.title", "Team lunch ").await;
    h.key("ctrl+z").await;
    h.expect("fields.title", "Team lunch Friday").await;
    h.key("ctrl+y").await;
    h.expect("fields.title", "Team lunch ").await;
    h.key("ctrl+z").await;
    h.expect("fields.title", "Team lunch Friday").await;
    h.key("Home").await;
    h.key("ctrl+shift+Right").await;
    h.type_text("S").await;
    h.expect("fields.title", "S lunch Friday").await;
    h.key("ctrl+z").await;
    h.expect("fields.title", "Team lunch Friday").await;
    // Y and N answer some dialogs; in a field they are only text.
    h.key("End").await;
    h.type_text(" yn").await;
    h.expect("fields.title", "Team lunch Friday yn").await;
    h.expect("dialog", "Event").await;
}

scenarios!(
    compose_body_editing_keys,
    subject_editing_keys,
    search_editing_keys,
    settings_search_editing_keys,
    event_dialog_editing_keys,
);
