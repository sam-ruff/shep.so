use serde_json::Value;
use shep_mail_content::{attachments, mime, reader};

#[test]
fn shared_representation_and_cid_fixtures() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("../../reader-fixtures.json")).unwrap();
    for case in fixtures {
        let raw = case["raw"].as_str().unwrap().as_bytes();
        assert_eq!(
            serde_json::to_value(reader::decode(raw).unwrap()).unwrap(),
            case["body"],
            "{}",
            case["name"]
        );
        let files: Vec<_> = attachments::catalog(raw)
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(
            serde_json::to_value(files).unwrap(),
            case["files"],
            "{}",
            case["name"]
        );
    }
}

fn nested(depth: usize, ending: &str) -> Vec<u8> {
    let mut raw = String::new();
    for index in 0..depth {
        raw.push_str(&format!("Content-Type: multipart/mixed; boundary=level{index:06}{ending}{ending}--level{index:06}{ending}"));
    }
    raw.push_str(&format!(
        "Content-Type: text/plain{ending}{ending}Deep text"
    ));
    for index in (0..depth).rev() {
        raw.push_str(&format!("{ending}--level{index:06}--{ending}"));
    }
    raw.into_bytes()
}

#[test]
fn deeply_nested_input_is_refused_before_recursive_parse_on_a_small_stack() {
    let raw = nested(4096, "\r\n");
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            assert!(
                mime::parse(&raw)
                    .unwrap_err()
                    .to_string()
                    .contains("nested")
            );
            assert!(
                attachments::catalog(&raw)
                    .unwrap_err()
                    .to_string()
                    .contains("nested")
            );
            assert!(
                reader::decode(&raw)
                    .unwrap_err()
                    .to_string()
                    .contains("nested")
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn boundary_preflight_retains_mailparse_permissive_behavior() {
    for ending in ["\n", "\r\n"] {
        for depth in [0, 1, 2, 16, mime::MAX_DEPTH] {
            let raw = nested(depth, ending);
            let ours = mime::parse(&raw).unwrap();
            let upstream = mailparse::parse_mail(&raw).unwrap();
            assert_eq!(format!("{ours:?}"), format!("{upstream:?}"));
            assert_eq!(reader::extract(&ours).unwrap().text, "Deep text");
        }
    }
    for raw in [
        "Content-Type: multipart/mixed; boundary=x\n\n--x\n\nNo close",
        "Content-Type: multipart/mixed; boundary=x\n\n--x-suffix\n\nPermissive prefix\n--x--\n",
        "Content-Type: multipart/mixed; boundary=\"\"\n\n--\n\nEmpty boundary\n----\n",
        "Content-Type: multipart/digest; boundary=d\n\n--d\n\nSubject: inner\n\nAttached message\n--d--\n",
        "Content-Type: multipart/mixed; boundary=x\n\n--x--\nEpilogue\n",
        "Content-Type: multipart/mixed; boundary=x\n\n--x",
        "Content-Type: multipart/mixed; boundary=x\n\n--x\n\n--x\n\n--x--\n",
    ] {
        assert_eq!(
            format!("{:?}", mime::parse(raw.as_bytes()).unwrap()),
            format!("{:?}", mailparse::parse_mail(raw.as_bytes()).unwrap())
        );
    }
}

#[test]
fn malformed_transfer_has_a_controlled_error_without_sender_data() {
    let raw = b"Content-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\nPRIVATE-unparseable***";
    let error = reader::decode(raw).unwrap_err().to_string();
    assert!(error.contains("Could not decode"));
    assert!(!error.contains("PRIVATE"));
}

#[test]
fn html_nesting_uses_iterative_plain_text_fallback() {
    let html = format!(
        "Content-Type: text/html\r\n\r\n{}<p>Deep HTML</p>{}",
        "<div>".repeat(4096),
        "</div>".repeat(4096)
    );
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            assert_eq!(reader::decode(html.as_bytes()).unwrap().text, "Deep HTML");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn repeated_section_references_reuse_one_payload() {
    let mut raw = "Content-Type: multipart/related; boundary=outer\r\n\r\n--outer\r\nContent-Type: multipart/mixed; boundary=inner\r\n\r\n".to_owned();
    for _ in 0..1000 {
        raw.push_str("--inner\r\nContent-Type: text/html\r\n\r\n<p>Text</p><img src=cid:logo>\r\n");
    }
    raw.push_str("--inner--\r\n--outer\r\nContent-Type: image/webp\r\nContent-ID: <logo>\r\n\r\n");
    raw.push_str(&"X".repeat(1024 * 1024));
    raw.push_str("\r\n--outer--\r\n");
    let body = reader::decode(raw.as_bytes()).unwrap();
    assert_eq!(body.html.len(), 1000);
    assert_eq!(body.resources.len(), 1);
    assert_eq!(body.resources.values().next().unwrap().len(), 1024 * 1024);
    let key = body.resources.keys().next().unwrap();
    assert!(
        body.html
            .iter()
            .all(|part| part.inline["logo"].as_ref() == Some(key))
    );
}

#[test]
fn corrupt_optional_images_do_not_block_cache_text_or_replies() {
    let raw = b"Content-Type: multipart/related; boundary=r\r\n\r\n--r\r\nContent-Type: text/html\r\n\r\n<p>Readable text</p>\r\n--r\r\nContent-Type: image/webp\r\nContent-ID: <logo>\r\nContent-Transfer-Encoding: base64\r\n\r\n%%%bad%%%\r\n--r--\r\n";
    assert_eq!(
        reader::text(&mime::parse(raw).unwrap()).unwrap(),
        "Readable text"
    );
    assert!(
        reader::decode(raw)
            .unwrap_err()
            .to_string()
            .contains("inline image")
    );
}
