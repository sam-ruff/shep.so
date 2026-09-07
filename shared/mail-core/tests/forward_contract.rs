use mailparse::MailHeaderMap;
use serde_json::{Value, json};
use shep_mail_core::{attachments, compose, mime, model::Account, reader};

#[test]
fn prepared_forward_owns_new_files_and_a_new_thread_with_a_reserved_smtp_identity() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../forward-fixtures.json")).unwrap();
    let raw = cases[0]["raw"].as_str().unwrap().as_bytes();
    let account: Account = serde_json::from_value(json!({"id":"work", "name":"Fixture",
        "email":"sender@example.test", "protocol":"Imap", "host":"mail.example.test",
        "port":993, "username":"fixture", "smtp_host":"mail.example.test", "smtp_port":465}))
    .unwrap();
    let (first, _) = compose::prepare_forward("first".into(), "work".into(), raw).unwrap();
    for edit_quote in [false, true] {
        let (mut draft, files) =
            compose::prepare_forward("second".into(), "work".into(), raw).unwrap();
        assert_eq!(draft.id, "second");
        assert_eq!(draft.account_id, "work");
        assert!(draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty());
        assert!(draft.in_reply_to.is_none() && draft.references.is_empty());
        for file in &draft.attachments {
            assert!(!first.attachments.iter().any(|f| f.id == file.id));
        }
        draft.to = "new-recipient@example.test".into();
        draft.body.insert_str(0, "Review <this> & reply.\n");
        if edit_quote {
            draft.body = "Edited original quotation.".into();
        }
        let wire = compose::build_with_message_id(
            &account,
            &draft,
            files,
            "<reserved-forward@example.test>",
        )
        .unwrap()
        .formatted();
        let parsed = mime::parse(&wire).unwrap();
        assert_eq!(
            parsed.headers.get_first_value("Message-ID").as_deref(),
            Some("<reserved-forward@example.test>")
        );
        for header in ["References", "In-Reply-To", "Bcc"] {
            assert!(parsed.headers.get_first_value(header).is_none());
        }
        let body = reader::extract(&parsed).unwrap();
        assert_eq!(body.html.is_empty(), edit_quote);
        if edit_quote {
            assert!(body.text.contains("Edited original quotation."));
            assert!(!body.text.contains("Complete café."));
        } else {
            assert!(body.text.contains("Review <this> & reply."));
            assert_eq!(body.resources.values().next().unwrap(), b"inline fixture");
        }
        let mut decoded = Vec::new();
        attachments::decoded_parts(&parsed, |info, bytes| decoded.push((info, bytes))).unwrap();
        assert_eq!(decoded.len(), if edit_quote { 3 } else { 2 });
        for (index, media) in ["application/x-first", "application/x-second"]
            .iter()
            .enumerate()
        {
            assert_eq!(decoded[index].0.media_type, *media);
            assert_eq!(decoded[index].1, [0, 255, 1, 13, 10]);
        }
        if edit_quote {
            assert_eq!(decoded[2].1, b"inline fixture");
        }
    }
}
