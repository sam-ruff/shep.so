#[test]
fn list_attachment_counts_match_the_shared_download_catalog() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../attachment-fixtures.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let raw = case["raw"].as_str().unwrap().as_bytes().to_vec();
        let files = shep_mail_core::attachments::catalog(&raw).unwrap();
        let mail =
            shep_mail_core::model::parse_mail("fixture", "files", "INBOX", raw, false, false)
                .unwrap();
        assert_eq!(mail.summary.attachment_count, files.len());
    }
}
