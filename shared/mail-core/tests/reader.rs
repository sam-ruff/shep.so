use shep_mail_core::{compose, model};

#[test]
fn provider_cache_and_reply_use_the_selected_plain_representation_and_file_count() {
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../reader-fixtures.json")).unwrap();
    for case in fixtures {
        let raw = format!("From: Sender <sender@example.test>\r\nTo: User <user@example.test>\r\nSubject: MIME fixture\r\n{}",case["raw"].as_str().unwrap()).into_bytes();
        let mail = model::parse_mail("fixture", "1", "INBOX", raw.clone(), true, false).unwrap();
        assert_eq!(
            mail.text,
            case["body"]["text"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            mail.summary.attachment_count,
            case["files"].as_array().unwrap().len()
        );
        let reply = compose::reply_from_raw(mail.summary, &raw, &[], false).unwrap();
        for line in mail.text.lines().filter(|line| !line.is_empty()) {
            assert!(reply.body.contains(line), "{}: {}", case["name"], line);
        }
    }
}
