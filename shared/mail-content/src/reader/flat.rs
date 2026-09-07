//! Adapt selected MIME sections to renderers that accept one HTML document.
//! This is untrusted HTML, not a sanitizer. CID identities are rebound before
//! combining sections, so one related part cannot supply another part's images.
use super::Body;
use std::{collections::BTreeMap, sync::Arc};

pub struct FlatHtml {
    pub source: String,
    pub inline: BTreeMap<String, Arc<[u8]>>,
}

pub fn flatten(body: Body) -> Option<FlatHtml> {
    if body.html.is_empty() {
        return None;
    }
    let mut sections = Vec::new();
    let mut styles = String::new();
    let mut inline = BTreeMap::new();
    let resources: BTreeMap<_, Arc<[u8]>> = body
        .resources
        .into_iter()
        .map(|(k, v)| (k, v.into()))
        .collect();
    for part in body.html {
        let mut document = scraper::Html::parse_document(&part.source);
        let base = document
            .select(&scraper::Selector::parse("base[href]").unwrap())
            .next()
            .and_then(|n| url::Url::parse(n.attr("href")?).ok());
        let mut resource = |value: &str| {
            let value = value.trim();
            if let Some((scheme, id)) = value.split_once(':')
                && scheme.eq_ignore_ascii_case("cid")
            {
                let id = percent_encoding::percent_decode_str(id)
                    .decode_utf8()
                    .ok()?;
                let key = part.inline.get(id.trim_matches(['<', '>']))?.as_ref()?;
                let bytes = resources.get(key)?;
                let cid = format!("shep-{key}@inline");
                inline.entry(cid.clone()).or_insert_with(|| bytes.clone());
                return Some(format!("cid:{cid}"));
            }
            Some(
                base.as_ref()
                    .and_then(|base| base.join(value).ok())
                    .map(String::from)
                    .unwrap_or_else(|| value.into()),
            )
        };
        let ids: Vec<_> = document.tree.nodes().map(|node| node.id()).collect();
        for id in ids {
            let Some(node) = document.tree.get(id) else {
                continue;
            };
            let Some(element) = scraper::ElementRef::wrap(node) else {
                continue;
            };
            if element.value().name() == "style" {
                let rewritten = crate::document::css::rewrite_urls(
                    &element.text().collect::<String>(),
                    &mut resource,
                );
                styles.push_str(&format!("<style>{rewritten}</style>"));
                document.tree.get_mut(id).unwrap().detach();
            } else if element.value().name() == "base" {
                document.tree.get_mut(id).unwrap().detach();
            } else if let Some(mut node) = document.tree.get_mut(id)
                && let scraper::Node::Element(element) = node.value()
            {
                element.attrs.retain_mut(|(name, value)| {
                    let rewritten = match name.local.as_ref() {
                        "src" | "href" | "background" | "poster" => resource(value),
                        "style" => Some(crate::document::css::rewrite_urls(value, &mut resource)),
                        "srcset" => None,
                        _ => return true,
                    };
                    if let Some(rewritten) = rewritten {
                        *value = rewritten.into();
                        true
                    } else {
                        false
                    }
                });
            }
        }
        let element = document
            .select(&scraper::Selector::parse("body").unwrap())
            .next()
            .unwrap();
        let attributes = element
            .value()
            .attrs()
            .map(|(name, value)| format!(" {name}=\"{}\"", escape(value)))
            .collect::<String>();
        sections.push((attributes, element.inner_html()));
    }
    let (attributes, content) = if sections.len() == 1 {
        sections.pop().unwrap()
    } else {
        (
            String::new(),
            sections
                .into_iter()
                .map(|(attrs, content)| format!("<section{attrs}>{content}</section>"))
                .collect::<Vec<_>>()
                .join("\n<hr>\n"),
        )
    };
    Some(FlatHtml {
        source: format!(
            "<!doctype html><html><head>{styles}</head><body{attributes}>{content}</body></html>"
        ),
        inline,
    })
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
