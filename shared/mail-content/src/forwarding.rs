//! Complete-source forwarding for native clients and browser workers.
//!
//! The retained HTML is outgoing MIME content, NOT safe to insert into a UI.
//! All displays still require the confined-document preparation path. This
//! module has no draft store, identity generator, credentials or network calls.
use mailparse::MailHeaderMap;
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

// Same limits for native and browser draft preparation and MIME construction.
pub const MAX_ATTACHMENT_BYTES: usize = 18 * 1024 * 1024;
pub const MAX_ATTACHMENTS: usize = 32;

/// Metadata serializes separately from binary files. A caller must persist all
/// files and its new draft in one transaction; failure must leave neither.
#[derive(Debug, Serialize)]
pub struct PreparedForward {
    pub subject: String,
    pub body: String,
    pub forward: ForwardQuote,
    pub files: Vec<ForwardFile>,
}

#[derive(Debug, Serialize)]
pub struct ForwardFile {
    pub name: String,
    pub media_type: String,
    pub content_id: Option<String>,
    pub size: usize,
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

pub fn prepare(raw: &[u8]) -> anyhow::Result<PreparedForward> {
    let parsed = crate::mime::parse(raw)?;
    anyhow::ensure!(
        crate::attachments::parts(&parsed)
            .take(MAX_ATTACHMENTS + 1)
            .count()
            <= MAX_ATTACHMENTS,
        "This forward exceeds the current attachment limit. No partial draft was saved."
    );
    // Readable caches can omit damaged optional resources. A forward cannot
    // silently drop them or save an empty replacement file.
    let selected = crate::reader::extract(&parsed)?;
    let selected_text = selected.text.clone();
    let html = crate::reader::flatten(selected);
    let mut headers = String::from("---------- Forwarded message ----------\n");
    for name in ["From", "Date", "Subject", "To", "Cc"] {
        if let Some(value) = parsed.headers.get_first_value(name) {
            // Only explicit public headers; never Bcc or transport metadata.
            headers.push_str(&format!(
                "{name}: {}\n",
                value.replace(['\r', '\n', '\0'], " ")
            ));
        }
    }
    let subject = parsed
        .headers
        .get_first_value("Subject")
        .unwrap_or_default()
        .replace(['\r', '\n', '\0'], " ");
    let subject = if ["fwd:", "fw:"].iter().any(|prefix| {
        subject
            .get(..prefix.len())
            .is_some_and(|s| s.eq_ignore_ascii_case(prefix))
    }) {
        subject
    } else {
        format!("Fwd: {subject}")
    };
    let quoted = format!("{headers}\n{selected_text}");
    let forward = html.as_ref().map(|html| {
        let mut document = scraper::Html::parse_document(&html.source);
        // Keep the original email's styling but don't propagate active content.
        let ids: Vec<_> = document
            .select(
                &scraper::Selector::parse("script,iframe,object,embed,form,base,meta,link")
                    .expect("static selector"),
            )
            .map(|n| n.id())
            .collect();
        for id in ids {
            if let Some(mut node) = document.tree.get_mut(id) {
                node.detach();
            }
        }
        let ids: Vec<_> = document.tree.nodes().map(|n| n.id()).collect();
        for id in ids {
            if let Some(mut node) = document.tree.get_mut(id)
                && let scraper::Node::Element(element) = node.value()
            {
                element.attrs.retain(|(name, value)| {
                    !(name.local.as_ref().starts_with("on")
                        || matches!(name.local.as_ref(), "href" | "src" | "action")
                            && value.trim().to_ascii_lowercase().starts_with("javascript:"))
                });
            }
        }
        let head = document
            .select(&scraper::Selector::parse("head style").expect("static selector"))
            .map(|n| n.html())
            .collect::<String>();
        let body = document
            .select(&scraper::Selector::parse("body").expect("static selector"))
            .next()
            .expect("HTML parser supplies body");
        let attrs = body
            .value()
            .attrs()
            .map(|(name, value)| format!(" {name}=\"{}\"", escape(value)))
            .collect::<String>();
        ForwardQuote {
            text: quoted.clone(),
            html_head: head,
            html_attributes: attrs,
            html_body: format!(
                "<div style=\"white-space:pre-wrap\">{}</div>{}",
                escape(&format!("{headers}\n")),
                body.inner_html()
            ),
        }
    });
    let mut files = Vec::new();
    // Use metadata from the actual MIME part, including duplicate names/bytes
    // with different media types. Suggested names never carry sender paths.
    crate::attachments::decoded_parts(&parsed, |info, bytes| {
        files.push(ForwardFile {
            name: info.name,
            media_type: info.media_type,
            content_id: None,
            size: bytes.len(),
            bytes,
        });
    })?;
    if let Some(html) = html {
        for (cid, bytes) in html.inline {
            let format = image::guess_format(&bytes).ok();
            files.push(ForwardFile {
                name: format!(
                    "image-{}.{}",
                    files.len() + 1,
                    format.map(|f| f.extensions_str()[0]).unwrap_or("bin")
                ),
                media_type: inline_type(&parsed, &bytes).unwrap_or_else(|| {
                    format
                        .map(|f| f.to_mime_type())
                        .unwrap_or("application/octet-stream")
                        .into()
                }),
                content_id: Some(cid),
                size: bytes.len(),
                bytes: bytes.to_vec(),
            });
        }
    }
    anyhow::ensure!(
        files.len() <= MAX_ATTACHMENTS
            && files.iter().map(|f| f.bytes.len()).sum::<usize>() <= MAX_ATTACHMENT_BYTES,
        "This forward exceeds the current attachment limit. No partial draft was saved."
    );
    Ok(PreparedForward {
        subject,
        body: format!("\n\n{quoted}"),
        forward: forward.unwrap_or_else(|| ForwardQuote {
            text: quoted,
            html_head: String::new(),
            html_attributes: String::new(),
            html_body: String::new(),
        }),
        files,
    })
}

fn inline_type(parsed: &mailparse::ParsedMail<'_>, bytes: &[u8]) -> Option<String> {
    let mut stack = vec![parsed];
    while let Some(part) = stack.pop() {
        // A file with a Content-ID remains an attachment when explicitly
        // attached. It must not supply the type of a selected inline image.
        if crate::reader::is_inline_image(part)
            && part.get_body_raw().ok().as_deref() == Some(bytes)
        {
            return Some(part.ctype.mimetype.clone());
        }
        stack.extend(part.subparts.iter().rev());
    }
    None
}
