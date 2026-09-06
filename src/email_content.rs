//! Choose MIME representations before rendering. A text/plain label must not
//! expose a clearly mislabeled XHTML document, and multipart alternatives must
//! never be concatenated as duplicate messages.
use crate::model::Attachment;
use mailparse::{DispositionType, MailHeaderMap, ParsedMail};
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone)]
pub struct HtmlBody {
    pub signature: [u8; 32],
    pub source: String,
    pub has_quotes: bool,
    pub remote_images: Vec<crate::model::RemoteImage>,
    pub bytes: usize,
    /// Content-ID resources are scoped to this MIME representation. These bytes
    /// are decoded and converted to WebP by the rendering worker, never the UI.
    pub inline: HashMap<String, Arc<[u8]>>,
}
impl HtmlBody {
    pub fn new(source: String, inline: HashMap<String, Arc<[u8]>>) -> Self {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(source.as_bytes());
        let mut keys: Vec<_> = inline.keys().collect();
        keys.sort();
        for key in keys {
            hash.update(key.as_bytes());
            hash.update(&inline[key]);
        }
        let document = scraper::Html::parse_document(&source);
        let remote_images = crate::remote_images::extract_document(&document);
        let has_quotes = document
            .select(
                &scraper::Selector::parse("blockquote,.gmail_quote,.yahoo_quoted")
                    .expect("static selector"),
            )
            .next()
            .is_some();
        Self {
            bytes: source.len()
                + inline.values().map(|bytes| bytes.len()).sum::<usize>()
                + remote_images
                    .iter()
                    .map(|image| image.url.len() + image.alt.len())
                    .sum::<usize>(),
            has_quotes,
            remote_images,
            signature: hash.finalize().into(),
            source,
            inline,
        }
    }
}

#[cfg(test)]
mod resource_tests {
    use super::*;
    #[test]
    fn image_policy_metadata_exists_before_any_render_including_css_and_relative_urls() {
        let body = HtmlBody::new(r#"<base href="https://images.example.test/news/">
            <style>@import url('https://ignore.example.test/styles.css');
            @font-face { src: url('https://ignore.example.test/font.woff'); }
            @media screen { .banner { background: url('../banner.webp'); } }
            .escaped { background-image: u\72l(https://images.example.test/escaped.webp); }
            </style><table background="paper.webp"><tr><td style="background:url(&quot;../banner.webp&quot;)">
            <img src="logo.webp" alt="Company logo"><img src="cid:local"><img src="data:image/webp;base64,AA==">
            <img src="file:///tmp/private"><img src="https://user:secret@example.test/image">
            <a href="https://links.example.test/help">Help</a></td></tr></table>"#.into(), HashMap::new());
        let urls: std::collections::HashSet<_> = body
            .remote_images
            .iter()
            .map(|image| image.url.as_str())
            .collect();
        assert_eq!(
            urls,
            std::collections::HashSet::from([
                "https://images.example.test/banner.webp",
                "https://images.example.test/escaped.webp",
                "https://images.example.test/news/paper.webp",
                "https://images.example.test/news/logo.webp",
            ])
        );
        assert_eq!(
            body.remote_images.len(),
            4,
            "Shared CSS references must not duplicate resources"
        );
        assert_eq!(
            body.remote_images
                .iter()
                .find(|image| image.url.ends_with("logo.webp"))
                .unwrap()
                .alt,
            "Company logo"
        );
    }
}
#[derive(Default)]
pub struct Content {
    pub text: String,
    pub html: Option<HtmlBody>,
    pub attachments: Vec<Attachment>,
}
#[derive(Default)]
struct Part {
    plain: Option<String>,
    html: Option<String>,
    inline: HashMap<String, Arc<[u8]>>,
    attachments: Vec<Attachment>,
}
impl Part {
    fn text(&self) -> String {
        self.plain
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| {
                self.html
                    .as_deref()
                    .map(crate::model::html_to_text)
                    .unwrap_or_default()
            })
    }
}

pub fn extract(parsed: &ParsedMail<'_>) -> Content {
    let part = walk(parsed);
    Content {
        text: part.text(),
        html: part.html.map(|source| HtmlBody::new(source, part.inline)),
        attachments: part.attachments,
    }
}

fn walk(part: &ParsedMail<'_>) -> Part {
    let disposition = part.get_content_disposition();
    let cid = part
        .headers
        .get_first_value("Content-ID")
        .map(|id| id.trim().trim_matches(['<', '>']).to_owned())
        .filter(|id| !id.is_empty());
    let inline_image = part.ctype.mimetype.starts_with("image/")
        && cid.is_some()
        && disposition.disposition != DispositionType::Attachment;
    if disposition.disposition == DispositionType::Attachment
        || disposition.params.contains_key("filename") && !inline_image
    {
        return Part {
            attachments: vec![Attachment {
                name: disposition
                    .params
                    .get("filename")
                    .or_else(|| part.ctype.params.get("name"))
                    .cloned()
                    .unwrap_or_else(|| "attachment.bin".into()),
                bytes: part.get_body_raw().unwrap_or_default(),
            }],
            ..Default::default()
        };
    }
    if inline_image {
        return Part {
            inline: HashMap::from([(
                cid.unwrap(),
                Arc::from(part.get_body_raw().unwrap_or_default()),
            )]),
            ..Default::default()
        };
    }
    if part.subparts.is_empty() {
        let Ok(body) = part.get_body() else {
            return Part::default();
        };
        return match part.ctype.mimetype.as_str() {
            "text/html" | "application/xhtml+xml" => Part {
                html: Some(body),
                ..Default::default()
            },
            "text/plain" => match mislabeled_html(&body) {
                Some(html) => Part {
                    html: Some(html),
                    ..Default::default()
                },
                None => Part {
                    plain: Some(body),
                    ..Default::default()
                },
            },
            _ => Part::default(),
        };
    }
    if part.ctype.mimetype == "multipart/alternative" {
        let mut result = Part::default();
        for child in &part.subparts {
            let mut choice = walk(child);
            if choice.plain.as_ref().is_some_and(|s| !s.trim().is_empty()) {
                result.plain = choice.plain.take();
            }
            if choice.html.as_ref().is_some_and(|s| !s.trim().is_empty()) {
                result.html = choice.html.take();
                result.inline = choice.inline;
            }
            result.attachments.extend(choice.attachments);
        }
        return result;
    }
    if part.ctype.mimetype == "multipart/related" {
        let start = part
            .ctype
            .params
            .get("start")
            .map(|s| s.trim_matches(['<', '>']));
        let root = start
            .and_then(|start| {
                part.subparts.iter().position(|p| {
                    p.headers
                        .get_first_value("Content-ID")
                        .is_some_and(|id| id.trim().trim_matches(['<', '>']) == start)
                })
            })
            .unwrap_or(0);
        let mut result = walk(&part.subparts[root]);
        for (index, child) in part.subparts.iter().enumerate() {
            if index == root {
                continue;
            }
            let resource = walk(child);
            // Other text/html resources are resources, not another message body.
            result.inline.extend(resource.inline);
            result.attachments.extend(resource.attachments);
        }
        return result;
    }
    let mut bodies = Vec::new();
    let mut result = Part::default();
    for child in &part.subparts {
        let mut item = walk(child);
        result.inline.extend(item.inline.drain());
        result.attachments.append(&mut item.attachments);
        if item.plain.is_some() || item.html.is_some() {
            bodies.push(item);
        }
    }
    if bodies.len() == 1 {
        let body = bodies.pop().unwrap();
        result.plain = body.plain;
        result.html = body.html;
    } else if !bodies.is_empty() {
        result.plain = Some(
            bodies
                .iter()
                .map(Part::text)
                .collect::<Vec<_>>()
                .join("\n\n"),
        );
        if bodies.iter().any(|p| p.html.is_some()) {
            result.html = Some(
                bodies
                    .iter()
                    .map(|p| {
                        p.html.clone().unwrap_or_else(|| {
                            format!(
                                "<pre style=\"white-space:pre-wrap\">{}</pre>",
                                escape(p.plain.as_deref().unwrap_or_default())
                            )
                        })
                    })
                    .collect::<Vec<_>>()
                    .join("\n<hr>\n"),
            );
        }
    }
    result
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn is_document(text: &str) -> bool {
    let text = text.trim_start_matches(['\u{feff}', ' ', '\t', '\r', '\n']);
    // Inspect a bounded prefix; body size must not multiply sniffing allocations.
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
        // One entity-decoding pass only. Ordinary prose/code fragments remain text.
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

#[cfg(test)]
mod tests;
