//! Build forwards from the complete cached MIME, never the shortened reader.
use super::*;
use crate::email_content::escape;
use crate::model::*;
use anyhow::Context;
use mailparse::MailHeaderMap;

pub fn prepare_forward(
    id: String,
    account: String,
    raw: &[u8],
) -> anyhow::Result<(Draft, Vec<FilePart>)> {
    let parsed =
        shep_mail_core::mime::parse(raw).context("The original message could not be read.")?;
    // A readable cache may omit damaged optional resources, but forwarding must
    // not silently turn them into empty files or discard original formatting.
    let selected = shep_mail_core::reader::extract(&parsed)?;
    let mut attachments = Vec::new();
    for part in shep_mail_core::attachments::parts(&parsed) {
        let disposition = part.get_content_disposition();
        attachments.push(Attachment {
            name: disposition.params.get("filename").or_else(|| part.ctype.params.get("name"))
                .cloned().unwrap_or_else(|| "attachment.bin".into()),
            bytes: part.get_body_raw().context("An original attachment could not be decoded. Refresh this message and retry forwarding.")?,
        });
    }
    let content = crate::email_content::Content {
        text: selected.text.clone(),
        html: shep_mail_core::reader::flatten(selected).map(|flat| {
            crate::email_content::HtmlBody::new(flat.source, flat.inline.into_iter().collect())
        }),
        attachments,
    };
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
    let quoted = format!("{headers}\n{}", content.text);
    let forward = content.html.as_ref().map(|html| {
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
    for file in content.attachments {
        let media_type = attachment_type(&parsed, &file).unwrap_or_else(|| {
            mime_guess::from_path(&file.name)
                .first_or_octet_stream()
                .to_string()
        });
        files.push(FilePart {
            attachment: DraftAttachment {
                id: uuid::Uuid::new_v4().to_string(),
                name: file.name,
                media_type,
                size: file.bytes.len(),
                content_id: None,
            },
            bytes: file.bytes,
        });
    }
    if let Some(html) = content.html {
        let mut inline: Vec<_> = html.inline.into_iter().collect();
        inline.sort_by(|a, b| a.0.cmp(&b.0));
        for (cid, bytes) in inline {
            anyhow::ensure!(
                !cid.contains(['\r', '\n', '\0', '<', '>']) && cid.len() <= 998,
                "An inline image has an invalid Content-ID."
            );
            let format = image::guess_format(&bytes).ok();
            files.push(FilePart {
                attachment: DraftAttachment {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: format!(
                        "image-{}.{}",
                        files.len() + 1,
                        format.map(|f| f.extensions_str()[0]).unwrap_or("bin")
                    ),
                    media_type: inline_type(&parsed, &cid, &bytes).unwrap_or_else(|| {
                        format
                            .map(|f| f.to_mime_type())
                            .unwrap_or("application/octet-stream")
                            .into()
                    }),
                    size: bytes.len(),
                    content_id: Some(cid),
                },
                bytes: bytes.to_vec(),
            });
        }
    }
    anyhow::ensure!(
        files.len() <= MAX_ATTACHMENTS
            && files.iter().map(|f| f.bytes.len()).sum::<usize>() <= MAX_ATTACHMENT_BYTES,
        "This forward exceeds the current attachment limit. No partial draft was saved."
    );
    let draft = Draft {
        id,
        account_id: account,
        subject,
        body: format!("\n\n{quoted}"),
        forward: Some(forward.unwrap_or_else(|| ForwardQuote {
            text: quoted.clone(),
            html_head: String::new(),
            html_attributes: String::new(),
            html_body: String::new(),
        })),
        revision: 1,
        attachments: files.iter().map(|f| f.attachment.clone()).collect(),
        ..Default::default()
    };
    Ok((draft, files))
}

fn attachment_type(parsed: &mailparse::ParsedMail<'_>, expected: &Attachment) -> Option<String> {
    let disposition = parsed.get_content_disposition();
    if disposition
        .params
        .get("filename")
        .or_else(|| parsed.ctype.params.get("name"))
        == Some(&expected.name)
        && parsed.get_body_raw().ok().as_deref() == Some(&expected.bytes)
    {
        return Some(parsed.ctype.mimetype.clone());
    }
    parsed
        .subparts
        .iter()
        .find_map(|part| attachment_type(part, expected))
}

fn inline_type(parsed: &mailparse::ParsedMail<'_>, cid: &str, bytes: &[u8]) -> Option<String> {
    use sha2::{Digest, Sha256};
    if parsed
        .headers
        .get_first_value("Content-ID")
        .is_some_and(|id| {
            id.trim().trim_matches(['<', '>']) == cid
                || cid == format!("shep-{:x}@inline", Sha256::digest(bytes))
        })
        && parsed.get_body_raw().ok().as_deref() == Some(bytes)
    {
        return Some(parsed.ctype.mimetype.clone());
    }
    parsed
        .subparts
        .iter()
        .find_map(|part| inline_type(part, cid, bytes))
}
