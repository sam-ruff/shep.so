//! Complete, resource-confined print documents. All preparation is off-thread;
//! the platform owns printer/PDF selection and its completion semantics.
use crate::{document, reader::escape};
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use mailparse::MailHeaderMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const RUNTIME: &str = include_str!("printing/runtime.js");
pub fn runtime_csp_source() -> String {
    format!(
        "'sha256-{}'",
        STANDARD.encode(Sha256::digest(RUNTIME.as_bytes()))
    )
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Options {
    pub generation: String,
    #[serde(default)]
    pub plain: bool,
}

#[derive(Debug, Serialize)]
pub struct Prepared {
    pub signature: String,
    pub title: String,
    pub document: String,
    pub issues: Vec<String>,
}

pub fn prepare(raw: &[u8], options: &Options) -> Result<Prepared> {
    anyhow::ensure!(
        !options.generation.is_empty() && options.generation.len() <= 128,
        "Invalid print generation."
    );
    let parsed = crate::mime::parse(raw)?;
    let mut body = if options.plain {
        crate::reader::Body {
            text: crate::reader::text(&parsed)?,
            ..Default::default()
        }
    } else {
        crate::reader::extract(&parsed)?
    };
    let headers: Vec<_> = ["Subject", "From", "To", "Cc", "Date"]
        .into_iter()
        .filter_map(|name| {
            parsed
                .headers
                .get_first_value(name)
                .map(|value| (name, value.replace(['\r', '\n', '\0'], " ")))
        })
        .collect();
    let title = headers
        .iter()
        .find(|(name, _)| *name == "Subject")
        .map(|(_, value)| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Shep message")
        .chars()
        .take(200)
        .collect::<String>();
    let files: Vec<_> = crate::attachments::parts(&parsed)
        .enumerate()
        .map(|(index, part)| {
            let disposition = part.get_content_disposition();
            disposition
                .params
                .get("filename")
                .or_else(|| part.ctype.params.get("name"))
                .map(|name| crate::attachments::filename(name))
                .unwrap_or_else(|| format!("attachment-{}", index + 1))
        })
        .collect();
    if options.plain || body.html.is_empty() {
        // Plain mode prints the complete selected MIME text, never a cached
        // preview, a Find snippet or the collapsed quotation view.
        body.html = vec![crate::reader::RawHtmlPart {
            source: format!(
                "<pre style=\"white-space:pre-wrap;overflow-wrap:anywhere;font:14px/1.5 Arial,sans-serif\">{}</pre>",
                escape(&body.text)
            ),
            inline: Default::default(),
        }];
    }
    let document::Content {
        content, resources, ..
    } = document::sanitize(&body);
    let data = serde_json::json!({"generation": options.generation, "images": resources.images, "headers": headers, "files": files});
    let data = serde_json::to_string(&data)?.replace('<', "\\u003c");
    let runtime = runtime_csp_source();
    let csp = format!(
        "default-src 'none'; script-src {runtime}; style-src 'unsafe-inline'; img-src blob:; font-src 'none'; media-src 'none'; frame-src 'none'; connect-src 'none'; worker-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'"
    );
    let document = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"{csp}\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"referrer\" content=\"no-referrer\"><title>{}</title><style>html{{color-scheme:light}}body{{margin:24px;font:14px/1.5 Arial,sans-serif;overflow-wrap:break-word}}body:not([text]){{color:#18181b}}body:not([bgcolor]):not([background]){{background:#fff}}img{{max-width:100%;height:auto}}@media print{{body{{margin:0}}img,table{{max-width:100%}}}}@page{{margin:15mm}}</style></head><body><template id=\"shep-print-content\">{content}</template><script id=\"shep-print-data\" type=\"application/json\">{data}</script><script>{RUNTIME}</script></body></html>",
        escape(&title)
    );
    Ok(Prepared {
        signature: format!("{:x}", Sha256::digest(raw)),
        title,
        document,
        issues: resources.issues.into_iter().collect(),
    })
}
