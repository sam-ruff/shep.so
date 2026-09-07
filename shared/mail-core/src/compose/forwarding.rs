use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardQuote {
    pub text: String,
    pub html_head: String,
    #[serde(default)]
    pub html_attributes: String,
    pub html_body: String,
}
impl ForwardQuote {
    /// Preserve formatting when adding a note above the original. Editing the
    /// quoted text deliberately switches to the edited plain-text alternative.
    pub fn render(&self, body: &str) -> Option<String> {
        if self.html_body.is_empty() {
            return None;
        }
        let note = body.strip_suffix(&self.text)?;
        Some(format!(
            "<!doctype html><html><head><meta charset=\"utf-8\">{}</head><body{}><div style=\"white-space:pre-wrap\">{}</div>{}</body></html>",
            self.html_head,
            self.html_attributes,
            escape(note),
            self.html_body
        ))
    }
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
