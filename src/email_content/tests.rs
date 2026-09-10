use super::*;
fn read(raw: &str) -> Content {
    extract(&shep_mail_core::mime::parse(raw.as_bytes()).unwrap()).unwrap()
}
#[test]
fn mislabeled_and_escaped_xhtml_render_as_content_but_prose_and_code_stay_text() {
    let html = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Example</title></head><body><h1>Verify your example account</h1><p>Fictional code: SAMPLE-ONLY</p></body></html>";
    for source in [
        html.to_owned(),
        escape(html),
        format!("<?xml version=\"1.0\"?>{html}"),
    ] {
        let content = read(&format!(
            "Content-Type: text/plain; charset=utf-8\r\n\r\n{source}"
        ));
        assert!(content.html.is_some());
        assert!(content.text.contains("SAMPLE-ONLY"));
        assert!(!content.text.contains("<html"));
        assert!(!content.text.contains("xmlns"));
    }
    for body in [
        "Write <b>bold</b> in this template.",
        "<htmlish>Example</html>",
        "<div>A code fragment</div>",
        "Literal &lt;b&gt;tag&lt;/b&gt;",
        "2 < 3 & 4 > 1",
    ] {
        let content = read(&format!("Content-Type: text/plain\r\n\r\n{body}"));
        assert!(content.html.is_none(), "{body}");
        assert_eq!(content.text, body);
    }
}
#[test]
fn alternatives_keep_plain_option_and_last_html_without_duplicate_body() {
    let content = read(
        "Content-Type: multipart/alternative; boundary=choice\r\n\r\n--choice\r\nContent-Type: text/plain\r\n\r\nPlain option\r\n--choice\r\nContent-Type: text/html\r\n\r\n<p>Earlier HTML</p>\r\n--choice\r\nContent-Type: text/html\r\n\r\n<table><tr><td>Styled option</td></tr></table>\r\n--choice--\r\n",
    );
    assert_eq!(content.text.trim(), "Plain option");
    let html = content.html.unwrap();
    assert!(html.source.contains("Styled option"));
    assert!(!html.source.contains("Earlier HTML"));
    assert!(!html.source.contains("Plain option"));
}
#[test]
fn related_start_selects_body_and_keeps_inline_image_out_of_attachment_bar() {
    let content = read(
        "Content-Type: multipart/related; boundary=related; start=\"<body@example>\"\r\n\r\n--related\r\nContent-Type: image/png\r\nContent-ID: <logo@example>\r\nContent-Disposition: inline; filename=logo.png\r\nContent-Transfer-Encoding: base64\r\n\r\ncGljdHVyZQ==\r\n--related\r\nContent-Type: text/html\r\nContent-ID: <body@example>\r\n\r\n<html><body><img src=\"cid:logo@example\"><p>Letter</p></body></html>\r\n--related\r\nContent-Type: text/html\r\nContent-ID: <other@example>\r\n\r\n<p>Unrelated resource</p>\r\n--related--\r\n",
    );
    assert_eq!(content.text.trim(), "Letter");
    assert!(content.attachments.is_empty());
    let html = content.html.unwrap();
    let doc = scraper::Html::parse_document(&html.source);
    let src = doc
        .select(&scraper::Selector::parse("img").unwrap())
        .next()
        .unwrap()
        .attr("src")
        .unwrap();
    assert_eq!(&*html.inline[src.strip_prefix("cid:").unwrap()], b"picture");
    assert!(!html.source.contains("Unrelated"));
}
#[test]
fn attached_html_is_never_the_message_and_empty_plain_falls_back_to_html() {
    let content = read(
        "Content-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: multipart/alternative; boundary=a\r\n\r\n--a\r\nContent-Type: text/plain\r\n\r\n \r\n--a\r\nContent-Type: application/xhtml+xml\r\n\r\n<html><body><p>Actual letter</p></body></html>\r\n--a--\r\n--m\r\nContent-Type: text/html\r\nContent-Disposition: attachment; filename=example.html\r\n\r\n<p>Attached document</p>\r\n--m--\r\n",
    );
    assert_eq!(content.text.trim(), "Actual letter");
    assert!(content.html.unwrap().source.contains("Actual letter"));
    assert_eq!(content.attachments.len(), 1);
    assert_eq!(content.attachments[0].name, "example.html");
    assert!(String::from_utf8_lossy(&content.attachments[0].bytes).contains("Attached document"));
}
