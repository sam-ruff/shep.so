use serde_json::{Value, json};
use shep_mail_core::{attachments, compose, mime, model::*, reader};

fn account() -> Account {
    serde_json::from_value(json!({"id":"work", "name":"Fixture",
        "email":"sender@example.test", "protocol":"Imap", "host":"mail.example.test",
        "port":993, "username":"fixture", "smtp_host":"mail.example.test", "smtp_port":465}))
    .expect("fixture account")
}

fn draft(body: &str, html: Option<&str>) -> Draft {
    Draft {
        id: "formatted".into(),
        account_id: "work".into(),
        to: "reader@example.test".into(),
        subject: "Plans".into(),
        body: body.into(),
        body_html: html.map(Into::into),
        ..Default::default()
    }
}

struct Sent {
    text: String,
    html: Vec<String>,
    kind: String,
}

fn sent(draft: &Draft, files: Vec<compose::FilePart>) -> Sent {
    let wire = compose::build_with_message_id(&account(), draft, files, "<f@example.test>")
        .expect("build")
        .formatted();
    let parsed = mime::parse(&wire).expect("parse");
    let body = reader::extract(&parsed).expect("extract");
    Sent {
        text: body.text.replace("\r\n", "\n").trim_end().to_owned(),
        html: body.html.into_iter().map(|part| part.source).collect(),
        kind: parsed.ctype.mimetype.clone(),
    }
}

const BOLD: &str = "<div style=\"white-space:pre-wrap\"><div><b>Bring</b> notes</div></div>";

#[test]
fn formatted_body_sends_html_with_its_plain_alternative() {
    let sent = sent(&draft("*Bring* notes", Some(BOLD)), vec![]);
    assert_eq!(sent.kind, "multipart/alternative");
    assert_eq!(sent.text, "*Bring* notes");
    assert_eq!(sent.html.len(), 1);
    assert!(sent.html[0].contains(BOLD), "{}", sent.html[0]);
    assert!(sent.html[0].starts_with("<!doctype html>"));
}

#[test]
fn an_unformatted_draft_stays_single_part_text() {
    let sent = sent(&draft("Typed *stars* stay as they are", None), vec![]);
    assert_eq!(sent.kind, "text/plain");
    assert!(sent.html.is_empty());
    assert_eq!(sent.text, "Typed *stars* stay as they are");
}

#[test]
fn a_reply_quote_is_sent_exactly_as_plain_text_in_both_parts() {
    let mut reply = draft("*Bring* notes", Some(BOLD));
    reply.reply_context = Some(ReplyContext {
        account_id: "work".into(),
        mail_id: "original".into(),
        quote: "\n\nOn Monday, Sender wrote:\n> Use <b>tags</b> & *stars*".into(),
        include_quote: true,
    });
    let with_quote = sent(&reply, vec![]);
    assert_eq!(
        with_quote.text,
        "*Bring* notes\n\nOn Monday, Sender wrote:\n> Use <b>tags</b> & *stars*"
    );
    let html = &with_quote.html[0];
    assert!(html.contains(BOLD));
    assert!(
        html.replace("\r\n", "\n").contains(
            "<div style=\"white-space:pre-wrap\">\n\nOn Monday, Sender wrote:\n&gt; Use &lt;b&gt;tags&lt;/b&gt; &amp; *stars*</div>"
        ),
        "{html}"
    );
    reply.reply_context.as_mut().expect("context").include_quote = false;
    let without = sent(&reply, vec![]);
    assert!(!without.text.contains("Sender wrote"));
    assert!(!without.html[0].contains("Sender wrote"));
}

#[test]
fn formatted_mail_keeps_attachments_beside_the_alternative() {
    let mut message = draft("*Bring* notes", Some(BOLD));
    let file = DraftAttachment {
        id: "file".into(),
        name: "notes.txt".into(),
        media_type: "text/plain".into(),
        size: 5,
        content_id: None,
    };
    message.attachments = vec![file.clone()];
    let wire = compose::build_with_message_id(
        &account(),
        &message,
        vec![compose::FilePart {
            attachment: file,
            bytes: b"hello".to_vec(),
        }],
        "<f@example.test>",
    )
    .expect("build")
    .formatted();
    let parsed = mime::parse(&wire).expect("parse");
    assert_eq!(parsed.ctype.mimetype, "multipart/mixed");
    assert_eq!(parsed.subparts[0].ctype.mimetype, "multipart/alternative");
    let body = reader::extract(&parsed).expect("extract");
    assert!(body.html[0].source.contains(BOLD));
    let mut decoded = Vec::new();
    attachments::decoded_parts(&parsed, |info, bytes| decoded.push((info.name, bytes)))
        .expect("attachments");
    assert_eq!(decoded, [("notes.txt".to_owned(), b"hello".to_vec())]);
}

fn forward() -> (Draft, Vec<compose::FilePart>) {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../forward-fixtures.json")).expect("fixtures");
    let raw = cases[0]["raw"].as_str().expect("raw").as_bytes();
    let (mut forward, files) =
        compose::prepare_forward("fwd".into(), "work".into(), raw).expect("forward");
    forward.to = "reader@example.test".into();
    (forward, files)
}

#[test]
fn a_formatted_note_keeps_the_forwarded_html_and_its_inline_images() {
    let (mut forward, files) = forward();
    forward.body.insert_str(0, "*Please review*");
    let note = "<div style=\"white-space:pre-wrap\"><div><b>Please review</b></div></div>";
    forward.body_html = Some(note.into());
    assert_eq!(forward.forwarded_note(), Some("*Please review*\n\n"));
    let wire = compose::build_with_message_id(&account(), &forward, files, "<f@example.test>")
        .expect("build")
        .formatted();
    let parsed = mime::parse(&wire).expect("parse");
    let body = reader::extract(&parsed).expect("extract");
    assert!(body.text.starts_with("*Please review*"));
    assert!(body.text.contains("Complete café."));
    let html = &body.html[0].source;
    assert!(html.contains(note));
    assert!(html.contains("Forwarded message"));
    assert_eq!(
        body.resources.values().next().map(Vec::as_slice),
        Some(&b"inline fixture"[..])
    );
}

#[test]
fn an_edited_forward_sends_the_written_html_and_attaches_its_images() {
    let (mut forward, files) = forward();
    forward.body = "*Edited* original".into();
    let written = "<div style=\"white-space:pre-wrap\"><div><b>Edited</b> original</div></div>";
    forward.body_html = Some(written.into());
    assert_eq!(forward.forwarded_note(), None);
    let count = files.len();
    let wire = compose::build_with_message_id(&account(), &forward, files, "<f@example.test>")
        .expect("build")
        .formatted();
    let parsed = mime::parse(&wire).expect("parse");
    let body = reader::extract(&parsed).expect("extract");
    assert_eq!(body.text.trim_end(), "*Edited* original");
    assert!(body.html[0].source.contains(written));
    assert!(!body.html[0].source.contains("Forwarded message"));
    let mut decoded = 0;
    attachments::decoded_parts(&parsed, |_, _| decoded += 1).expect("attachments");
    assert_eq!(decoded, count);
}

#[test]
fn a_note_must_end_its_line_before_the_original() {
    let (mut forward, _) = forward();
    let original = forward.body.trim_start_matches('\n').to_owned();
    forward.body = format!("Note{original}");
    assert_eq!(forward.forwarded_note(), None);
    forward.body = original;
    assert_eq!(forward.forwarded_note(), Some(""));
}

#[test]
fn a_formatted_note_above_a_plain_forward_shows_the_original_as_text() {
    let (mut forward, _) = forward();
    let quote = forward.forward.as_mut().expect("quote");
    quote.html_body.clear();
    forward.body.insert_str(0, "*Please review*");
    forward.body_html = Some("<div><b>Please review</b></div>".into());
    let delivery = forward.delivery();
    assert!(!delivery.forwarded_html);
    let html = delivery.html.expect("html");
    assert!(html.contains("<div><b>Please review</b></div><div style=\"white-space:pre-wrap\">"));
    assert!(html.contains("Forwarded message"));
}

#[test]
fn body_html_is_omitted_for_plain_drafts_and_read_back_when_formatted() {
    let plain = serde_json::to_value(draft("x", None)).expect("plain");
    assert!(plain.get("body_html").is_none());
    let formatted = draft("*x*", Some("<b>x</b>"));
    let saved = serde_json::to_value(&formatted).expect("formatted");
    assert_eq!(saved["body_html"], "<b>x</b>");
    let read: Draft = serde_json::from_value(saved).expect("read");
    assert_eq!(read, formatted);
    let legacy: Draft = serde_json::from_value(plain).expect("legacy");
    assert_eq!(legacy.body_html, None);
}
