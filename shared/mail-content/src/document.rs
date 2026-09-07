//! Resource-confined HTML for browser frames and native system web views.
//! Prepare on a worker. The only executable code is the fixed display runtime;
//! sender HTML is sanitized, links are inert metadata, and images are bounded
//! WebP blobs. Callers must additionally sandbox frames and deny navigation.
mod css;
mod resources;

use crate::reader::{Body, RawHtmlPart, escape};
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use url::Url;

const RUNTIME: &str = include_str!("document/runtime.js");
/// The containing site's CSP must also permit this exact trusted runtime,
/// because srcdoc frames inherit that policy and can only restrict it further.
pub fn runtime_csp_source() -> String {
    format!(
        "'sha256-{}'",
        STANDARD.encode(Sha256::digest(RUNTIME.as_bytes()))
    )
}
const BASE_CSS: &str = "html{margin:0;min-height:100%}body{margin:0;padding:12px;font:15px/1.5 Arial,sans-serif;overflow-wrap:break-word;color:var(--shep-text,#18181b);background:var(--shep-background,#fff)}*{box-sizing:border-box;user-select:text;-webkit-user-select:text}a[data-shep-link]{color:inherit;text-decoration:underline;cursor:pointer}::highlight(shep-matches){background:#ede3fb;color:#18181b}::highlight(shep-active){background:#b896e4;color:#18181b}shep-match{display:inline!important;padding:0!important;margin:0!important;border:0!important;font:inherit!important;color:inherit!important;background:#7754a544!important}shep-match[data-active]{background:#b896e4!important;color:#18181b!important}shep-match:before,shep-match:after{content:none!important}";

#[derive(Debug, Deserialize, Serialize)]
pub struct Options {
    pub generation: String,
    #[serde(default)]
    pub dark: bool,
    #[serde(default)]
    pub quotes: bool,
}
#[derive(Debug, Serialize)]
pub struct Prepared {
    pub signature: String,
    pub text: String,
    pub document: Option<String>,
    pub remote_images: Vec<RemoteImage>,
    pub issues: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct RemoteImage {
    pub url: String,
    pub alt: String,
}
#[derive(Debug, Serialize)]
struct Image {
    bytes: String,
    width: u32,
    height: u32,
}

pub fn prepare(raw: &[u8], options: &Options) -> Result<Prepared> {
    anyhow::ensure!(
        options.generation.len() <= 128 && !options.generation.is_empty(),
        "Invalid reader generation."
    );
    let body = crate::reader::decode(raw)?;
    from_body(body, format!("{:x}", Sha256::digest(raw)), options)
}
fn sanitizer() -> ammonia::Builder<'static> {
    let mut clean = ammonia::Builder::default();
    clean
        .rm_clean_content_tags(&["style"])
        .add_clean_content_tags(&[
            "script", "iframe", "object", "embed", "svg", "math", "template", "noscript",
        ])
        .add_tags(&["style", "font"])
        .add_generic_attributes(&[
            "class",
            "id",
            "style",
            "dir",
            "lang",
            "align",
            "valign",
            "bgcolor",
            "background",
            "width",
            "height",
        ])
        .add_tag_attributes("font", &["color", "size", "face"])
        .add_tag_attributes("table", &["cellpadding", "cellspacing", "border"])
        .add_tag_attributes("img", &["srcset"])
        .add_url_schemes(&["cid", "data"])
        .url_relative(ammonia::UrlRelative::PassThrough);
    clean
}
fn from_body(body: Body, signature: String, options: &Options) -> Result<Prepared> {
    if body.html.is_empty() {
        return Ok(Prepared {
            signature,
            text: body.text,
            document: None,
            remote_images: vec![],
            issues: vec![],
        });
    }
    let mut resources = resources::Resources::default();
    let mut content = String::new();
    let mut links = BTreeMap::new();
    let clean = sanitizer();
    for (index, part) in body.html.iter().enumerate() {
        let source = Html::parse_document(&part.source);
        let base = source
            .select(&Selector::parse("base[href]").unwrap())
            .next()
            .and_then(|node| resources::web_url(node.value().attr("href")?, None))
            .and_then(|url| Url::parse(&url).ok());
        let element = source
            .select(&Selector::parse("body").unwrap())
            .next()
            .unwrap();
        let attrs = element
            .value()
            .attrs()
            .map(|(name, value)| format!(" {name}=\"{}\"", escape(value)))
            .collect::<String>();
        let styles = source
            .select(&Selector::parse("head style").unwrap())
            .map(|style| style.html())
            .collect::<String>();
        let input = format!("<div{attrs}>{styles}{}</div>", element.inner_html());
        let sanitized = clean.clean(&input).to_string();
        let tree = Html::parse_fragment(&sanitized);
        let wrapper = tree.root_element().child_elements().next().unwrap();
        if index > 0 {
            content.push_str("<hr>");
        }
        content.push_str("<section");
        if index == 0 {
            content.push_str(" data-shep-body=\"true\"");
        }
        attributes(
            wrapper,
            &mut content,
            &mut resources,
            base.as_ref(),
            part,
            &body,
            &mut links,
        );
        content.push('>');
        let mut stack = wrapper
            .children()
            .rev()
            .map(|node| (node.id(), false))
            .collect::<Vec<_>>();
        while let Some((id, close)) = stack.pop() {
            let node = tree.tree.get(id).unwrap();
            if let Some(element) = ElementRef::wrap(node) {
                let name = element.value().name();
                if close {
                    content.push_str(&format!("</{name}>"));
                    continue;
                }
                if name == "style" {
                    let css = css::rewrite(&element.text().collect::<String>(), |url| {
                        resources.image(url, "Email background", base.as_ref(), part, &body)
                    });
                    content.push_str(&format!("<style>{css}</style>"));
                    continue;
                }
                content.push('<');
                content.push_str(name);
                attributes(
                    element,
                    &mut content,
                    &mut resources,
                    base.as_ref(),
                    part,
                    &body,
                    &mut links,
                );
                content.push('>');
                if !matches!(name, "area" | "br" | "col" | "hr" | "img" | "wbr") {
                    stack.push((id, true));
                    stack.extend(element.children().rev().map(|node| (node.id(), false)));
                }
            } else if let Some(text) = node.value().as_text() {
                content.push_str(&escape(text));
            }
        }
        content.push_str("</section>");
    }
    let data = serde_json::json!({"generation":options.generation,"dark":options.dark,"quotes":options.quotes,"images":resources.images,"links":links});
    let data = serde_json::to_string(&data)?.replace('<', "\\u003c");
    let runtime = runtime_csp_source();
    let csp = format!(
        "default-src 'none'; script-src {runtime}; style-src 'unsafe-inline'; img-src blob:; font-src 'none'; media-src 'none'; frame-src 'none'; connect-src 'none'; worker-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
    );
    let document = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"referrer\" content=\"no-referrer\"><style>{BASE_CSS}</style></head><body><template id=\"shep-content\">{content}</template><script id=\"shep-data\" type=\"application/json\">{data}</script><script>{RUNTIME}</script></body></html>"
    );
    Ok(Prepared {
        signature,
        text: body.text,
        document: Some(document),
        remote_images: resources.remote.into_values().collect(),
        issues: resources.issues.into_iter().collect(),
    })
}
fn attributes(
    element: ElementRef<'_>,
    output: &mut String,
    resources: &mut resources::Resources,
    base: Option<&Url>,
    part: &RawHtmlPart,
    body: &Body,
    links: &mut BTreeMap<String, String>,
) {
    let name = element.value().name();
    let alt = element.value().attr("alt").unwrap_or("Email image");
    for (key, value) in element.value().attrs() {
        let mapped = match key {
            "href" if matches!(name, "a" | "area") => {
                if let Some(link) = resources::link(value, base) {
                    let index = links.len().to_string();
                    links.insert(index.clone(), link);
                    output.push_str(&format!(
                        " data-shep-link=\"{index}\" role=\"link\" tabindex=\"0\""
                    ));
                }
                continue;
            }
            "src" if name == "img" => resources
                .image(value, alt, base, part, body)
                .map(|key| format!("urn:shep-image:{key}")),
            "background" => resources
                .image(value, "Email background", base, part, body)
                .map(|key| format!("urn:shep-image:{key}")),
            "style" => Some(css::rewrite(value, |url| {
                resources.image(url, "Email background", base, part, body)
            })),
            "srcset" => Some(srcset(value, |url| {
                resources.image(url, alt, base, part, body)
            })),
            "href" | "src" | "ping" | "action" | "formaction" | "target" | "download" => None,
            _ if key.starts_with("data-") || key.starts_with("on") => None,
            _ => Some(value.into()),
        };
        if let Some(value) = mapped {
            output.push_str(&format!(" {key}=\"{}\"", escape(&value)));
        }
    }
}
fn srcset(value: &str, mut image: impl FnMut(&str) -> Option<String>) -> String {
    let mut rest = value;
    let mut candidates = Vec::new();
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if rest.is_empty() {
            break;
        }
        let end = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let mut url = &rest[..end];
        rest = &rest[end..];
        let descriptor = if url.ends_with(',') {
            url = url.trim_end_matches(',');
            ""
        } else {
            let end = rest.find(',').unwrap_or(rest.len());
            let value = rest[..end].trim();
            rest = &rest[end..];
            value
        };
        let valid = descriptor.is_empty()
            || (descriptor.split_whitespace().count() == 1
                && descriptor
                    .strip_suffix('x')
                    .and_then(|v| v.parse::<f64>().ok())
                    .is_some_and(|v| v.is_finite() && v > 0.0))
            || descriptor
                .strip_suffix('w')
                .and_then(|v| v.parse::<u32>().ok())
                .is_some_and(|v| v > 0);
        if valid && let Some(key) = image(url) {
            candidates.push(format!("urn:shep-image:{key} {descriptor}"));
        }
    }
    candidates.join(", ")
}
