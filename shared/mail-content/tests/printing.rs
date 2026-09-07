use shep_mail_content::printing::{self, Options};

fn options(plain: bool) -> Options {
    Options {
        generation: "print-fixture".into(),
        plain,
    }
}
fn data(document: &str) -> serde_json::Value {
    let html = scraper::Html::parse_document(document);
    let json = html
        .select(&scraper::Selector::parse("#shep-print-data").unwrap())
        .next()
        .unwrap()
        .text()
        .collect::<String>();
    serde_json::from_str(&json).unwrap()
}

#[test]
fn complete_formatted_print_uses_selected_mime_resources_and_independent_headers() {
    let raw = include_bytes!("../../html-reader-fixture.eml");
    let prepared = printing::prepare(raw, &options(false)).unwrap();
    let meta = data(&prepared.document);
    assert_eq!(meta["generation"], "print-fixture");
    assert_eq!(meta["images"].as_object().unwrap().len(), 1);
    assert!(prepared.document.contains("table"));
    assert!(prepared.document.contains("Alpha in quoted history."));
    assert!(prepared.document.contains(&printing::runtime_csp_source()));
    assert!(!prepared.document.contains("images.example.test"));
    assert!(!prepared.document.contains("SCRIPT-FIXTURE"));
    assert!(!prepared.document.contains("href=\"https:"));
    assert!(prepared.issues.is_empty());
}

#[test]
fn print_headers_and_filenames_are_data_not_markup_or_private_thread_headers() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../forward-fixtures.json")).unwrap();
    let raw = fixture[0]["raw"].as_str().unwrap().replace(
        "aW5saW5lIGZpeHR1cmU=",
        "UklGRiIAAABXRUJQVlA4IBYAAAAwAQCdASoBAAEADsD+JaQAA3AAAAAA",
    );
    let prepared = printing::prepare(raw.as_bytes(), &options(false)).unwrap();
    let meta = data(&prepared.document);
    assert_eq!(prepared.title, "Café project");
    assert_eq!(
        meta["files"],
        serde_json::json!(["duplicate.bin", "duplicate.bin"])
    );
    assert!(!prepared.document.contains("private@example.test"));
    assert!(!prepared.document.contains("original-thread"));
    let hostile = b"Subject: </title><script>ATTACK()</script>\r\nFrom: <style>body{display:none}</style>\r\nBcc: secret@example.test\r\n\r\nReal body";
    let prepared = printing::prepare(hostile, &options(true)).unwrap();
    assert_eq!(
        data(&prepared.document)["headers"][0][1],
        "</title><script>ATTACK()</script>"
    );
    let parsed = scraper::Html::parse_document(&prepared.document);
    assert_eq!(
        parsed
            .select(&scraper::Selector::parse("script").unwrap())
            .count(),
        2
    );
    assert!(!prepared.document.contains("secret@example.test"));
}

#[test]
fn plain_print_is_complete_and_ignores_unneeded_damaged_inline_resources() {
    let text = format!("{}\nCOMPLETE PRINT END", "Printable line. ".repeat(3000));
    let raw = format!("Subject: Long print\r\n\r\n{text}");
    let prepared = printing::prepare(raw.as_bytes(), &options(true)).unwrap();
    assert!(prepared.document.contains(&text));
    let damaged = b"Content-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<p>Readable without images</p><img src='cid:broken'>\r\n--x\r\nContent-Type: image/png\r\nContent-ID: <broken>\r\nContent-Transfer-Encoding: base64\r\n\r\nPRIVATE-invalid***\r\n--x--\r\n";
    assert!(printing::prepare(damaged, &options(false)).is_err());
    let plain = printing::prepare(damaged, &options(true)).unwrap();
    assert!(plain.document.contains("Readable without images"));
    assert!(
        data(&plain.document)["images"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(!plain.document.contains("PRIVATE-invalid"));
}
