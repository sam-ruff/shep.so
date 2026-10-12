use anyhow::Context;
use serde_json::{Value, json};
use shep_mail_core::{compose::ReplyHeaders, model::*};

#[test]
fn reply_quote_choice_keeps_thread_headers_and_private_envelope() -> anyhow::Result<()> {
    let account: Account = serde_json::from_value(
        json!({"id":"fixture", "name":"Fixture", "email":"owner@example.test", "protocol":"Pop3", "host":"mail.example.test", "port":995, "username":"owner", "smtp_host":"mail.example.test", "smtp_port":465}),
    )?;
    let mut draft = Draft {
        id: "reply".into(),
        account_id: account.id.clone(),
        to: "sender@example.test".into(),
        bcc: "private@example.test".into(),
        subject: "Re: Plan".into(),
        body: "Typed answer".into(),
        in_reply_to: Some("<original@example.test>".into()),
        references: vec![
            "<root@example.test>".into(),
            "<original@example.test>".into(),
        ],
        reply_context: Some(ReplyContext {
            account_id: account.id.clone(),
            mail_id: "original".into(),
            quote: "\n\nOn yesterday, Sender wrote:\n> Original text".into(),
            include_quote: true,
        }),
        ..Default::default()
    };
    for include in [true, false, true] {
        draft
            .reply_context
            .as_mut()
            .context("Reply context")?
            .include_quote = include;
        let message = shep_mail_core::compose::build_with_message_id(
            &account,
            &draft,
            vec![],
            "<reply@example.test>",
        )?;
        let bytes = message.formatted();
        let parsed = mailparse::parse_mail(&bytes)?;
        let body = parsed.get_body()?;
        assert!(body.contains("Typed answer"));
        assert_eq!(body.contains("Original text"), include);
        assert!(!String::from_utf8_lossy(&bytes).contains("Bcc:"));
        assert_eq!(message.envelope().to().len(), 2);
        assert!(String::from_utf8_lossy(&bytes).contains("In-Reply-To: <original@example.test>"));
        assert!(String::from_utf8_lossy(&bytes).contains("<root@example.test>"));
        assert_eq!(draft.body, "Typed answer");
    }
    Ok(())
}

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
        let detail = MailDetail::<()> {
            html: None,
            summary: mail.summary,
            body: c["text"].as_str().unwrap().into(),
            latest_body: String::new(),
            body_truncated: false,
            body_limit: c["text"].as_str().unwrap().chars().count(),
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
