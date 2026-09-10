//! Choose MIME representations before rendering. A text/plain label must not
//! expose a clearly mislabeled XHTML document, and multipart alternatives must
//! never be concatenated as duplicate messages.
use crate::model::Attachment;
use mailparse::ParsedMail;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone)]
pub struct HtmlBody {
    pub signature: [u8; 32],
    pub source: String,
    pub has_quotes: bool,
    /// Plain letters get a reading column; authored layouts keep their geometry.
    /// Determined with the MIME document off the UI thread.
    pub reading_column: bool,
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
        let reading_column = !has_authored_layout(&document);
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
            reading_column,
            remote_images,
            signature: hash.finalize().into(),
            source,
            inline,
        }
    }
}

fn has_authored_layout(document: &scraper::Html) -> bool {
    // Tables and explicit layout declarations are deliberate sender geometry.
    // Typography, colors and paragraph margins alone still describe a letter.
    // Be conservative with CSS escapes rather than guessing their meaning.
    let selector = scraper::Selector::parse("table,[width],[height],[align],[style],style")
        .expect("static selector");
    document.select(&selector).any(|element| {
        let value = element.value();
        if value.name() == "table"
            || ["width", "height", "align"]
                .iter()
                .any(|attr| value.attr(attr).is_some())
        {
            return true;
        }
        let css = if value.name() == "style" {
            element.text().collect::<String>()
        } else {
            value.attr("style").unwrap_or_default().to_owned()
        };
        let css: String = css
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect();
        css.contains('\\')
            || [
                "width:",
                "height:",
                "display:",
                "position:",
                "float:",
                "columns:",
                "column-count:",
                "grid:",
                "flex:",
                "@media",
                "@import",
            ]
            .iter()
            .any(|property| css.contains(property))
    })
}

#[cfg(test)]
mod resource_tests {
    use super::*;
    #[test]
    fn reading_defaults_distinguish_letters_from_authored_layouts() {
        for source in [
            "<p>A plain letter.</p><blockquote>Earlier reply</blockquote>",
            "<body style='background:#fafafa;color:#222;font-family:Arial'><p style='margin:0'>Letter with a signature.</p><b>Alex</b></body>",
            "<style>p { font-size:14px; margin:0 }</style><p>Typography alone is a letter.</p>",
        ] {
            assert!(
                HtmlBody::new(source.into(), HashMap::new()).reading_column,
                "{source}"
            );
        }
        for source in [
            "<table><tr><td>Sender layout</td></tr></table>",
            "<div style='max-width:800px'>Sized content</div>",
            "<style>@media screen { .columns { display:flex } }</style><div>Columns</div>",
            "<div style='position:absolute;left:50px'>Positioned content</div>",
            "<div width='1000'>Legacy dimensions</div>",
            "<div style='w\\69dth:1000px'>Escaped layout</div>",
        ] {
            assert!(
                !HtmlBody::new(source.into(), HashMap::new()).reading_column,
                "{source}"
            );
        }
    }
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
/// Metadata remains readable when an optional inline image is damaged. The
/// shared decoder owns MIME selection, resource scopes and nesting validation.
pub fn extract(parsed: &ParsedMail<'_>) -> anyhow::Result<Content> {
    let (text, attachments) = crate::model::content(parsed)?;
    let html = shep_mail_core::reader::extract(parsed)
        .ok()
        .and_then(shep_mail_core::reader::flatten)
        .map(|flat| HtmlBody::new(flat.source, flat.inline.into_iter().collect()));
    Ok(Content {
        text,
        html,
        attachments,
    })
}

pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
#[cfg(test)]
mod tests;
