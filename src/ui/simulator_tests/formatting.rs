//! The message body's formatting toolbar and shortcuts. Formatting shows in
//! the editor as it will look; the text itself never gains marks.
use super::*;
use iced::Point;
use serde_json::Value::Null;

/// Toolbar buttons after the text size menu, left to right.
#[derive(Clone, Copy)]
enum Tool {
    Bold,
    Italic,
    Underline,
    Strikethrough,
    Bullets,
    Numbers,
    Link,
    Clear,
}

/// Centre of a toolbar button, measured from the text size menu, which is
/// the toolbar's only text.
fn tool(h: &mut Harness, size_label: &str, tool: Tool) -> Point {
    let size = h.text_center(size_label);
    let index = tool as usize;
    let separators = match tool {
        Tool::Bold | Tool::Italic | Tool::Underline | Tool::Strikethrough => 0.,
        Tool::Bullets | Tool::Numbers => 1.,
        Tool::Link | Tool::Clear => 2.,
    };
    Point::new(
        size.x + FIRST_TOOL + index as f32 * TOOL_STEP + separators * SEPARATOR,
        size.y,
    )
}

/// From the size label's centre to the first button's centre.
const FIRST_TOOL: f32 = 84.;
const TOOL_STEP: f32 = 36.;
const SEPARATOR: f32 = 13.;
/// Brings the formatting rows of the shortcut settings into view.
const SHORTCUT_SCROLL: u32 = 8;

async fn click_tool(h: &mut Harness, label: &str, which: Tool) {
    let at = tool(h, label, which);
    h.click(at).await;
}

async fn open_body(h: &mut Harness) {
    h.click_at(110., 218.).await;
    h.expect("composer.visible", true).await;
    h.expect("focused_input", "to").await;
    h.key("ctrl+b").await;
    h.expect("compose_recipients.to.entry", "").await;
    h.key("Tab").await;
    h.expect("native_focus", "subject").await;
    h.type_text("Plans").await;
    h.key("ctrl+i").await;
    h.key("ctrl+u").await;
    h.expect("compose_fields.subject", "Plans").await;
    h.key("Tab").await;
    h.expect("native_focus", "compose-body").await;
}

/// `test_compose_formatting_toolbar`, shortcut and toolbar part.
async fn compose_formatting_toolbar() {
    let mut h = Harness::start().await;
    open_body(&mut h).await;
    h.expect("compose_format.html", Null).await;
    h.type_text("Hello ").await;
    h.key("ctrl+b").await;
    h.expect("compose_format.active", serde_json::json!(["Bold"]))
        .await;
    h.type_text("team").await;
    h.key("ctrl+b").await;
    h.type_text(" today").await;
    // The editor holds the text alone; the marks are only in the plain part.
    h.expect("editor", "Hello team today").await;
    h.expect("compose_format.plain", "Hello *team* today").await;
    h.expect_contains("compose_format.html", "<strong>team</strong>")
        .await;
    // Toolbar buttons format the selection and give focus back to the body.
    h.key("ctrl+a").await;
    click_tool(&mut h, "Normal", Tool::Italic).await;
    h.expect("compose_format.plain", "_Hello *team* today_")
        .await;
    h.expect("focused_input", "compose-body").await;
    h.key("ctrl+z").await;
    h.expect("compose_format.plain", "Hello *team* today").await;
    click_tool(&mut h, "Normal", Tool::Bullets).await;
    h.expect("compose_format.bullets", true).await;
    h.expect("compose_format.plain", "- Hello *team* today")
        .await;
    h.expect("focused_input", "compose-body").await;
    h.key("ctrl+End").await;
    h.key("Return").await;
    h.type_text("second").await;
    h.expect("compose_format.plain", "- Hello *team* today\n- second")
        .await;
    click_tool(&mut h, "Normal", Tool::Bullets).await;
    h.expect("compose_format.bullets", false).await;
    click_tool(&mut h, "Normal", Tool::Numbers).await;
    h.expect("compose_format.numbers", true).await;
    h.expect("compose_format.plain", "- Hello *team* today\n1. second")
        .await;
    h.expect("focused_input", "compose-body").await;
    // Text size comes from its menu above the editor.
    h.key("shift+Home").await;
    h.click_text("Normal").await;
    h.click_text("Large").await;
    h.expect("compose_format.size", "Large").await;
    h.expect_contains("compose_format.html", "font-size").await;
    h.expect("focused_input", "compose-body").await;
    // Strikethrough and underline, then Clear takes them off again.
    click_tool(&mut h, "Large", Tool::Strikethrough).await;
    click_tool(&mut h, "Large", Tool::Underline).await;
    h.expect(
        "compose_format.active",
        serde_json::json!(["Underline", "Strikethrough"]),
    )
    .await;
    h.expect("compose_format.plain", "- Hello *team* today\n1. _second_")
        .await;
    click_tool(&mut h, "Large", Tool::Clear).await;
    h.expect("compose_format.active", serde_json::json!([]))
        .await;
    h.expect("compose_format.size", "Normal").await;
    h.expect("compose_format.plain", "- Hello *team* today\n1. second")
        .await;
    h.expect("focused_input", "compose-body").await;
    // The link button opens an address field under the toolbar.
    h.key("End").await;
    h.key("Return").await;
    h.type_text("docs").await;
    h.key("shift+Home").await;
    click_tool(&mut h, "Normal", Tool::Link).await;
    h.expect("compose_format.link", "").await;
    h.expect("focused_input", iced_cube::formatting_toolbar::LINK_FIELD)
        .await;
    h.type_text("example.test/docs").await;
    // Mail shortcuts never run from the address field.
    h.key("ctrl+d").await;
    h.key("Return").await;
    h.expect("compose_format.link", Null).await;
    h.expect(
        "compose_format.plain",
        "- Hello *team* today\n1. second\n2. docs <https://example.test/docs>",
    )
    .await;
    // The saved draft keeps the formatting.
    h.expect_contains(
        "compose_format.saved_html",
        "<a href=\"https://example.test/docs\">docs</a>",
    )
    .await;
    h.expect(
        "compose_format.saved_body",
        "- Hello *team* today\n1. second\n2. docs <https://example.test/docs>",
    )
    .await;
    // Ctrl+K still searches, and Ctrl+B there formats nothing.
    h.key("ctrl+k").await;
    h.expect("focused_input", "search").await;
    h.key("ctrl+b").await;
    h.expect("query", "").await;
    h.expect("dialog", Null).await;
    h.expect("total", 120).await;
    h.expect("mail_pending", 0).await;
}

/// Shortcuts that were remapped or turned off.
async fn remapped_and_disabled_formatting_shortcuts() {
    let mut h = Harness::start().await;
    h.key("ctrl+comma").await;
    h.expect("tab", "Preferences").await;
    h.click_at(645., 156.).await;
    h.expect("settings_tab", "Shortcuts").await;
    h.hover(900., 600.).await;
    h.scroll(SHORTCUT_SCROLL).await;
    h.click_text("Turn off formatting shortcuts").await;
    h.expect("shortcuts.Bold", "").await;
    h.expect("shortcuts.Underline", "").await;
    h.expect("preferences_saved", true).await;
    h.key("ctrl+1").await;
    h.expect("tab", "Mail").await;
    open_body(&mut h).await;
    h.type_text("plain ").await;
    h.key("ctrl+b").await;
    h.type_text("text").await;
    h.expect("compose_format.active", serde_json::json!([]))
        .await;
    h.expect("compose_format.html", Null).await;
    h.expect("editor", "plain text").await;
    h.key("ctrl+a").await;
    click_tool(&mut h, "Normal", Tool::Bold).await;
    h.expect("compose_format.plain", "*plain text*").await;
}

/// The narrow window keeps the toolbar on one row.
async fn compact_formatting_toolbar() {
    let mut h = Harness::with_size(900., 640.).await;
    open_body(&mut h).await;
    h.type_text("Short note").await;
    h.key("ctrl+a").await;
    click_tool(&mut h, "Normal", Tool::Underline).await;
    h.expect("compose_format.plain", "_Short note_").await;
    h.expect("compose_format.active", serde_json::json!(["Underline"]))
        .await;
    h.expect("focused_input", "compose-body").await;
    h.key("End").await;
    h.type_text("!").await;
    h.expect("editor", "Short note!").await;
}

scenarios!(
    compose_formatting_toolbar,
    remapped_and_disabled_formatting_shortcuts,
    compact_formatting_toolbar,
);
