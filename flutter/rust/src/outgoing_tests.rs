use crate::{
    api::MobileProfile,
    outgoing,
    tests::{account, draft, profile, request, seed},
};
use lettre::address::Envelope;
use rusqlite::params;
use secrecy::SecretString;
use serde_json::{Value, json};
use shep_mail_core::{model::Account, providers::mail::DeliveryFailure};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;

pub(crate) struct Script {
    calls: AtomicUsize,
    entered: Notify,
    pub(crate) release: Notify,
    result: Mutex<Option<Result<(), DeliveryFailure>>>,
    raw: Mutex<Vec<u8>>,
    panic: bool,
}
struct ScriptSmtp(Arc<Script>);
impl outgoing::Smtp for ScriptSmtp {
    fn send(
        &self,
        _account: Account,
        _password: SecretString,
        envelope: Envelope,
        raw: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<(), DeliveryFailure>> + Send>> {
        let s = self.0.clone();
        Box::pin(async move {
            s.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                envelope.to().len(),
                2,
                "Bcc belongs in the immutable envelope"
            );
            assert!(!String::from_utf8_lossy(&raw).contains("Bcc:"));
            *s.raw.lock().unwrap() = raw;
            s.entered.notify_one();
            s.release.notified().await;
            assert!(!s.panic, "Synthetic SMTP panic");
            s.result.lock().unwrap().take().unwrap()
        })
    }
}
pub(crate) fn smtp(
    p: &MobileProfile,
    result: Result<(), DeliveryFailure>,
    panic: bool,
) -> Arc<Script> {
    let s = Arc::new(Script {
        calls: AtomicUsize::new(0),
        entered: Notify::new(),
        release: Notify::new(),
        result: Mutex::new(Some(result)),
        raw: Mutex::new(vec![]),
        panic,
    });
    p.operations
        .outgoing
        .set_smtp(Arc::new(ScriptSmtp(s.clone())));
    s
}
pub(crate) async fn send(p: &MobileProfile) -> Value {
    request(
        p,
        json!({"op":"send","id":"draft-one","revision":1,"password":"synthetic-only","incoming_password":"synthetic-incoming"}),
    )
    .await
}
fn start_send(p: &MobileProfile) -> tokio::task::JoinHandle<Value> {
    let p = MobileProfile {
        database: p.database.clone(),
        operations: p.operations.clone(),
    };
    tokio::spawn(async move { send(&p).await })
}
async fn failed(p: &MobileProfile, payload: Value) -> String {
    let result: Value =
        serde_json::from_str(&p.request(payload.to_string()).await.unwrap()).unwrap();
    result["error"]
        .as_str()
        .expect("Expected a rejected operation")
        .to_owned()
}
pub(crate) async fn setup(p: &MobileProfile) {
    request(p, json!({"op":"save_account","account":account()})).await;
    request(
        p,
        json!({"op":"save_draft","draft":draft(1,"Immutable outgoing body")}),
    )
    .await;
}
async fn enter(s: &Script) {
    tokio::time::timeout(Duration::from_secs(5), s.entered.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn shared_profile_handles_and_other_process_cannot_release_active_smtp() {
    let (dir, p) = profile().await;
    setup(&p).await;
    let s = smtp(&p, Ok(()), false);
    let sending = start_send(&p);
    enter(&s).await;
    let path = dir.path().join("mail.sqlite3");
    let second = MobileProfile::open(path.to_str().unwrap().into())
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&p.database, &second.database));
    assert!(Arc::ptr_eq(&p.operations, &second.operations));
    let record = request(&second, json!({"op":"outbox"})).await;
    let id = record["rows"][0]["id"].as_str().unwrap();
    assert_eq!(record["rows"][0]["state"], "submitting");
    for action in ["return", "mark", "local", "check"] {
        assert!(
            failed(
                &second,
                json!({"op":"recover_outgoing","id":id,"action":action,"confirmed":true})
            )
            .await
            .contains("operation in progress")
        );
    }
    let expected = s.raw.lock().unwrap().clone();
    let saved = p
        .database
        .read(|db| Ok(db.query_row("SELECT raw FROM outgoing", [], |r| r.get::<_, Vec<u8>>(0))?))
        .await
        .unwrap();
    assert_eq!(saved, expected);
    assert_other_process_refused(path).await;
    assert_eq!(send(&second).await["state"], "submitting");
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    s.release.notify_one();
    assert_eq!(sending.await.unwrap()["state"], "delivered");
    let sent = request(&second, json!({"op":"page","folder":"Sent"})).await;
    assert_eq!(sent["total"], 1);
    assert_eq!(request(&second, json!({"op":"drafts"})).await, json!([]));
    assert_eq!(send(&second).await["state"], "delivered");
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
}
async fn assert_other_process_refused(path: std::path::PathBuf) {
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "outgoing_tests::profile_lock_subprocess_child",
                "--nocapture",
            ])
            .env("SHEP_TEST_LOCKED_PROFILE", path)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("PROFILE_LOCK_REFUSED"));
}
#[tokio::test]
async fn profile_lock_subprocess_child() {
    let Ok(path) = std::env::var("SHEP_TEST_LOCKED_PROFILE") else {
        return;
    };
    let error = MobileProfile::open(path)
        .await
        .err()
        .expect("Another process must retain ownership");
    assert!(error.to_string().contains("another Shep process"));
    println!("PROFILE_LOCK_REFUSED");
}
#[tokio::test]
async fn abandoned_submitting_is_uncertain_only_after_exclusive_profile_handover() {
    let (dir, p) = profile().await;
    setup(&p).await;
    p.database.write(|db|{db.execute("INSERT INTO outgoing VALUES('abandoned','draft-one','submitting','fixture','<old@example.test>',?1,?2)",params![b"From: alex@example.test\r\nSubject: Original\r\n\r\nOriginal".to_vec(),draft(1,"Original").to_string()])?;Ok(())}).await.unwrap();
    let path = dir.path().join("mail.sqlite3");
    let same = MobileProfile::open(path.to_str().unwrap().into())
        .await
        .unwrap();
    assert_eq!(
        request(&same, json!({"op":"delivery","id":"draft-one"})).await["state"],
        "submitting"
    );
    drop(same);
    drop(p);
    let reopened = MobileProfile::open(path.to_str().unwrap().into())
        .await
        .unwrap();
    assert_eq!(
        request(&reopened, json!({"op":"delivery","id":"draft-one"})).await["state"],
        "uncertain"
    );
    assert!(
        failed(
            &reopened,
            json!({"op":"recover_outgoing","id":"abandoned","action":"return"})
        )
        .await
        .contains("duplicate")
    );
    let recovered = request(
        &reopened,
        json!({"op":"recover_outgoing","id":"abandoned","action":"return","confirmed":true}),
    )
    .await;
    assert_eq!(recovered["recovery"], "returned");
    assert_eq!(
        request(&reopened, json!({"op":"drafts"})).await[0]["body"],
        "Original"
    );
}
#[tokio::test]
async fn accepted_smtp_survives_sent_cache_failure_and_preserves_newer_local_edits() {
    let (_dir, p) = profile().await;
    setup(&p).await;
    p.database.write(|db|{db.execute_batch("CREATE TRIGGER fail_sent BEFORE INSERT ON mail BEGIN SELECT RAISE(FAIL,'Synthetic disk full'); END;")?;Ok(())}).await.unwrap();
    let s = smtp(&p, Ok(()), false);
    s.release.notify_one();
    let sent = send(&p).await;
    assert_eq!(sent["state"], "delivered");
    assert!(sent["warning"].as_str().unwrap().contains("SMTP accepted"));
    let id = sent["id"].as_str().unwrap();
    assert_eq!(
        request(&p, json!({"op":"outbox"})).await["rows"][0]["state"],
        "delivered"
    );
    assert!(
        failed(
            &p,
            json!({"op":"recover_outgoing","id":id,"action":"return","confirmed":true})
        )
        .await
        .contains("was delivered")
    );
    p.database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_sent")?;
            Ok(())
        })
        .await
        .unwrap();
    request(
        &p,
        json!({"op":"recover_outgoing","id":id,"action":"check"}),
    )
    .await;
    let page = request(&p, json!({"op":"page","folder":"Sent"})).await;
    let local = page["mail"][0]["id"].as_str().unwrap();
    request(
        &p,
        json!({"op":"mutate","id":local,"folder":"Archive","starred":true}),
    )
    .await;
    request(
        &p,
        json!({"op":"recover_outgoing","id":id,"action":"local"}),
    )
    .await;
    let archive = request(&p, json!({"op":"page","folder":"Archive"})).await;
    assert_eq!(archive["mail"][0]["id"], local);
    assert_eq!(archive["mail"][0]["starred"], true);
    let raw = p
        .database
        .read(|db| Ok(db.query_row("SELECT raw FROM mail", [], |r| r.get::<_, Vec<u8>>(0))?))
        .await
        .unwrap();
    assert_eq!(raw, *s.raw.lock().unwrap());
    assert_eq!(request(&p, json!({"op":"outbox"})).await["total"], 0);
    assert_eq!(send(&p).await["state"], "reviewed");
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn pending_terminal_acknowledgement_is_retried_without_resending() {
    let (_dir, p) = profile().await;
    setup(&p).await;
    p.database.write(|db|{db.execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE ON outgoing BEGIN SELECT RAISE(FAIL,'Synthetic receipt failure'); END;")?;Ok(())}).await.unwrap();
    let s = smtp(&p, Ok(()), false);
    s.release.notify_one();
    let sent = send(&p).await;
    let id = sent["id"].as_str().unwrap();
    assert_eq!(sent["state"], "delivered");
    assert!(
        sent["warning"]
            .as_str()
            .unwrap()
            .contains("delivery record")
    );
    let same = MobileProfile {
        database: p.database.clone(),
        operations: p.operations.clone(),
    };
    assert_eq!(
        request(&same, json!({"op":"outbox"})).await["rows"][0]["state"],
        "delivered"
    );
    assert!(
        failed(
            &same,
            json!({"op":"recover_outgoing","id":id,"action":"return","confirmed":true})
        )
        .await
        .contains("Synthetic receipt failure")
    );
    p.database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_receipt")?;
            Ok(())
        })
        .await
        .unwrap();
    request(
        &same,
        json!({"op":"recover_outgoing","id":id,"action":"check"}),
    )
    .await;
    assert_eq!(
        request(&same, json!({"op":"page","folder":"Sent"})).await["total"],
        1
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn rejected_and_reviewed_uncertain_recovery_clones_files_once_and_cannot_revive_original() {
    for uncertain in [false, true] {
        let (dir, p) = profile().await;
        setup(&p).await;
        let file = dir.path().join("original.bin");
        std::fs::write(&file, [0, 255, 1]).unwrap();
        let files=request(&p,json!({"op":"add_draft_files","id":"draft-one","paths":[{"path":file,"name":"original.bin"}]})).await;
        let s = smtp(
            &p,
            if uncertain {
                Err(DeliveryFailure::Uncertain)
            } else {
                Err(DeliveryFailure::Rejected("Synthetic rejection".into()))
            },
            false,
        );
        s.release.notify_one();
        let sent=request(&p,json!({"op":"send","id":"draft-one","revision":1,"file_revision":files["file_revision"],"password":"synthetic"})).await;
        let id = sent["id"].as_str().unwrap();
        if uncertain {
            assert!(
                failed(
                    &p,
                    json!({"op":"recover_outgoing","id":id,"action":"return"})
                )
                .await
                .contains("duplicate")
            );
        }
        // A failed transaction must retain the original draft and file owner.
        p.database.write(|db|{db.execute_batch("CREATE TRIGGER fail_recovery BEFORE INSERT ON outgoing_meta BEGIN SELECT RAISE(FAIL,'Synthetic recovery failure'); END;")?;Ok(())}).await.unwrap();
        assert!(
            failed(
                &p,
                json!({"op":"recover_outgoing","id":id,"action":"return","confirmed":true})
            )
            .await
            .contains("Synthetic recovery failure")
        );
        assert_eq!(
            request(&p, json!({"op":"drafts"})).await[0]["id"],
            "draft-one"
        );
        p.database
            .write(|db| {
                db.execute_batch("DROP TRIGGER fail_recovery")?;
                Ok(())
            })
            .await
            .unwrap();
        let recovered = request(
            &p,
            json!({"op":"recover_outgoing","id":id,"action":"return","confirmed":true}),
        )
        .await;
        let drafts = request(&p, json!({"op":"drafts"})).await;
        assert_eq!(drafts.as_array().unwrap().len(), 1);
        assert_eq!(drafts[0]["id"], recovered["draft_id"]);
        assert_eq!(drafts[0]["body"], "Immutable outgoing body");
        assert_ne!(
            drafts[0]["attachments"][0]["id"],
            files["attachments"][0]["id"]
        );
        let saved = p
            .database
            .read(|db| {
                Ok(db.query_row("SELECT bytes FROM draft_files", [], |r| {
                    r.get::<_, Vec<u8>>(0)
                })?)
            })
            .await
            .unwrap();
        assert_eq!(saved, [0, 255, 1]);
        assert!(
            failed(
                &p,
                json!({"op":"save_draft","draft":draft(99,"old editor")})
            )
            .await
            .contains("submitted")
        );
        assert_eq!(
            request(
                &p,
                json!({"op":"recover_outgoing","id":id,"action":"return","confirmed":true})
            )
            .await,
            recovered
        );
        assert_eq!(send(&p).await["state"], "reviewed");
        assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn cancelled_caller_and_provider_panic_keep_owned_submission_recoverable() {
    for panic in [false, true] {
        let (_dir, p) = profile().await;
        setup(&p).await;
        let s = smtp(&p, Ok(()), panic);
        let caller = start_send(&p);
        enter(&s).await;
        caller.abort();
        assert!(caller.await.is_err());
        s.release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if p.operations.try_account("fixture").await.is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let state = if panic { "uncertain" } else { "delivered" };
        let status = request(&p, json!({"op":"delivery","id":"draft-one"})).await;
        assert_eq!(status["state"], state);
        if panic {
            let id = status["id"].as_str().unwrap();
            assert!(
                failed(&p, json!({"op":"recover_outgoing","id":id,"action":"mark"}))
                    .await
                    .contains("Confirm")
            );
            request(
                &p,
                json!({"op":"recover_outgoing","id":id,"action":"mark","confirmed":true}),
            )
            .await;
            let stored = p
                .database
                .read(|db| {
                    Ok(db.query_row("SELECT state FROM outgoing", [], |r| r.get::<_, String>(0))?)
                })
                .await
                .unwrap();
            assert_eq!(stored, "uncertain");
        }
        assert_eq!(
            request(&p, json!({"op":"page","folder":"Sent"})).await["total"],
            1
        );
        assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn outbox_is_paged_without_raw_mime_or_draft_bodies() {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    p.database
        .write(|db| {
            let tx = db.transaction()?;
            for n in 0..45 {
                let mut d = draft(1, "Private immutable draft body");
                d["id"] = json!(format!("d{n}"));
                tx.execute(
                    "INSERT INTO outgoing VALUES(?1,?2,'uncertain','fixture',?3,?4,?5)",
                    params![
                        format!("attempt{n:02}"),
                        format!("d{n}"),
                        format!("<m{n}@example.test>"),
                        vec![0u8; 1024],
                        d.to_string()
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    for (offset, expected, count) in [(0, 0, 20), (20, 20, 20), (100, 40, 5)] {
        let result = request(&p, json!({"op":"outbox","offset":offset})).await;
        assert_eq!(result["total"], 45);
        assert_eq!(result["offset"], expected);
        assert_eq!(result["rows"].as_array().unwrap().len(), count);
        assert!(!result.to_string().contains("Private immutable draft body"));
        assert!(result["rows"][0].get("raw").is_none());
    }
}
