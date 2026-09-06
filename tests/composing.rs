use mailparse::MailHeaderMap;
use shep::{compose, model::*, store::Store};

fn account() -> Account {
    serde_json::from_value(serde_json::json!({"id":"work", "name":"Work", "email":"alex@example.com", "protocol":"Imap", "host":"imap.example.com", "port":993, "username":"alex", "smtp_host":"smtp.example.com", "smtp_port":465})).unwrap()
}
fn draft() -> Draft {
    Draft {
        id: "draft-1".into(),
        account_id: "work".into(),
        to: "\"Friend, First\" <friend@example.com>".into(),
        subject: "Planning café".into(),
        body: "First line\n.Second line".into(),
        revision: 1,
        ..Default::default()
    }
}

#[test]
fn recipients_and_bcc_use_an_envelope_without_exposing_hidden_addresses() {
    let mut draft = draft();
    draft.cc = "Copied <copy@example.com>".into();
    draft.bcc = "hidden@example.com, FRIEND@example.com".into();
    let message = compose::build(&account(), &draft, vec![]).unwrap();
    let recipients: Vec<_> = message
        .envelope()
        .to()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        recipients,
        [
            "friend@example.com",
            "copy@example.com",
            "hidden@example.com"
        ]
    );
    let raw = message.formatted();
    let mail = mailparse::parse_mail(&raw).unwrap();
    assert!(mail.headers.get_first_value("Bcc").is_none());
    assert!(!String::from_utf8_lossy(&raw).contains("hidden@example.com"));
    assert_eq!(
        mail.get_body().unwrap().replace("\r\n", "\n"),
        format!("{}\n", draft.body)
    );
    draft.to.clear();
    draft.cc.clear();
    assert!(
        compose::build(&account(), &draft, vec![]).is_ok(),
        "Bcc-only mail is valid"
    );
    draft.bcc = "bad address".into();
    assert!(
        compose::build(&account(), &draft, vec![])
            .unwrap_err()
            .to_string()
            .contains("Bcc")
    );
    draft.bcc = "hidden@example.com\r\nX-Injected: true".into();
    assert!(compose::build(&account(), &draft, vec![]).is_err());
}

#[tokio::test]
async fn reply_all_honors_reply_to_excludes_self_and_preserves_thread_headers() {
    let store = Store::memory().unwrap();
    let mail = parse_mail("work", "1", "INBOX", b"From: Author <author@example.com>\r\nReply-To: Team <team@example.com>\r\nTo: alex@example.com, colleague@example.com\r\nCc: COLLEAGUE@example.com, copy@example.com\r\nBcc: secret@example.com\r\nMessage-ID: <parent@example.com>\r\nReferences: <root@example.com>\r\nSubject: RE: Planning\r\n\r\nOriginal body".to_vec(), false, false).unwrap();
    let id = mail.summary.id.clone();
    store.upsert(vec![mail]).await.unwrap();
    let detail = store.detail(id).await.unwrap();
    let reply = detail.reply.draft(&detail, &[account()], false);
    assert_eq!(reply.to, "Team <team@example.com>");
    assert!(reply.cc.is_empty());
    let reply = detail.reply.draft(&detail, &[account()], true);
    assert_eq!(reply.to, "Team <team@example.com>, colleague@example.com");
    assert_eq!(reply.cc, "copy@example.com");
    assert!(reply.bcc.is_empty());
    assert_eq!(reply.subject, "RE: Planning");
    assert!(reply.body.contains("> Original body"));
    let wire = compose::build(&account(), &reply, vec![])
        .unwrap()
        .formatted();
    let mail = mailparse::parse_mail(&wire).unwrap();
    assert_eq!(
        mail.headers.get_first_value("In-Reply-To").as_deref(),
        Some("<parent@example.com>")
    );
    assert_eq!(
        mail.headers.get_first_value("References").as_deref(),
        Some("<root@example.com> <parent@example.com>")
    );
}

#[tokio::test]
async fn cached_attachments_survive_reopen_and_old_autosaves_without_copying_file_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drafts.sqlite");
    let file = dir.path().join("café notes.bin");
    let bytes = vec![0, 255, 1, 128, 13, 10];
    std::fs::write(&file, &bytes).unwrap();
    let store = Store::open(&path).unwrap();
    let old = draft();
    let mut latest = old.clone();
    latest.body = "Newer text".into();
    latest.revision += 1;
    store.save_draft(latest.clone()).await.unwrap();
    let added = store
        .add_draft_files(old.clone(), vec![file.clone()])
        .await
        .unwrap();
    assert_eq!(added.drafts[0].body, latest.body);
    assert_eq!(added.drafts[0].attachments.len(), 1);
    store.save_draft(old).await.unwrap();
    std::fs::remove_file(file).unwrap();
    drop(store);
    let store = Store::open(path).unwrap();
    let snapshot = store.draft_state().await.unwrap();
    let saved = &snapshot.drafts[0];
    assert_eq!(saved.body, latest.body);
    assert!(
        !serde_json::to_string(saved)
            .unwrap()
            .contains("attachments"),
        "Text autosaves contain no file associations or bytes"
    );
    let files = store.draft_files(saved.clone()).await.unwrap();
    assert_eq!(files[0].bytes, bytes);
    let wire = compose::build(&account(), saved, files)
        .unwrap()
        .formatted();
    let mail = mailparse::parse_mail(&wire).unwrap();
    assert_eq!(mail.subparts.len(), 2);
    assert_eq!(mail.subparts[1].get_body_raw().unwrap(), bytes);
    assert_eq!(
        mail.subparts[1]
            .get_content_disposition()
            .params
            .get("filename")
            .map(String::as_str),
        Some("café notes.bin")
    );
    let removed = store
        .remove_draft_file(saved.id.clone(), saved.attachments[0].id.clone())
        .await
        .unwrap();
    store.save_draft(saved.clone()).await.unwrap();
    assert!(
        store.draft_state().await.unwrap().drafts[0]
            .attachments
            .is_empty()
    );
    assert!(removed.revision > snapshot.revision);
    assert!(store.draft_files(saved.clone()).await.is_err());
}

#[tokio::test]
async fn failed_file_import_is_atomic_and_sent_tombstones_reject_late_saves() {
    let store = Store::memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("one.txt");
    std::fs::write(&good, "file body").unwrap();
    let draft = draft();
    store.save_draft(draft.clone()).await.unwrap();
    assert!(
        store
            .add_draft_files(
                draft.clone(),
                vec![good.clone(), dir.path().join("missing")]
            )
            .await
            .is_err()
    );
    assert!(
        store.draft_state().await.unwrap().drafts[0]
            .attachments
            .is_empty()
    );
    let added = store
        .add_draft_files(draft.clone(), vec![good.clone()])
        .await
        .unwrap();
    let sent = added.drafts[0].clone();
    let finished = store.finish_draft_send(sent.clone()).await.unwrap();
    assert!(finished.drafts.is_empty());
    store.save_draft(sent.clone()).await.unwrap();
    assert!(store.draft_state().await.unwrap().drafts.is_empty());
    assert!(store.ensure_draft_unsent(sent.clone()).await.is_err());
    assert!(store.add_draft_files(sent, vec![good]).await.is_err());
}

#[tokio::test]
async fn successful_send_cleanup_preserves_a_newer_draft_revision() {
    let store = Store::memory().unwrap();
    let sent = draft();
    let mut latest = sent.clone();
    latest.revision += 1;
    latest.body = "More to say".into();
    store.save_draft(latest.clone()).await.unwrap();
    let state = store.finish_draft_send(sent.clone()).await.unwrap();
    assert_eq!(state.drafts, [latest.clone()]);
    store.save_draft(sent.clone()).await.unwrap();
    assert_eq!(store.draft_state().await.unwrap().drafts, [latest.clone()]);
    assert!(store.ensure_draft_unsent(sent).await.is_err());
    assert!(store.ensure_draft_unsent(latest).await.is_ok());
}

#[tokio::test]
async fn legacy_drafts_load_and_oversized_files_do_not_enter_storage() {
    let old: Draft = serde_json::from_str(r#"{"id":"old","account_id":"work","to":"friend@example.com","subject":"Old","body":"Kept"}"#).unwrap();
    assert_eq!(old.revision, 0);
    assert!(old.cc.is_empty() && old.attachments.is_empty());
    let store = Store::memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.bin");
    std::fs::File::create(&path)
        .unwrap()
        .set_len((compose::MAX_ATTACHMENT_BYTES + 1) as u64)
        .unwrap();
    assert!(store.add_draft_files(old, vec![path]).await.is_err());
    assert!(store.draft_state().await.unwrap().drafts.is_empty());
}
