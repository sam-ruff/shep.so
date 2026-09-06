use serde_json::{Value, json};
use shep_mail_core::{compose::ReplyHeaders, model::*};

#[test]
fn cached_envelope_and_reply_match_shared_browser_fixtures() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../compose-fixtures.json")).unwrap();
    for c in cases {
        let raw = c["raw"].as_str().unwrap().as_bytes();
        let headers = ReplyHeaders::parse(&mailparse::parse_mail(raw).unwrap());
        assert_eq!(
            serde_json::to_value(headers.envelope()).unwrap(),
            c["envelope"],
            "{}",
            c["name"]
        );
        let accounts: Vec<Account> = c["own"].as_array().unwrap().iter().enumerate().map(|(n,email)| serde_json::from_value(json!({"id":n.to_string(), "name":"Fixture", "email":email, "protocol":"Imap", "host":"mail.example.test", "port":993, "username":"fixture", "smtp_host":"mail.example.test", "smtp_port":465})).unwrap()).collect();
        let mut mail = parse_mail("fixture", "1.2", "INBOX", raw.to_vec(), false, false).unwrap();
        mail.summary.timestamp = c["timestamp"].as_i64().unwrap();
        mail.summary.sender = c["sender"].as_str().unwrap().into();
        mail.summary.subject = c["subject"].as_str().unwrap().into();
        let detail = MailDetail {
            summary: mail.summary,
            body: c["text"].as_str().unwrap().into(),
            latest_body: String::new(),
            body_truncated: false,
            remote_images: Vec::new(),
            replies: Vec::new(),
            attachments: Default::default(),
            reply: headers.clone(),
        };
        let draft = headers.draft(&detail, &accounts, c["all"].as_bool().unwrap());
        let value = serde_json::to_value(&draft).unwrap();
        for (key, expected) in c["expected"].as_object().unwrap() {
            assert_eq!(&value[key], expected, "{}: {key}", c["name"]);
        }
        assert!(draft.bcc.is_empty());
        assert!(draft.attachments.is_empty());
    }
}
