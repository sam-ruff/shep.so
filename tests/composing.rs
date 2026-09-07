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

#[tokio::test]
async fn discard_retires_all_revisions_and_files_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drafts.sqlite");
    let file = dir.path().join("attachment.txt");
    std::fs::write(&file, "Private draft attachment").unwrap();
    let store = Store::open(&path).unwrap();
    let original = draft();
    let mut other = original.clone();
    other.id = "other-draft".into();
    store.save_draft(other.clone()).await.unwrap();
    let state = store
        .add_draft_files(original.clone(), vec![file.clone()])
        .await
        .unwrap();
    let attached = state
        .drafts
        .iter()
        .find(|d| d.id == original.id)
        .unwrap()
        .clone();
    let deleted = store.delete_draft(original.id.clone()).await.unwrap();
    assert_eq!(deleted.drafts, [other.clone()]);
    assert!(deleted.revision > state.revision);
    assert!(store.draft_files(attached).await.is_err());
    drop(store);
    let store = Store::open(path).unwrap();
    for revision in [
        0,
        original.revision,
        original.revision + 10,
        i64::MAX as u64,
    ] {
        let mut late = original.clone();
        late.revision = revision;
        store.save_draft(late.clone()).await.unwrap();
        assert!(store.ensure_draft_unsent(late.clone()).await.is_err());
        assert!(
            store
                .add_draft_files(late, vec![file.clone()])
                .await
                .is_err()
        );
    }
    // Stale send cleanup cannot weaken the permanent tombstone.
    store.finish_draft_send(original.clone()).await.unwrap();
    store.save_draft(original.clone()).await.unwrap();
    assert_eq!(store.draft_state().await.unwrap().drafts, [other]);
    let files: i64 = store
        .run(|c| Ok(c.query_row("SELECT COUNT(*) FROM draft_attachments", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(files, 0);
    store.delete_draft(original.id).await.unwrap();
}

#[tokio::test]
async fn failed_discard_rolls_back_text_files_and_retirement_then_can_retry() {
    let store = Store::memory().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "Still attached").unwrap();
    let saved = store.add_draft_files(draft(), vec![file]).await.unwrap();
    let draft = saved.drafts[0].clone();
    store.run(|c| { c.execute_batch("CREATE TRIGGER fail_discard BEFORE DELETE ON draft_attachments BEGIN SELECT RAISE(ABORT,'fixture disk error'); END;")?; Ok(()) }).await.unwrap();
    assert!(
        store
            .delete_draft(draft.id.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("fixture disk error")
    );
    let current = store.draft_state().await.unwrap();
    assert_eq!(current.revision, saved.revision);
    assert_eq!(current.drafts, saved.drafts);
    assert_eq!(
        store.draft_files(draft.clone()).await.unwrap()[0].bytes,
        b"Still attached"
    );
    store.ensure_draft_unsent(draft.clone()).await.unwrap();
    store
        .run(|c| {
            c.execute_batch("DROP TRIGGER fail_discard")?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        store
            .delete_draft(draft.id)
            .await
            .unwrap()
            .drafts
            .is_empty()
    );
}

const FORWARD_MIME: &str = concat!(
    "From: Sender <sender@example.test>\r\nTo: alex@example.com, team@example.test\r\n",
    "Cc: copied@example.test\r\nBcc: private@example.test\r\nSubject: Café project\r\n",
    "Date: Sun, 6 Sep 2026 10:00:00 +0000\r\nMessage-ID: <original@example.test>\r\n",
    "References: <thread@example.test>\r\nMIME-Version: 1.0\r\n",
    "Content-Type: multipart/mixed; boundary=outer\r\n\r\n",
    "--outer\r\nContent-Type: multipart/alternative; boundary=alternatives\r\n\r\n",
    "--alternatives\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nComplete original café.\r\n",
    "--alternatives\r\nContent-Type: multipart/related; boundary=related\r\n\r\n",
    "--related\r\nContent-Type: text/html; charset=utf-8\r\n\r\n",
    "<html><head><style>td{color:purple}</style></head><body bgcolor=white><table><tr><td>Complete original café.</td></tr></table><img src=cid:diagram><script>alert('sample')</script></body></html>\r\n",
    "--related\r\nContent-Type: image/png\r\nContent-ID: <diagram>\r\n",
    "Content-Transfer-Encoding: base64\r\n\r\naW5saW5lIGZpeHR1cmU=\r\n--related--\r\n--alternatives--\r\n",
    "--outer\r\nContent-Type: application/x-example\r\nContent-Disposition: attachment; filename=notes.bin\r\n",
    "Content-Transfer-Encoding: base64\r\n\r\nAP8BDQo=\r\n--outer--\r\n"
);

#[tokio::test]
async fn forwarding_retains_html_inline_images_and_files_without_inheriting_recipients_or_thread() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.db");
    let store = Store::open(&path).unwrap();
    let original = parse_mail(
        "work",
        "forward-original",
        "INBOX",
        FORWARD_MIME.as_bytes().to_vec(),
        true,
        true,
    )
    .unwrap();
    let source = original.summary.id.clone();
    store.upsert(vec![original]).await.unwrap();
    let state = store
        .forward_draft(source.clone(), "new-forward".into())
        .await
        .unwrap();
    let mut forward = state.drafts[0].clone();
    assert_eq!(forward.account_id, "work");
    assert_eq!(forward.subject, "Fwd: Café project");
    assert!(forward.to.is_empty() && forward.cc.is_empty() && forward.bcc.is_empty());
    assert!(forward.in_reply_to.is_none() && forward.references.is_empty());
    assert!(
        forward
            .body
            .contains("To: alex@example.com, team@example.test")
    );
    assert!(!forward.body.contains("private@example.test"));
    assert_eq!(forward.attachments.len(), 2);
    assert!(
        compose::build(
            &account(),
            &forward,
            store.draft_files(forward.clone()).await.unwrap()
        )
        .is_err()
    );
    forward.to = "new-recipient@example.test".into();
    forward
        .body
        .insert_str(0, "Please review <these> & reply.\n");
    forward.revision += 1;
    store.save_draft(forward.clone()).await.unwrap();
    // Reopening must use persisted MIME/attachments, not the original body/cache.
    drop(store);
    let store = Store::open(&path).unwrap();
    forward = store.draft_state().await.unwrap().drafts.remove(0);
    let files = store.draft_files(forward.clone()).await.unwrap();
    assert_eq!(files[0].bytes, [0, 255, 1, 13, 10]);
    assert_eq!(files[0].attachment.media_type, "application/x-example");
    assert_eq!(files[1].bytes, b"inline fixture");
    let forwarded_cid = files[1]
        .attachment
        .content_id
        .clone()
        .expect("The image stays inline");
    assert_eq!(files[1].attachment.media_type, "image/png");
    let wire = compose::build(&account(), &forward, files)
        .unwrap()
        .formatted();
    let parsed = mailparse::parse_mail(&wire).unwrap();
    assert!(parsed.headers.get_first_value("In-Reply-To").is_none());
    assert!(parsed.headers.get_first_value("References").is_none());
    assert_ne!(
        parsed.headers.get_first_value("Message-ID").as_deref(),
        Some("<original@example.test>")
    );
    let content = shep::email_content::extract(&parsed).unwrap();
    let html = content.html.unwrap();
    assert!(html.source.contains("<table>"));
    assert!(
        html.source
            .contains("Please review &lt;these&gt; &amp; reply.")
    );
    assert!(html.source.contains("td{color:purple}"));
    assert!(!html.source.contains("<script"));
    assert_eq!(html.inline[&forwarded_cid].as_ref(), b"inline fixture");
    assert_eq!(content.attachments[0].bytes, [0, 255, 1, 13, 10]);
    assert!(store.detail(source.clone()).await.unwrap().summary.unread);
    assert_eq!(
        store.raw_message(source).await.unwrap(),
        FORWARD_MIME.as_bytes()
    );
    store.delete_draft(forward.id.clone()).await.unwrap();
    assert!(store.draft_state().await.unwrap().drafts.is_empty());
    assert!(store.draft_files(forward).await.is_err());
}

#[test]
fn editing_forwarded_text_uses_the_edited_plain_body_and_retains_image_bytes() {
    let (mut draft, files) =
        compose::prepare_forward("draft".into(), "work".into(), FORWARD_MIME.as_bytes()).unwrap();
    draft.to = "friend@example.com".into();
    draft.body = draft
        .body
        .replace("Complete original café.", "Edited café quote.");
    let wire = compose::build(&account(), &draft, files)
        .unwrap()
        .formatted();
    let parsed = mailparse::parse_mail(&wire).unwrap();
    let content = shep::email_content::extract(&parsed).unwrap();
    assert!(content.html.is_none());
    assert!(content.text.contains("Edited café quote."));
    assert!(!content.text.contains("Complete original café."));
    assert_eq!(content.attachments.len(), 2);
    assert_eq!(content.attachments[1].bytes, b"inline fixture");
}

#[tokio::test]
async fn forward_uses_complete_plain_source_and_missing_source_never_saves_partial_draft() {
    let store = Store::memory().unwrap();
    let body = format!(
        "{}\nEND OF COMPLETE ORIGINAL",
        "Long message. ".repeat(4000)
    );
    let raw = format!("From: friend@example.test\r\nSubject: FW: Already forwarded\r\n\r\n{body}")
        .into_bytes();
    let original = parse_mail("work", "long", "INBOX", raw, false, false).unwrap();
    let id = original.summary.id.clone();
    store.upsert(vec![original]).await.unwrap();
    assert!(store.detail(id.clone()).await.unwrap().body_truncated);
    let state = store
        .forward_draft(id.clone(), "full".into())
        .await
        .unwrap();
    let forward = &state.drafts[0];
    assert!(forward.body.ends_with("END OF COMPLETE ORIGINAL"));
    assert_eq!(forward.subject, "FW: Already forwarded");
    assert!(
        forward
            .forward
            .as_ref()
            .unwrap()
            .render(&forward.body)
            .is_none()
    );
    assert!(
        store
            .forward_draft("missing".into(), "partial".into())
            .await
            .is_err()
    );
    assert!(store.forward_draft(id, "full".into()).await.is_err());
    assert_eq!(store.draft_state().await.unwrap().drafts.len(), 1);
}

#[tokio::test]
async fn failed_forward_file_insert_rolls_back_the_whole_draft_and_retry_is_clean() {
    let store = Store::memory().unwrap();
    let mail = parse_mail(
        "work",
        "original",
        "INBOX",
        FORWARD_MIME.as_bytes().to_vec(),
        false,
        false,
    )
    .unwrap();
    let source = mail.summary.id.clone();
    store.upsert(vec![mail]).await.unwrap();
    store.run(|c| { c.execute_batch("CREATE TRIGGER fail_forward BEFORE INSERT ON draft_inline BEGIN SELECT RAISE(ABORT,'fixture disk error'); END;")?; Ok(()) }).await.unwrap();
    assert!(
        store
            .forward_draft(source.clone(), "retry".into())
            .await
            .is_err()
    );
    assert!(store.draft_state().await.unwrap().drafts.is_empty());
    let files: i64 = store
        .run(|c| Ok(c.query_row("SELECT COUNT(*) FROM draft_attachments", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(files, 0);
    store
        .run(|c| {
            c.execute_batch("DROP TRIGGER fail_forward")?;
            Ok(())
        })
        .await
        .unwrap();
    let state = store.forward_draft(source, "retry".into()).await.unwrap();
    assert_eq!(state.drafts[0].attachments.len(), 2);
}

#[tokio::test]
async fn damaged_forward_resources_refuse_without_saving_a_partial_draft() {
    for disposition in [
        "attachment; filename=broken.png",
        "inline; filename=broken.png",
    ] {
        let store = Store::memory().unwrap();
        let raw = format!(
            "Content-Type: multipart/related; boundary=r\r\n\r\n--r\r\nContent-Type: text/html\r\n\r\n<p>Readable letter</p><img src='cid:broken'>\r\n--r\r\nContent-Type: image/png\r\nContent-ID: <broken>\r\nContent-Disposition: {disposition}\r\nContent-Transfer-Encoding: base64\r\n\r\n%%%bad%%%\r\n--r--\r\n"
        );
        let stored =
            shep::model::parse_mail("work", "broken", "INBOX", raw.into_bytes(), true, false)
                .unwrap();
        let id = stored.summary.id.clone();
        store.upsert(vec![stored]).await.unwrap();
        assert!(
            store
                .detail(id.clone())
                .await
                .unwrap()
                .body
                .contains("Readable letter")
        );
        assert!(store.forward_draft(id, "new-forward".into()).await.is_err());
        assert!(store.draft_state().await.unwrap().drafts.is_empty());
    }
}
