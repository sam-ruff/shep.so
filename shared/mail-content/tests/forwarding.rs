use serde_json::Value;
use shep_mail_content::forwarding::{MAX_ATTACHMENT_BYTES, MAX_ATTACHMENTS, prepare};

#[test]
fn complete_source_forward_contract_is_shared_with_browser_wasm() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../forward-fixtures.json")).unwrap();
    for case in cases {
        let result = prepare(case["raw"].as_str().unwrap().as_bytes()).unwrap();
        assert_eq!(result.subject, case["subject"], "{}", case["name"]);
        assert_eq!(
            result.forward.render(&result.body).is_some(),
            case["html"].as_bool().unwrap()
        );
        let retained = format!(
            "{}{}{}{}",
            result.body,
            result.forward.html_head,
            result.forward.html_attributes,
            result.forward.html_body
        );
        for text in case["contains"].as_array().unwrap() {
            assert!(
                retained.contains(text.as_str().unwrap()),
                "{} missing {text}",
                case["name"]
            );
        }
        for text in case["absent"].as_array().unwrap() {
            assert!(
                !retained.contains(text.as_str().unwrap()),
                "{} retained {text}",
                case["name"]
            );
        }
        let expected = case["files"].as_array().unwrap();
        assert_eq!(result.files.len(), expected.len());
        for (file, expected) in result.files.iter().zip(expected) {
            let mut metadata = expected.clone();
            metadata.as_object_mut().unwrap().remove("hex");
            assert_eq!(serde_json::to_value(file).unwrap(), metadata);
            let hex: String = file.bytes.iter().map(|b| format!("{b:02x}")).collect();
            assert_eq!(hex, expected["hex"]);
        }
        // Editing the original must never send the old hidden HTML quote.
        assert!(result.forward.render("Replacement quote.").is_none());
        if case["html"] == true {
            assert!(
                result
                    .forward
                    .render(&format!("Review <this> & reply.\n{}", result.body))
                    .unwrap()
                    .contains("Review &lt;this&gt; &amp; reply.")
            );
        }
    }
}

#[test]
fn complete_plain_body_is_not_a_reader_preview() {
    let body = format!("{}END OF SOURCE", "Long message. ".repeat(4000));
    let raw = format!("Subject: Full source\r\n\r\n{body}");
    let prepared = prepare(raw.as_bytes()).unwrap();
    assert!(prepared.body.ends_with(&body));
    assert!(prepared.body.len() > 32000);
    assert!(prepared.forward.render(&prepared.body).is_none());
}

fn broken(disposition: &str) -> Vec<u8> {
    format!("Content-Type: multipart/related; boundary=r\r\n\r\n--r\r\nContent-Type: text/html\r\n\r\n<p>Readable text</p><img src=cid:bad>\r\n--r\r\nContent-Type: image/png\r\nContent-ID: <bad>\r\nContent-Disposition: {disposition}; filename=bad.png\r\nContent-Transfer-Encoding: base64\r\n\r\nPRIVATE-invalid***\r\n--r--\r\n").into_bytes()
}

#[test]
fn corrupt_attachment_or_inline_resource_refuses_the_entire_forward() {
    for disposition in ["inline", "attachment"] {
        let error = prepare(&broken(disposition)).unwrap_err().to_string();
        assert!(error.contains("Could not decode"));
        assert!(!error.contains("PRIVATE"));
    }
    assert!(prepare(b"\r\nStill usable").is_ok());
}

#[test]
fn refuses_excess_files_bytes_and_nesting_before_returning_partial_content() {
    let mut wide = String::from("Content-Type: multipart/mixed; boundary=w\r\n\r\n");
    for _ in 0..=MAX_ATTACHMENTS {
        wide.push_str("--w\r\nContent-Disposition: attachment; filename=empty.txt\r\n\r\nx\r\n");
    }
    wide.push_str("--w--\r\n");
    assert!(
        prepare(wide.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("attachment limit")
    );
    let large = format!(
        "Content-Disposition: attachment; filename=large.bin\r\n\r\n{}",
        "x".repeat(MAX_ATTACHMENT_BYTES + 1)
    );
    assert!(
        prepare(large.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("attachment limit")
    );
    let mut deep = String::new();
    for i in 0..4096 {
        deep.push_str(&format!(
            "Content-Type: multipart/mixed; boundary=level{i:06}\r\n\r\n--level{i:06}\r\n"
        ));
    }
    deep.push_str("\r\nDeep text");
    assert!(
        prepare(deep.as_bytes())
            .unwrap_err()
            .to_string()
            .contains("nested")
    );
}
