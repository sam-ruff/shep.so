//! Select MIME bodies before rendering. HTML and inline bytes here are still
//! untrusted sender data: consumers must sanitize/confine them before display.
//! Separate mixed sections retain independent Content-ID scopes.
use anyhow::{Result, anyhow};
use mailparse::{DispositionType, MailHeaderMap, ParsedMail};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Default, Serialize)]
pub struct Body {
    /// Prefer the selected nonempty plain alternative; otherwise derive text.
    pub text: String,
    pub html: Vec<RawHtmlPart>,
    /// Each selected inline payload is stored once, even across many sections.
    pub resources: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Serialize)]
pub struct RawHtmlPart {
    pub source: String,
    /// None denotes an ambiguous duplicate CID, which must remain unresolved.
    /// Inner related scopes shadow outer ones, including ambiguous inner IDs.
    pub inline: BTreeMap<String, Option<String>>,
}

#[derive(Default)]
struct Part {
    plain: Option<String>,
    html: Vec<RawHtmlPart>,
    inline: BTreeMap<String, Option<String>>,
}
impl Part {
    fn text(&self) -> String {
        self.plain
            .as_ref()
            .filter(|text| !text.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| {
                self.html
                    .iter()
                    .map(|part| crate::plain::html_to_text(&part.source))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
    }
    fn has_body(&self) -> bool {
        self.plain.is_some() || !self.html.is_empty()
    }
    fn bind(&mut self, scope: BTreeMap<String, Option<String>>, budget: &mut Budget) -> Result<()> {
        for part in &mut self.html {
            for (id, bytes) in &scope {
                if !part.inline.contains_key(id) {
                    crate::mime::decoded(&mut budget.bindings, id.len() + 64)?;
                    part.inline.insert(id.clone(), bytes.clone());
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct Budget {
    decoded: usize,
    bindings: usize,
    resources: BTreeMap<String, Vec<u8>>,
    collect_resources: bool,
}

pub fn decode(raw: &[u8]) -> Result<Body> {
    extract(&crate::mime::parse(raw)?)
}

pub fn extract(parsed: &ParsedMail<'_>) -> Result<Body> {
    crate::mime::validate_tree(parsed)?;
    let mut budget = Budget {
        collect_resources: true,
        ..Default::default()
    };
    let part = walk(parsed, &mut budget)?;
    let used: std::collections::BTreeSet<_> = part
        .html
        .iter()
        .flat_map(|part| part.inline.values().filter_map(Option::as_ref))
        .cloned()
        .collect();
    budget.resources.retain(|key, _| used.contains(key));
    Ok(Body {
        text: part.text(),
        html: part.html,
        resources: budget.resources,
    })
}

/// Cache text and replies do not decode inline images. A corrupt optional image
/// cannot prevent the readable representation from being cached or quoted.
pub fn text(parsed: &ParsedMail<'_>) -> Result<String> {
    crate::mime::validate_tree(parsed)?;
    Ok(walk(parsed, &mut Budget::default())?.text())
}

/// One classification for selection, list indicators and attachment downloads.
pub(crate) fn is_inline_image(part: &ParsedMail<'_>) -> bool {
    part.ctype.mimetype.starts_with("image/")
        && content_id(part).is_some()
        && part.get_content_disposition().disposition != DispositionType::Attachment
}
pub fn is_attachment(part: &ParsedMail<'_>) -> bool {
    let disposition = part.get_content_disposition();
    disposition.disposition == DispositionType::Attachment
        || (!is_inline_image(part)
            && (disposition.params.contains_key("filename")
                || part.ctype.params.contains_key("name")))
}
fn content_id(part: &ParsedMail<'_>) -> Option<String> {
    part.headers
        .get_first_value("Content-ID")
        .map(|id| normalize_id(&id).to_owned())
        .filter(|id| !id.is_empty())
}
fn normalize_id(id: &str) -> &str {
    id.trim().trim_matches(['<', '>']).trim()
}
fn merge(scope: &mut BTreeMap<String, Option<String>>, incoming: BTreeMap<String, Option<String>>) {
    for (id, bytes) in incoming {
        use std::collections::btree_map::Entry;
        match scope.entry(id) {
            Entry::Vacant(entry) => {
                entry.insert(bytes);
            }
            Entry::Occupied(mut entry) => {
                entry.insert(None);
            }
        }
    }
}

fn walk(part: &ParsedMail<'_>, total: &mut Budget) -> Result<Part> {
    if is_attachment(part) {
        return Ok(Part::default());
    }
    if is_inline_image(part) {
        if !total.collect_resources {
            return Ok(Part::default());
        }
        let bytes = part.get_body_raw().map_err(|_| {
            anyhow!("Could not decode an inline image. Refresh this message and retry.")
        })?;
        crate::mime::decoded(&mut total.decoded, bytes.len())?;
        use sha2::{Digest, Sha256};
        let key = format!("{:x}", Sha256::digest(&bytes));
        total.resources.entry(key.clone()).or_insert(bytes);
        return Ok(Part {
            inline: BTreeMap::from([(content_id(part).unwrap(), Some(key))]),
            ..Default::default()
        });
    }
    if part.subparts.is_empty() {
        if !matches!(
            part.ctype.mimetype.as_str(),
            "text/plain" | "text/html" | "application/xhtml+xml"
        ) {
            return Ok(Part::default());
        }
        let body = part
            .get_body()
            .map_err(|_| anyhow!("Could not decode this message's text. Refresh it and retry."))?;
        crate::mime::decoded(&mut total.decoded, body.len())?;
        if body.trim().is_empty() {
            return Ok(Part::default());
        }
        let html = if part.ctype.mimetype == "text/plain" {
            mislabeled_html(&body)
        } else {
            Some(body.clone())
        };
        return Ok(match html {
            Some(source) => Part {
                html: vec![RawHtmlPart {
                    source,
                    inline: BTreeMap::new(),
                }],
                ..Default::default()
            },
            None => Part {
                plain: Some(body),
                ..Default::default()
            },
        });
    }
    if part.ctype.mimetype == "multipart/alternative" {
        let mut result = Part::default();
        for child in &part.subparts {
            let mut choice = walk(child, total)?;
            if choice.plain.is_some() {
                result.plain = choice.plain.take();
            }
            if !choice.html.is_empty() {
                result.html = choice.html;
            }
        }
        return Ok(result);
    }
    if part.ctype.mimetype == "multipart/related" {
        let start = part
            .ctype
            .params
            .get("start")
            .map(|value| normalize_id(value));
        let root = start
            .and_then(|id| {
                part.subparts
                    .iter()
                    .position(|child| content_id(child).as_deref() == Some(id))
            })
            .unwrap_or(0);
        let mut result = walk(&part.subparts[root], total)?;
        let mut scope = std::mem::take(&mut result.inline);
        for (index, child) in part.subparts.iter().enumerate() {
            if index != root {
                // Other HTML/text related resources are not additional bodies.
                // A nested related message owns its own resources, not ours.
                if is_inline_image(child) {
                    merge(&mut scope, walk(child, total)?.inline);
                }
            }
        }
        result.bind(scope, total)?;
        return Ok(result);
    }
    let mut bodies = Vec::new();
    let mut scope = BTreeMap::new();
    for child in &part.subparts {
        let mut item = walk(child, total)?;
        merge(&mut scope, std::mem::take(&mut item.inline));
        if item.has_body() {
            bodies.push(item);
        }
    }
    let mut result = if bodies.len() == 1 {
        bodies.pop().unwrap()
    } else {
        let has_html = bodies.iter().any(|body| !body.html.is_empty());
        let text = bodies
            .iter()
            .map(Part::text)
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut result = Part {
            plain: (!text.is_empty()).then_some(text),
            ..Default::default()
        };
        if has_html {
            for mut body in bodies {
                if body.html.is_empty() {
                    result.html.push(RawHtmlPart {
                        source: format!(
                            "<pre style=\"white-space:pre-wrap\">{}</pre>",
                            escape(body.plain.as_deref().unwrap_or_default())
                        ),
                        inline: BTreeMap::new(),
                    });
                } else {
                    result.html.append(&mut body.html);
                }
            }
        }
        result
    };
    result.bind(scope, total)?;
    Ok(result)
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn is_document(text: &str) -> bool {
    let text = text.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    let prefix: String = text
        .chars()
        .take(1024)
        .flat_map(char::to_lowercase)
        .collect();
    let prefix = if prefix.starts_with("<?xml") {
        prefix
            .split_once("?>")
            .map(|(_, rest)| rest.trim_start())
            .unwrap_or("")
    } else {
        &prefix
    };
    let root = prefix.starts_with("<!doctype html")
        || prefix
            .strip_prefix("<html")
            .is_some_and(|rest| rest.starts_with(['>', ' ', '\t', '\r', '\n']));
    root && text
        .as_bytes()
        .windows(7)
        .any(|window| window.eq_ignore_ascii_case(b"</html>"))
}
fn mislabeled_html(body: &str) -> Option<String> {
    if is_document(body) {
        return Some(body.into());
    }
    let prefix = body.trim_start();
    if prefix.starts_with("&lt;") || prefix.starts_with("&#") {
        let decoded = scraper::Html::parse_fragment(body)
            .root_element()
            .text()
            .collect::<String>();
        if is_document(&decoded) {
            return Some(decoded);
        }
    }
    None
}
