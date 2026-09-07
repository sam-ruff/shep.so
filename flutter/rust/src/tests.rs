use crate::{
    api::MobileProfile,
    operations::{self, Request},
};
use rusqlite::params;
use serde_json::{Value, json};
use shep_mail_core::model::*;
use std::sync::Arc;

pub(crate) async fn profile() -> (tempfile::TempDir, MobileProfile) {
    let directory = tempfile::tempdir().unwrap();
    let profile = MobileProfile::open(
        directory
            .path()
            .join("mail.sqlite3")
            .to_str()
            .unwrap()
            .to_owned(),
    )
    .await
    .unwrap();
    (directory, profile)
}
pub(crate) fn account() -> Account {
    Account {
        id: "fixture".into(),
        name: "Fixture account".into(),
        email: "alex@example.test".into(),
        protocol: Protocol::Pop3,
        host: "mail.example.test".into(),
        port: 995,
        username: "alex".into(),
        smtp_host: "smtp.example.test".into(),
        smtp_port: 465,
        incoming_security: ConnectionSecurity::Tls,
        incoming_auth: IncomingAuth::Password,
        smtp_security: Some(ConnectionSecurity::Tls),
        smtp_auth: SmtpAuth::Automatic,
        smtp_username: String::new(),
        smtp_separate_password: false,
        sent_copy: SentCopyPolicy::LocalOnly,
        sent_folder: String::new(),
    }
}
pub(crate) async fn request(profile: &MobileProfile, payload: Value) -> Value {
    let reply: Value =
        serde_json::from_str(&profile.request(payload.to_string()).await.unwrap()).unwrap();
    assert!(reply.get("error").is_none(), "{reply}");
    reply["data"].clone()
}
pub(crate) async fn seed(profile: &MobileProfile, count: usize) {
    request(profile, json!({"op":"save_account","account":account()})).await;
    profile.database.write(move|db|{
        let tx=db.transaction()?;
        for n in 0..count {
            let raw=format!("From: Alex <alex@example.test>\r\nTo: robin@example.test\r\nSubject: Message {n:03}\r\nContent-Type: text/plain\r\n\r\nBody needle {n:03}").into_bytes();
            let mut mail=parse_mail("fixture",&format!("{n}"),"INBOX",raw,n%2==0,n%3==0)?;
            mail.summary.timestamp=n as i64;
            operations::insert_mail(&tx,mail,false)?;
        }
        tx.commit()?;Ok(())
    }).await.unwrap();
}
fn page(offset: u32) -> Value {
    json!({"op":"page","folder":"Inbox","offset":offset})
}
pub(crate) fn draft(revision: u64, body: &str) -> Value {
    json!({"id":"draft-one","account_id":"fixture","to":"robin@example.test","cc":"","bcc":"hidden@example.test","subject":"Draft","body":body,"revision":revision})
}

#[tokio::test]
async fn paging_search_detail_and_pop_flags_persist_across_reopen() {
    let (dir, p) = profile().await;
    seed(&p, 125).await;
    let first = request(&p, page(0)).await;
    assert_eq!(first["total"], 125);
    assert_eq!(first["mail"].as_array().unwrap().len(), 50);
    assert_eq!(first["mail"][0]["subject"], "Message 124");
    assert!(first["mail"][0].get("body").is_none());
    assert!(first["mail"][0].get("raw").is_none());
    let second = request(&p, page(50)).await;
    assert_eq!(second["mail"][0]["subject"], "Message 074");
    let last = request(&p, page(100)).await;
    assert_eq!(last["mail"].as_array().unwrap().len(), 25);
    let id = first["mail"][0]["id"].as_str().unwrap();
    let detail = request(&p, json!({"op":"detail","id":id})).await;
    assert!(detail["body"].as_str().unwrap().contains("Body needle 124"));
    let found = request(
        &p,
        json!({"op":"page","folder":"Inbox","query":"needle 124"}),
    )
    .await;
    assert_eq!(found["total"], 1);
    let punctuation = request(
        &p,
        json!({"op":"page","folder":"Inbox","query":"\" OR * -"}),
    )
    .await;
    assert_eq!(punctuation["total"], 0);
    request(
        &p,
        json!({"op":"mutate","id":id,"folder":"Archive","unread":false,"starred":true}),
    )
    .await;
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let archived = request(&reopened, json!({"op":"page","folder":"Archive"})).await;
    assert_eq!(archived["total"], 1);
    assert_eq!(archived["mail"][0]["unread"], false);
    assert_eq!(archived["mail"][0]["starred"], true);
    assert_eq!(request(&reopened, page(0)).await["unread"], 62);
}

#[tokio::test]
async fn pop_resync_preserves_local_move_and_flags() {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    let original = request(&p, page(0)).await;
    let id = original["mail"][0]["id"].as_str().unwrap();
    request(
        &p,
        json!({"op":"mutate","id":id,"folder":"Archive","unread":false,"starred":true}),
    )
    .await;
    p.database
        .write(|db| {
            let raw =
                b"From: Alex <alex@example.test>\r\nSubject: Updated\r\n\r\nUpdated body".to_vec();
            operations::insert_mail(
                db,
                parse_mail("fixture", "0", "INBOX", raw, true, false)?,
                true,
            )
        })
        .await
        .unwrap();
    let archive = request(&p, json!({"op":"page","folder":"Archive"})).await;
    assert_eq!(archive["mail"][0]["subject"], "Updated");
    assert_eq!(archive["mail"][0]["starred"], true);
    assert_eq!(archive["mail"][0]["unread"], false);
}

#[tokio::test]
async fn draft_revisions_discard_and_files_are_atomic_and_survive_restart() {
    let (dir, p) = profile().await;
    request(&p, json!({"op":"save_draft","draft":draft(4,"newer")})).await;
    request(&p, json!({"op":"save_draft","draft":draft(2,"old")})).await;
    assert_eq!(
        request(&p, json!({"op":"drafts"})).await[0]["body"],
        "newer"
    );
    p.database
        .write(|db| {
            db.execute(
                "INSERT INTO draft_files VALUES('file','draft-one','note.txt','text/plain',?1)",
                params![b"attachment".to_vec()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    request(
        &p,
        json!({"op":"discard_draft","id":"draft-one","revision":5}),
    )
    .await;
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let rejected: Value = serde_json::from_str(
        &reopened
            .request(json!({"op":"save_draft","draft":draft(9,"late save")}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(rejected["error"].as_str().unwrap().contains("discarded"));
    assert_eq!(request(&reopened, json!({"op":"drafts"})).await, json!([]));
    assert_eq!(
        reopened
            .database
            .read(
                |db| Ok(db.query_row("SELECT COUNT(*) FROM draft_files", [], |r| r
                    .get::<_, i64>(0))?)
            )
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn existing_submission_never_calls_smtp_or_allows_draft_revival() {
    let (dir, p) = profile().await;
    request(&p, json!({"op":"save_draft","draft":draft(1,"immutable")})).await;
    p.database.write(|db|{db.execute("INSERT INTO outgoing VALUES('submission','draft-one','submitting','fixture','<stable@example.test>',?1,?2)",params![b"immutable MIME".to_vec(),draft(1,"immutable").to_string()])?;Ok(())}).await.unwrap();
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    // No account exists, so accidentally entering the SMTP path would fail.
    let status = request(
        &reopened,
        json!({"op":"send","id":"draft-one","revision":1,"password":"fixture-only-not-a-real-password"}),
    )
    .await;
    assert_eq!(status["state"], "submitting");
    assert_eq!(status["message_id"], "<stable@example.test>");
    let source = dir.path().join("late.txt");
    std::fs::write(&source, b"Never associate this with an outgoing message").unwrap();
    for payload in [
        json!({"op":"add_draft_files","id":"draft-one","paths":[{"path":source,"name":"late.txt"}]}),
        json!({"op":"remove_draft_file","id":"draft-one","file":"absent"}),
    ] {
        let response: Value =
            serde_json::from_str(&reopened.request(payload.to_string()).await.unwrap()).unwrap();
        assert!(response["error"].as_str().unwrap().contains("submitted"));
    }
    assert_eq!(
        request(&reopened, json!({"op":"draft_files","id":"draft-one"})).await["attachments"],
        json!([])
    );

    let reply: Value = serde_json::from_str(
        &reopened
            .request(json!({"op":"save_draft","draft":draft(5,"changed")}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(reply["error"].is_string());
    let reply: Value = serde_json::from_str(
        &reopened
            .request(json!({"op":"discard_draft","id":"draft-one","revision":5}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(reply["error"].is_string());
}

#[tokio::test]
async fn account_identity_changes_are_rejected_without_erasing_cache() {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    let mut changed = account();
    changed.host = "other.example.test".into();
    assert!(
        operations::run(
            &p,
            Request::SaveAccount {
                account: changed,
                preserve_sent: false
            }
        )
        .await
        .is_err()
    );
    assert_eq!(request(&p, page(0)).await["total"], 1);
    assert_eq!(
        request(&p, json!({"op":"accounts"})).await["accounts"][0]["host"],
        "mail.example.test"
    );
}

#[tokio::test]
async fn independent_cached_reads_continue_while_writer_is_held_and_cancellation_preserves_fifo() {
    let (_dir, p) = profile().await;
    let db = p.database.clone();
    db.write(|db| {
        db.execute(
            "INSERT INTO drafts VALUES('ordered',0,json_object('body','initial'))",
            [],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let held = db.clone();
    let first = tokio::spawn(async move {
        held.write(move |db| {
            let _ = started_tx.send(());
            release_rx.recv().unwrap();
            db.execute(
                "UPDATE drafts SET content=json_object('body','first') WHERE id='ordered'",
                [],
            )?;
            Ok(())
        })
        .await
    });
    started_rx.await.unwrap();
    first.abort();
    let second_db = db.clone();
    let second = tokio::spawn(async move {
        second_db
            .write(|db| {
                db.execute(
                    "UPDATE drafts SET content=json_object('body','last') WHERE id='ordered'",
                    [],
                )?;
                Ok(())
            })
            .await
    });
    // This is a correctness barrier, not a latency measurement or a sleep.
    assert_eq!(
        db.read(|db| Ok(db.query_row(
            "SELECT json_extract(content,'$.body') FROM drafts WHERE id='ordered'",
            [],
            |r| r.get::<_, String>(0)
        )?))
        .await
        .unwrap(),
        "initial"
    );
    assert!(!second.is_finished());
    release_tx.send(()).unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(
        db.read(|db| Ok(db.query_row(
            "SELECT json_extract(content,'$.body') FROM drafts WHERE id='ordered'",
            [],
            |r| r.get::<_, String>(0)
        )?))
        .await
        .unwrap(),
        "last"
    );
}

#[tokio::test]
async fn invalid_paths_and_future_schema_are_refused() {
    assert!(
        MobileProfile::open("relative-mail.sqlite3".into())
            .await
            .is_err()
    );
    let (dir, p) = profile().await;
    p.database
        .write(|db| {
            db.execute_batch("PRAGMA user_version=99")?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancelled_bridge_waiter_still_finishes_its_owned_cache_write() {
    let (_dir, p) = profile().await;
    let p = Arc::new(p);
    let (entered, entered_rx) = tokio::sync::oneshot::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let database = p.database.clone();
    let blocker = tokio::spawn(async move {
        database
            .write(move |_| {
                let _ = entered.send(());
                release_rx.recv().unwrap();
                Ok(())
            })
            .await
    });
    entered_rx.await.unwrap();
    let copy = p.clone();
    let waiter = tokio::spawn(async move {
        copy.request(json!({"op":"save_draft","draft":draft(7,"saved")}).to_string())
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while p.database.pending_writes() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    waiter.abort();
    let _ = waiter.await;
    release.send(()).unwrap();
    blocker.await.unwrap().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if request(&p, json!({"op":"drafts"})).await[0]["revision"] == 7 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn acknowledged_move_survives_sync_reopen_and_preserves_one_local_identity() {
    use shep_mail_core::mail_actions::{Fingerprint, MoveReceipt};
    let (dir, p) = profile().await;
    let mut config = account();
    config.protocol = Protocol::Imap;
    config.port = 993;
    request(&p, json!({"op":"save_account","account":config})).await;
    let raw =
        b"Message-ID: <move@example.test>\r\nSubject: Durable move\r\n\r\nExact original".to_vec();
    let original = parse_mail("fixture", "42.7", "INBOX", raw.clone(), true, false).unwrap();
    let source = original.summary.clone();
    let receipt = MoveReceipt::server(
        &source,
        "fixture",
        "Archive",
        Some("91.8".into()),
        Fingerprint::of(&raw),
    );
    let source_id = source.id.clone();
    p.database
        .write(move |db| {
            let tx = db.transaction()?;
            operations::insert_mail(&tx, original, false)?;
            tx.execute(
                "INSERT INTO pending_moves(id,destination) VALUES(?1,'Archive')",
                [&source_id],
            )?;
            operations::save_move(&tx, &source_id, &receipt)?;
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    let archive = request(&p, json!({"op":"page","folder":"Archive"})).await;
    assert_eq!(archive["total"], 1);
    assert_eq!(archive["mail"][0]["id"], source.id);
    assert_eq!(archive["mail"][0]["remote_id"], "91.8");
    let destination = parse_mail("fixture", "91.8", "Archive", raw.clone(), false, true).unwrap();
    let returned = destination.summary.clone();
    p.database
        .write(move |db| operations::insert_mail(db, destination, false))
        .await
        .unwrap();
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let archive = request(&reopened, json!({"op":"page","folder":"Archive"})).await;
    assert_eq!(archive["total"], 1);
    assert_eq!(archive["mail"][0]["id"], source.id);
    assert_eq!(archive["mail"][0]["starred"], true);
    let identity = source.id.clone();
    reopened
        .database
        .write(move |db| operations::resolve_cached_move(db, &identity, &returned))
        .await
        .unwrap();
    let mut current = source.clone();
    current.folder = "Archive".into();
    current.remote_id = "91.8".into();
    let undo = MoveReceipt::server(
        &current,
        "fixture",
        "INBOX",
        Some("42.18".into()),
        Fingerprint::of(&raw),
    );
    let identity = source.id.clone();
    reopened
        .database
        .write(move |db| {
            let tx = db.transaction()?;
            operations::save_move(&tx, &identity, &undo)?;
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    let inbox = request(&reopened, page(0)).await;
    assert_eq!(inbox["total"], 1);
    assert_eq!(inbox["mail"][0]["id"], source.id);
    assert_eq!(inbox["mail"][0]["remote_id"], "42.18");
    let detail = request(&reopened, json!({"op":"detail","id":source.id})).await;
    assert!(detail["body"].as_str().unwrap().contains("Exact original"));
    reopened
        .database
        .read(|db| {
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM pending_moves", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            assert_eq!(
                db.query_row(
                    "SELECT COUNT(*) FROM mail_search WHERE mail_search MATCH 'Exact'",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                1
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn unacknowledged_and_legacy_moves_cannot_reuse_a_uid_after_restart() {
    let (dir, p) = profile().await;
    seed(&p, 1).await;
    p.database
        .write(|db| {
            db.execute(
                "INSERT INTO pending_moves VALUES('fixture:INBOX:0','Archive')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let response: Value = serde_json::from_str(
        &reopened
            .request(json!({"op":"mutate","id":"fixture:INBOX:0","folder":"INBOX"}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        response["error"]
            .as_str()
            .unwrap()
            .contains("not moved again")
    );
    p.database
        .write(|db| {
            db.execute("DELETE FROM pending_moves", [])?;
            db.execute("UPDATE mail SET moved=1,folder='Archive'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let response: Value = serde_json::from_str(
        &reopened
            .request(json!({"op":"mutate","id":"fixture:INBOX:0","folder":"INBOX"}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(response["error"].as_str().unwrap().contains("older move"));
}

#[tokio::test]
async fn move_recovery_deduplicates_only_matching_bytes_atomically() {
    use shep_mail_core::mail_actions::{Fingerprint, MoveReceipt};
    for conflicting in [false, true] {
        let (_dir, p) = profile().await;
        seed(&p, 1).await;
        p.database
            .write(move |db| {
                let raw: Vec<u8> = db.query_row("SELECT raw FROM mail", [], |r| r.get(0))?;
                let source = parse_mail("fixture", "0", "INBOX", raw.clone(), true, false)?;
                let receipt = MoveReceipt::server(
                    &source.summary,
                    "fixture",
                    "Archive",
                    None,
                    Fingerprint::of(&raw),
                );
                let tx = db.transaction()?;
                operations::save_move(&tx, &source.summary.id, &receipt)?;
                tx.commit()?;
                let copy = parse_mail(
                    "fixture",
                    "91.8",
                    "Archive",
                    if conflicting {
                        b"Subject: Another\r\n\r\nDifferent".to_vec()
                    } else {
                        raw
                    },
                    false,
                    true,
                )?;
                let target = copy.summary.clone();
                operations::insert_mail(db, copy, false)?;
                let result = operations::resolve_cached_move(db, &source.summary.id, &target);
                assert_eq!(result.is_err(), conflicting);
                assert_eq!(
                    db.query_row("SELECT COUNT(*) FROM mail", [], |r| r.get::<_, i64>(0))?,
                    if conflicting { 2 } else { 1 }
                );
                assert_eq!(
                    db.query_row("SELECT COUNT(*) FROM move_receipts", [], |r| r
                        .get::<_, i64>(0))?,
                    if conflicting { 1 } else { 0 }
                );
                Ok(())
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn complete_reconciliation_releases_only_current_source_intents_without_moving_mail() {
    let (_dir, p) = profile().await;
    seed(&p, 2).await;
    p.database
        .write(|db| {
            let tx = db.transaction()?;
            tx.execute(
                "INSERT INTO pending_moves SELECT id,'Archive' FROM mail",
                [],
            )?;
            operations::reconcile_folder(
                &tx,
                "fixture",
                "INBOX",
                &["fixture:INBOX:0".to_owned()].into(),
            )?;
            assert_eq!(
                tx.query_row("SELECT COUNT(*) FROM pending_moves", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            assert_eq!(
                tx.query_row("SELECT id FROM mail", [], |r| r.get::<_, String>(0))?,
                "fixture:INBOX:0"
            );
            assert_eq!(
                tx.query_row("SELECT folder FROM mail", [], |r| r.get::<_, String>(0))?,
                "INBOX"
            );
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn attachments_survive_source_deletion_text_autosave_reopen_and_exact_mime_build() {
    let (dir, p) = profile().await;
    seed(&p, 0).await;
    let mut message = draft(1, "Draft with files");
    message["in_reply_to"] = "<original@example.test>".into();
    message["references"] = json!(["<root@example.test>", "<original@example.test>"]);
    request(&p, json!({"op":"save_draft","draft":message})).await;
    let text = dir.path().join("one.txt");
    let binary = dir.path().join("two.bin");
    std::fs::write(&text, b"Keep this text").unwrap();
    std::fs::write(&binary, [0, 255, 1, 13, 10]).unwrap();
    let added=request(&p,json!({"op":"add_draft_files","id":"draft-one","paths":[{"path":text,"name":"one.txt"},{"path":binary,"name":"two.bin"}]})).await;
    assert_eq!(added["file_revision"], 1);
    assert_eq!(added["attachments"].as_array().unwrap().len(), 2);
    std::fs::remove_file(text).unwrap();
    std::fs::remove_file(binary).unwrap();
    message["revision"] = 2.into();
    message["body"] = "Updated text".into();
    message["attachments"] = json!([]);
    request(&p, json!({"op":"save_draft","draft":message})).await;
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let saved = request(&reopened, json!({"op":"drafts"})).await;
    assert_eq!(saved[0]["attachments"].as_array().unwrap().len(), 2);
    assert_eq!(saved[0]["references"].as_array().unwrap().len(), 2);
    let removed = request(
        &reopened,
        json!({"op":"remove_draft_file","id":"draft-one","file":added["attachments"][0]["id"]}),
    )
    .await;
    assert_eq!(removed["file_revision"], 2);
    assert_eq!(removed["attachments"][0]["name"], "two.bin");
    // A later text save carrying the old list cannot resurrect the removed blob.
    message["revision"] = 3.into();
    message["attachments"] = added["attachments"].clone();
    request(&reopened, json!({"op":"save_draft","draft":message})).await;
    let mismatch:Value=serde_json::from_str(&reopened.request(json!({"op":"send","id":"draft-one","revision":3,"file_revision":1,"password":"fixture"}).to_string()).await.unwrap()).unwrap();
    assert!(
        mismatch["error"]
            .as_str()
            .unwrap()
            .contains("attachments changed")
    );
    reopened
        .database
        .read(|db| {
            let mut draft = crate::drafts::editable(db, "draft-one")?;
            let parts = crate::drafts::files(db, &mut draft)?;
            let wire = shep_mail_core::compose::build_with_message_id(
                &account(),
                &draft,
                parts,
                "<send@example.test>",
            )?
            .formatted();
            let parsed = mailparse::parse_mail(&wire)?;
            use mailparse::MailHeaderMap;
            assert_eq!(
                parsed.headers.get_first_value("In-Reply-To").as_deref(),
                Some("<original@example.test>")
            );
            let (body, files) = content(&parsed)?;
            assert!(body.contains("Updated text"));
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].bytes, [0, 255, 1, 13, 10]);
            assert_eq!(files[0].name, "two.bin");
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM outgoing", [], |r| r.get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn attachment_limits_fail_atomically_and_discarded_drafts_reject_imports() {
    let (dir, p) = profile().await;
    request(&p, json!({"op":"save_draft","draft":draft(1,"Files")})).await;
    let small = dir.path().join("small");
    std::fs::write(&small, b"Small").unwrap();
    let large = dir.path().join("large");
    std::fs::File::create(&large)
        .unwrap()
        .set_len((shep_mail_core::compose::MAX_ATTACHMENT_BYTES + 1) as u64)
        .unwrap();
    let rejected:Value=serde_json::from_str(&p.request(json!({"op":"add_draft_files","id":"draft-one","paths":[{"path":small,"name":"small"},{"path":large,"name":"large"}]}).to_string()).await.unwrap()).unwrap();
    assert!(rejected["error"].is_string());
    assert_eq!(
        request(&p, json!({"op":"draft_files","id":"draft-one"})).await["attachments"],
        json!([])
    );
    request(
        &p,
        json!({"op":"discard_draft","id":"draft-one","revision":2}),
    )
    .await;
    let rejected:Value=serde_json::from_str(&p.request(json!({"op":"add_draft_files","id":"draft-one","paths":[{"path":small,"name":"small"}]}).to_string()).await.unwrap()).unwrap();
    assert!(rejected["error"].as_str().unwrap().contains("discarded"));
    p.database
        .read(|db| {
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM draft_files", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn cached_reply_all_uses_reply_to_and_excludes_every_configured_sender() {
    let (_dir, p) = profile().await;
    seed(&p, 0).await;
    let mut second = account();
    second.id = "other".into();
    second.email = "other@example.test".into();
    request(&p, json!({"op":"save_account","account":second})).await;
    let id=p.database.write(|db|{
        let raw=b"From: Sender <sender@example.test>\r\nReply-To: Support <support@example.test>\r\nTo: Alex <alex@example.test>, Other <other@example.test>, Peer <peer@example.test>\r\nCc: Peer <peer@example.test>, Copy <copy@example.test>\r\nMessage-ID: <original@example.test>\r\nReferences: <root@example.test>\r\nSubject: Re: Cached reply\r\nDate: Sun, 6 Sep 2026 10:00:00 +0000\r\n\r\nHello\r\nSecond line\r\n".to_vec();
        let mail=parse_mail("fixture","42.7","INBOX",raw,true,false)?;let id=mail.summary.id.clone();operations::insert_mail(db,mail,false)?;Ok(id)
    }).await.unwrap();
    let reply = request(&p, json!({"op":"reply","id":id,"all":true})).await;
    assert_eq!(
        reply["to"],
        "Support <support@example.test>, Peer <peer@example.test>"
    );
    assert_eq!(reply["cc"], "Copy <copy@example.test>");
    assert_eq!(reply["bcc"], "");
    assert_eq!(reply["subject"], "Re: Cached reply");
    assert_eq!(reply["in_reply_to"], "<original@example.test>");
    assert_eq!(
        reply["references"],
        json!(["<root@example.test>", "<original@example.test>"])
    );
    assert!(
        reply["body"]
            .as_str()
            .unwrap()
            .contains("> Hello\n> Second line")
    );
    request(&p, json!({"op":"save_draft","draft":reply})).await;
    assert_eq!(
        request(&p, json!({"op":"drafts"})).await[0]["in_reply_to"],
        "<original@example.test>"
    );
}

#[tokio::test]
async fn cached_attachment_bytes_reopen_alias_and_stale_identity_without_provider_capacity() {
    let (dir, p) = profile().await;
    seed(&p, 0).await;
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../shared/attachment-fixtures.json")).unwrap();
    let raw = fixtures[0]["raw"].as_str().unwrap().as_bytes().to_vec();
    p.database.write(move|db| {
        operations::insert_mail(db,parse_mail("fixture","files","INBOX",raw,false,false)?,false)?;
        db.execute("INSERT INTO mail_aliases(alias,id) VALUES('previous-file-id','fixture:INBOX:files')",[])?;
        Ok(())
    }).await.unwrap();
    let _occupied = p.operations.hold_network_capacity().await;
    let detail = request(&p, json!({"op":"detail","id":"previous-file-id"})).await;
    for file in fixtures[0]["files"].as_array().unwrap() {
        let bytes = request(
            &p,
            json!({"op":"attachment","id":"previous-file-id","file":file["id"]}),
        )
        .await;
        assert_eq!(bytes["bytes"], file["bytes"]);
        assert_eq!(bytes["info"]["name"], file["name"]);
    }
    assert_eq!(detail["files"].as_array().unwrap().len(), 3);
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let first = &fixtures[0]["files"][0];
    assert_eq!(
        request(
            &reopened,
            json!({"op":"attachment","id":"previous-file-id","file":first["id"]})
        )
        .await["bytes"],
        first["bytes"]
    );
    p.database.write(|db|{db.execute("UPDATE mail SET raw=CAST(replace(CAST(raw AS TEXT),'AP8BDQo=','AAECAwQ=') AS BLOB) WHERE remote_id='files'",[])?;Ok(())}).await.unwrap();
    let failed: Value = serde_json::from_str(
        &p.request(
            json!({"op":"attachment","id":"previous-file-id","file":first["id"]}).to_string(),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert!(failed["error"].as_str().unwrap().contains("changed"));
}

#[tokio::test]
async fn corrupt_attachment_does_not_hide_cached_body_or_allow_an_empty_save() {
    let (_dir, p) = profile().await;
    seed(&p, 0).await;
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../shared/attachment-fixtures.json")).unwrap();
    let raw = fixtures[0]["raw"]
        .as_str()
        .unwrap()
        .replace("AP8BDQo=", "%%%invalid%%%")
        .into_bytes();
    p.database
        .write(move |db| {
            operations::insert_mail(
                db,
                parse_mail("fixture", "corrupt", "INBOX", raw, false, false)?,
                false,
            )
        })
        .await
        .unwrap();
    let detail = request(&p, json!({"op":"detail","id":"fixture:INBOX:corrupt"})).await;
    assert!(
        detail["body"]
            .as_str()
            .unwrap()
            .contains("Cached incoming files.")
    );
    assert!(
        detail["file_error"]
            .as_str()
            .unwrap()
            .contains("cached message body")
    );
    assert!(detail["files"].as_array().unwrap().is_empty());
    let saved:Value=serde_json::from_str(&p.request(json!({"op":"attachment","id":"fixture:INBOX:corrupt","file":fixtures[0]["files"][0]["id"]}).to_string()).await.unwrap()).unwrap();
    assert!(saved["error"].is_string());
    assert!(saved.get("data").is_none());
}

#[tokio::test]
async fn selected_representations_reach_cached_native_detail_without_provider_slots() {
    let (_dir, p) = profile().await;
    seed(&p, 0).await;
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../../shared/reader-fixtures.json")).unwrap();
    for (index, case) in cases.iter().enumerate() {
        let raw = case["raw"].as_str().unwrap().as_bytes().to_vec();
        p.database
            .write(move |db| {
                operations::insert_mail(
                    db,
                    parse_mail("fixture", &index.to_string(), "INBOX", raw, false, false)?,
                    false,
                )
            })
            .await
            .unwrap();
    }
    let _occupied = p.operations.hold_network_capacity().await;
    for (index, case) in cases.iter().enumerate() {
        let detail = request(
            &p,
            json!({"op":"detail","id":format!("fixture:INBOX:{index}")}),
        )
        .await;
        assert_eq!(detail["body"], case["body"]["text"], "{}", case["name"]);
        assert_eq!(
            detail["files"].as_array().unwrap().len(),
            case["files"].as_array().unwrap().len()
        );
    }
}

#[tokio::test]
async fn find_visible_text_works_with_all_provider_capacity_occupied() {
    let (_dir, p) = profile().await;
    let _occupied = p.operations.hold_network_capacity().await;
    let cases: Value =
        serde_json::from_str(include_str!("../../../shared/find-cases.json")).unwrap();
    for case in cases.as_array().unwrap() {
        let result=request(&p,json!({"op":"find_text","blocks":case["blocks"],"query":case["query"],"match_case":case["match_case"]})).await;
        assert_eq!(result, case["hits"]);
    }
}
