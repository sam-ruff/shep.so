use super::Options;
use crate::email_content::escape;
use base64::Engine;
use mailparse::MailHeaderMap;

pub(super) fn prepare(raw: &[u8], options: Options, nonce: &str) -> anyhow::Result<String> {
    let parsed = shep_mail_core::mime::parse(raw)?;
    let content = crate::email_content::extract(&parsed)?;
    let headers: Vec<_> = ["Subject", "From", "To", "Cc", "Date"]
        .into_iter()
        .filter_map(|name| {
            parsed
                .headers
                .get_first_value(name)
                .map(|value| (name, value.replace(['\r', '\n', '\0'], " ")))
        })
        .collect();
    let files: Vec<_> = content.attachments.iter().map(|a| a.name.clone()).collect();
    let mut images = options.images;
    let source = if let Some(html) = content.html.filter(|_| !options.plain) {
        for (cid, bytes) in html.inline {
            images.insert(format!("cid:{cid}"), bytes);
        }
        html.source
    } else {
        format!(
            "<html><head></head><body><pre style=\"white-space:pre-wrap;overflow-wrap:anywhere;font:14px/1.5 system-ui\">{}</pre></body></html>",
            escape(&content.text)
        )
    };
    // Decode known raster resources only; SVG, HTML and arbitrary data URLs can
    // never become active documents. No remote loader exists on this path.
    let images: std::collections::HashMap<_, _> = images
        .into_iter()
        .filter_map(|(url, bytes)| {
            crate::remote_images::convert_to_webp(&bytes)
                .ok()
                .map(|bytes| {
                    (
                        url,
                        format!(
                            "data:image/webp;base64,{}",
                            base64::engine::general_purpose::STANDARD.encode(bytes)
                        ),
                    )
                })
        })
        .collect();
    let mut document = scraper::Html::parse_document(&source);
    let removed: Vec<_> = document.select(&scraper::Selector::parse("script,iframe,frame,frameset,object,embed,base,meta,link,svg,math,audio,video,source,track,input,button,textarea,select,template").unwrap()).map(|n|n.id()).collect();
    for id in removed {
        if let Some(mut node) = document.tree.get_mut(id) {
            node.detach();
        }
    }
    let ids: Vec<_> = document.tree.nodes().map(|n| n.id()).collect();
    for id in ids {
        if let Some(mut node) = document.tree.get_mut(id)
            && let scraper::Node::Element(element) = node.value()
        {
            let img = element.name.local.as_ref() == "img";
            // Missing images have no broken-image glyph or retained tracking URL.
            let missing_image = img
                && element
                    .attr("src")
                    .is_none_or(|src| !images.contains_key(resource_key(src).as_str()));
            element.attrs.retain_mut(|(name, value)| {
                let name = name.local.as_ref();
                if name.starts_with("on")
                    || matches!(
                        name,
                        "srcdoc"
                            | "srcset"
                            | "action"
                            | "formaction"
                            | "target"
                            | "ping"
                            | "autofocus"
                            | "contenteditable"
                            | "nonce"
                            | "is"
                    )
                {
                    return false;
                }
                if name == "href" {
                    return url::Url::parse(value)
                        .ok()
                        .is_some_and(|url| matches!(url.scheme(), "https" | "http" | "mailto"));
                }
                if matches!(name, "src" | "background" | "poster" | "data") {
                    if (img && name == "src" || name == "background")
                        && let Some(data) = images.get(&resource_key(value))
                    {
                        *value = data.as_str().into();
                        return true;
                    }
                    return false;
                }
                true
            });
            if missing_image {
                // Omit missing placeholders instead of printing broken-image icons.
                element.name.local = "span".into();
                element
                    .attrs
                    .retain(|(name, _)| matches!(name.local.as_ref(), "alt"));
            }
        }
    }
    let styles = document
        .select(&scraper::Selector::parse("head style").unwrap())
        .map(|n| n.html())
        .collect::<String>();
    let body = document
        .select(&scraper::Selector::parse("body").unwrap())
        .next()
        .unwrap();
    let attributes = body
        .value()
        .attrs()
        .map(|(k, v)| format!(" {k}=\"{}\"", escape(v)))
        .collect::<String>();
    let child = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'\"><meta name=\"referrer\" content=\"no-referrer\"><title>Shep message</title><style>html{{color-scheme:light}}body{{margin:24px}}img{{max-width:100%;height:auto}}@media print{{body{{margin:0}}img,table{{max-width:100%}}}}@page{{margin:15mm}}</style>{styles}</head><body{attributes}>{}</body></html>",
        body.inner_html()
    );
    // srcdoc is attribute-escaped. The parent is entirely trusted; the child
    // has no scripts, forms, navigation permissions or external resource loads.
    let header_json = serde_json::to_string(&(headers, files))?.replace('<', "\\u003c");
    let child = escape(&child);
    let markers = regex::Regex::new(r"\{\{(NONCE|HEADERS|DOCUMENT)\}\}").unwrap();
    Ok(markers
        .replace_all(
            include_str!("preview.html"),
            |capture: &regex::Captures<'_>| {
                match &capture[1] {
                    "NONCE" => nonce,
                    "HEADERS" => &header_json,
                    _ => &child,
                }
                .to_owned()
            },
        )
        .into_owned())
}

fn resource_key(value: &str) -> String {
    let value = value.trim();
    if value
        .get(..4)
        .is_some_and(|s| s.eq_ignore_ascii_case("cid:"))
    {
        format!(
            "cid:{}",
            percent_encoding::percent_decode_str(&value[4..]).decode_utf8_lossy()
        )
    } else {
        value.into()
    }
}
