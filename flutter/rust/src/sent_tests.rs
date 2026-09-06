use crate::{
    api::MobileProfile,
    sent,
    tests::{account, draft, profile, request},
};
use async_trait::async_trait;
use rusqlite::params;
use secrecy::SecretString;
use serde_json::{Value, json};
use shep_mail_core::{
    model::*,
    providers::mail::sent::{SentConnection, SentReceipt},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;

#[derive(Default)]
struct Server {
    opens: AtomicUsize,
    appends: AtomicUsize,
    found: AtomicBool,
    reject: AtomicBool,
    hold: AtomicBool,
    entered: Notify,
    release: Notify,
    folders: Mutex<Vec<String>>,
    bytes: Mutex<Vec<Vec<u8>>>,
}
struct Fake(Arc<Server>);
struct Connection {
    state: Arc<Server>,
    folder: String,
}
impl sent::Factory for Fake {
    fn open(&self, account: Account, _password: SecretString) -> sent::Open {
        let state = self.0.clone();
        Box::pin(async move {
            state.opens.fetch_add(1, Ordering::SeqCst);
            let folder = if account.sent_folder.is_empty() {
                "Sent Mail".into()
            } else {
                account.sent_folder
            };
            state.folders.lock().unwrap().push(folder.clone());
            Ok(Box::new(Connection { state, folder }) as Box<dyn SentConnection>)
        })
    }
}
#[async_trait]
impl SentConnection for Connection {
    fn folder(&self) -> &str {
        &self.folder
    }
    async fn find(&mut self, id: &str) -> anyhow::Result<Option<SentReceipt>> {
        assert!(id.starts_with('<') && id.ends_with('>'));
        Ok(self
            .state
            .found
            .load(Ordering::SeqCst)
            .then(|| SentReceipt {
                folder: self.folder.clone(),
                remote_id: Some("91.4".into()),
            }))
    }
    async fn append(&mut self, raw: &[u8], _timestamp: i64) -> anyhow::Result<SentReceipt> {
        self.state.appends.fetch_add(1, Ordering::SeqCst);
        self.state.bytes.lock().unwrap().push(raw.to_vec());
        self.state.entered.notify_one();
        if self.state.hold.load(Ordering::SeqCst) {
            self.state.release.notified().await;
        }
        anyhow::ensure!(
            !self.state.reject.load(Ordering::SeqCst),
            "Synthetic lost APPEND acknowledgment"
        );
        Ok(SentReceipt {
            folder: self.folder.clone(),
            remote_id: None,
        })
    }
}
fn transport(p: &MobileProfile) -> Arc<Server> {
    let server = Arc::new(Server::default());
    p.operations
        .sent
        .set_factory(Arc::new(Fake(server.clone())));
    server
}
async fn setup(p: &MobileProfile, delivery: &str) -> Vec<u8> {
    let mut account = account();
    account.protocol = Protocol::Imap;
    account.port = 993;
    account.sent_copy = SentCopyPolicy::Automatic;
    request(p, json!({"op":"save_account","account":account})).await;
    let draft = draft(1, "Exact Sent body");
    request(p, json!({"op":"save_draft","draft":draft})).await;
    let raw=b"Message-ID: <sent-copy@shep.so>\r\nFrom: alex@example.test\r\nTo: robin@example.test\r\nSubject: Exact Sent fixture\r\n\r\nExact Sent body\r\n".to_vec();
    let bytes = raw.clone();
    let delivery = delivery.to_owned();
    p.database.write(move|db|{
        db.execute("INSERT INTO outgoing VALUES('copy','draft-one',?1,'fixture','<sent-copy@shep.so>',?2,?3)",params![delivery,bytes,draft.to_string()])?;
        db.execute("INSERT INTO outgoing_meta(id,created,from_address) VALUES('copy',1788688800,'alex@example.test')",[])?;
        db.execute("INSERT INTO outgoing_sent(id,account,state) VALUES('copy',?1,'pending')",[serde_json::to_string(&account)?])?;
        Ok(())
    }).await.unwrap();
    raw
}
async fn copy(p: &MobileProfile, upload: bool, confirmed: bool) -> Value {
    let result=p.request(json!({"op":"sent_outgoing","id":"copy","copy":upload,"confirmed":confirmed,"password":"synthetic-incoming"}).to_string()).await.unwrap();
    serde_json::from_str(&result).unwrap()
}
async fn sql(p: &MobileProfile, text: &'static str) {
    p.database
        .write(move |db| {
            db.execute_batch(text)?;
            Ok(())
        })
        .await
        .unwrap();
}
async fn wait(ready: &Notify) {
    tokio::time::timeout(std::time::Duration::from_secs(5), ready.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn provider_lookup_resolves_uncertain_delivery_without_append_or_inventing_smtp_ack() {
    let (dir, p) = profile().await;
    let raw = setup(&p, "uncertain").await;
    let server = transport(&p);
    server.found.store(true, Ordering::SeqCst);
    assert_eq!(copy(&p, false, false).await["data"]["sent"], "saved");
    assert_eq!(server.appends.load(Ordering::SeqCst), 0);
    assert_eq!(request(&p, json!({"op":"drafts"})).await, json!([]));
    assert_eq!(request(&p, json!({"op":"outbox"})).await["total"], 0);
    let stored = p
        .database
        .read(|db| {
            Ok((
                db.query_row("SELECT state FROM outgoing", [], |r| r.get::<_, String>(0))?,
                db.query_row("SELECT raw FROM mail", [], |r| r.get::<_, Vec<u8>>(0))?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(stored, ("uncertain".into(), raw));
    drop(p);
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    assert_eq!(request(&reopened, json!({"op":"outbox"})).await["total"], 0);
    assert_eq!(
        request(&reopened, json!({"op":"page","folder":"Sent"})).await["total"],
        1
    );
}

#[tokio::test]
async fn unacknowledged_append_pins_folder_and_requires_review_before_another_copy() {
    let (_dir, p) = profile().await;
    let raw = setup(&p, "delivered").await;
    let server = transport(&p);
    server.reject.store(true, Ordering::SeqCst);
    assert!(
        copy(&p, true, false).await["error"]
            .as_str()
            .unwrap()
            .contains("did not acknowledge")
    );
    assert_eq!(server.appends.load(Ordering::SeqCst), 1);
    assert_eq!(
        request(&p, json!({"op":"outbox"})).await["rows"][0]["sent"],
        "uncertain"
    );
    request(&p,json!({"op":"save_sent_preferences","id":"fixture","policy":"Automatic","folder":"Changed Sent"})).await;
    assert!(
        copy(&p, true, false).await["error"]
            .as_str()
            .unwrap()
            .contains("confirm")
    );
    assert_eq!(server.opens.load(Ordering::SeqCst), 1);
    assert!(
        copy(&p, false, false).await["error"]
            .as_str()
            .unwrap()
            .contains("No matching copy")
    );
    assert_eq!(server.appends.load(Ordering::SeqCst), 1);
    server.reject.store(false, Ordering::SeqCst);
    assert_eq!(copy(&p, true, true).await["data"]["sent"], "saved");
    assert_eq!(server.appends.load(Ordering::SeqCst), 2);
    assert!(
        server
            .folders
            .lock()
            .unwrap()
            .iter()
            .all(|s| s == "Sent Mail")
    );
    assert_eq!(*server.bytes.lock().unwrap(), vec![raw.clone(), raw]);
    assert_eq!(copy(&p, true, true).await["data"]["sent"], "saved");
    assert_eq!(server.appends.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn sent_acknowledgment_and_local_cache_failures_never_repeat_an_append() {
    for stage in ["before", "receipt", "cache"] {
        let (_dir, p) = profile().await;
        setup(&p, "delivered").await;
        let server = transport(&p);
        sql(&p,match stage {
            "before" => "CREATE TRIGGER fail BEFORE UPDATE OF state ON outgoing_sent WHEN NEW.state='appending' BEGIN SELECT RAISE(FAIL,'Fixture disk full'); END;",
            "receipt" => "CREATE TRIGGER fail BEFORE UPDATE OF state ON outgoing_sent WHEN NEW.state='saved' BEGIN SELECT RAISE(FAIL,'Fixture disk full'); END;",
            _ => "CREATE TRIGGER fail BEFORE INSERT ON mail BEGIN SELECT RAISE(FAIL,'Fixture disk full'); END;",
        }).await;
        assert!(copy(&p, true, false).await["error"].is_string());
        assert_eq!(
            server.appends.load(Ordering::SeqCst),
            usize::from(stage != "before")
        );
        if stage == "receipt" {
            assert_eq!(
                request(&p, json!({"op":"outbox"})).await["rows"][0]["sent"],
                "saved"
            );
        }
        sql(&p, "DROP TRIGGER fail;").await;
        if stage == "before" {
            assert_eq!(copy(&p, true, false).await["data"]["sent"], "saved");
        } else {
            // A known acknowledgment repairs storage even with no credentials.
            assert_eq!(
                request(&p, json!({"op":"sent_outgoing","id":"copy","copy":false})).await["sent"],
                "saved"
            );
        }
        assert_eq!(server.appends.load(Ordering::SeqCst), 1);
        assert_eq!(
            server.opens.load(Ordering::SeqCst),
            if stage == "before" { 2 } else { 1 }
        );
        assert_eq!(request(&p, json!({"op":"outbox"})).await["total"], 0);
    }
}

#[tokio::test]
async fn cancelled_copy_waiter_keeps_account_ownership_until_append_finishes() {
    let (_dir, p) = profile().await;
    setup(&p, "delivered").await;
    let server = transport(&p);
    server.hold.store(true, Ordering::SeqCst);
    let clone = MobileProfile {
        database: p.database.clone(),
        operations: p.operations.clone(),
    };
    let pending = tokio::spawn(async move { copy(&clone, true, false).await });
    wait(&server.entered).await;
    pending.abort();
    let error: Value = serde_json::from_str(
        &p.request(json!({"op":"recover_outgoing","id":"copy","action":"local"}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("operation in progress")
    );
    assert_eq!(
        request(&p, json!({"op":"page","folder":"INBOX"})).await["total"],
        0
    );
    let row = request(&p, json!({"op":"outbox"})).await;
    assert_eq!(row["rows"][0]["sent"], "appending");
    server.release.notify_one();
    let _guard = p.operations.account("fixture").await;
    assert_eq!(request(&p, json!({"op":"outbox"})).await["total"], 0);
    assert_eq!(server.appends.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn changed_accounts_and_server_managed_policy_do_not_upload() {
    let (_dir, p) = profile().await;
    setup(&p, "delivered").await;
    let server = transport(&p);
    request(&p,json!({"op":"save_sent_preferences","id":"fixture","policy":"ServerManaged","folder":"Sent Mail"})).await;
    assert!(
        copy(&p, true, false).await["error"]
            .as_str()
            .unwrap()
            .contains("configured to save")
    );
    assert_eq!(server.appends.load(Ordering::SeqCst), 0);
    sql(
        &p,
        "UPDATE accounts SET settings=json_set(settings,'$.email','other@example.test');",
    )
    .await;
    assert!(
        copy(&p, false, false).await["error"]
            .as_str()
            .unwrap()
            .contains("account changed")
    );
    assert_eq!(server.opens.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn smtp_acknowledges_before_automatic_sent_copy_and_retains_its_background_owner() {
    let (_dir, p) = profile().await;
    crate::outgoing_tests::setup(&p).await;
    sql(&p,"UPDATE accounts SET settings=json_set(settings,'$.protocol','Imap','$.port',993,'$.sent_copy','Automatic');").await;
    let server = transport(&p);
    server.hold.store(true, Ordering::SeqCst);
    let smtp = crate::outgoing_tests::smtp(&p, Ok(()), false);
    smtp.release.notify_one();
    let result = crate::outgoing_tests::send(&p).await;
    assert_eq!(result["state"], "delivered");
    wait(&server.entered).await;
    let id = result["id"].as_str().unwrap();
    let error: Value = serde_json::from_str(
        &p.request(json!({"op":"recover_outgoing","id":id,"action":"local"}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(
        error["error"]
            .as_str()
            .unwrap()
            .contains("operation in progress")
    );
    assert_eq!(request(&p, json!({"op":"drafts"})).await, json!([]));
    assert_eq!(
        request(&p, json!({"op":"page","folder":"Sent"})).await["total"],
        1
    );
    let raw = p
        .database
        .read(|db| Ok(db.query_row("SELECT raw FROM outgoing", [], |r| r.get::<_, Vec<u8>>(0))?))
        .await
        .unwrap();
    assert_eq!(server.bytes.lock().unwrap()[0], raw);
    server.release.notify_one();
    let _guard = p.operations.account("fixture").await;
    assert_eq!(request(&p, json!({"op":"outbox"})).await["total"], 0);
}

#[tokio::test]
async fn synced_sent_deduplicates_only_the_untouched_local_copy_in_the_acknowledged_folder() {
    for edited in [false, true] {
        let (_dir, p) = profile().await;
        let raw = setup(&p, "delivered").await;
        let server = transport(&p);
        assert_eq!(copy(&p, true, false).await["data"]["sent"], "saved");
        if edited {
            for starred in [true, false] {
                request(
                    &p,
                    json!({"op":"mutate","id":"fixture:Sent:local-sent-copy","starred":starred}),
                )
                .await;
            }
        }
        for folder in ["Other", "Sent Mail"] {
            let remote = parse_mail("fixture", "91.4", folder, raw.clone(), false, false).unwrap();
            p.database
                .write(move |db| crate::operations::insert_mail(db, remote, false))
                .await
                .unwrap();
            let local = request(&p, json!({"op":"page","folder":"Sent"})).await;
            assert_eq!(
                local["total"],
                if edited && folder == "Sent Mail" {
                    2
                } else {
                    1
                }
            );
        }
        let local = request(
            &p,
            json!({"op":"detail","id":"fixture:Sent:local-sent-copy"}),
        )
        .await;
        assert_eq!(
            local["summary"]["remote_id"],
            if edited { "local-sent-copy" } else { "91.4" }
        );
        if !edited {
            let provider = request(&p, json!({"op":"detail","id":"fixture:Sent Mail:91.4"})).await;
            assert_eq!(provider, local);
        }
        assert_eq!(server.appends.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn restart_does_not_repeat_appending_and_manual_mark_can_finish_with_local_choice() {
    let (dir, p) = profile().await;
    setup(&p, "uncertain").await;
    sql(
        &p,
        "UPDATE outgoing_sent SET state='appending',folder='Original Sent';",
    )
    .await;
    drop(p);
    let p = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let server = transport(&p);
    let row = request(&p, json!({"op":"outbox"})).await;
    assert_eq!(row["rows"][0]["sent"], "uncertain");
    assert!(
        copy(&p, true, true).await["error"]
            .as_str()
            .unwrap()
            .contains("Review delivery")
    );
    request(
        &p,
        json!({"op":"recover_outgoing","id":"copy","action":"mark","confirmed":true}),
    )
    .await;
    let row = request(&p, json!({"op":"outbox"})).await;
    assert_eq!(row["rows"][0]["marked"], true);
    assert!(
        copy(&p, true, false).await["error"]
            .as_str()
            .unwrap()
            .contains("confirm")
    );
    assert_eq!(server.opens.load(Ordering::SeqCst), 0);
    request(
        &p,
        json!({"op":"recover_outgoing","id":"copy","action":"local"}),
    )
    .await;
    assert_eq!(request(&p, json!({"op":"outbox"})).await["total"], 0);
    assert_eq!(server.appends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn provider_confirmed_copy_blocks_stale_return_and_reconnect_preserves_newer_sent_preferences()
 {
    let (_dir, p) = profile().await;
    setup(&p, "uncertain").await;
    let server = transport(&p);
    server.found.store(true, Ordering::SeqCst);
    sql(&p,"CREATE TRIGGER fail BEFORE UPDATE OF state ON outgoing_sent WHEN NEW.state='saved' BEGIN SELECT RAISE(FAIL,'Synthetic disk full'); END;").await;
    assert!(copy(&p, false, false).await["error"].is_string());
    let stale: Value = serde_json::from_str(
        &p.request(
            json!({"op":"recover_outgoing","id":"copy","action":"return","confirmed":true})
                .to_string(),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert!(
        stale["error"]
            .as_str()
            .unwrap()
            .contains("already confirmed")
    );
    sql(&p, "DROP TRIGGER fail;").await;
    assert_eq!(copy(&p, false, false).await["data"]["sent"], "saved");
    let old = request(&p, json!({"op":"accounts"})).await["accounts"][0].clone();
    request(&p,json!({"op":"save_sent_preferences","id":"fixture","policy":"ServerManaged","folder":"New Sent"})).await;
    request(
        &p,
        json!({"op":"save_account","account":old,"preserve_sent":true}),
    )
    .await;
    let saved = request(&p, json!({"op":"accounts"})).await;
    assert_eq!(saved["accounts"][0]["sent_copy"], "ServerManaged");
    assert_eq!(saved["accounts"][0]["sent_folder"], "New Sent");
}

#[tokio::test]
async fn local_sent_actions_need_no_password_but_remote_actions_request_credentials_without_writes()
{
    let (dir, p) = profile().await;
    let raw = setup(&p, "delivered").await;
    transport(&p);
    assert_eq!(copy(&p, true, false).await["data"]["sent"], "saved");
    let local = "fixture:Sent:local-sent-copy";
    for fields in [
        json!({"starred":true}),
        json!({"unread":true}),
        json!({"folder":"Archive"}),
    ] {
        let mut action = json!({"op":"mutate","id":local});
        action
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        assert_eq!(request(&p, action).await, json!({"committed":true}));
    }
    let mut remote = parse_mail("fixture", "91.4", "Other", raw, false, false).unwrap();
    // A stable UI ID can retain its local prefix after a provider handover.
    // Credential routing must use the current remote_id, never this UI ID.
    remote.summary.id = "fixture:Sent:local-sent-provider".into();
    p.database
        .write(move |db| crate::operations::insert_mail(db, remote, false))
        .await
        .unwrap();
    for fields in [
        json!({"starred":true}),
        json!({"unread":true}),
        json!({"folder":"Archive"}),
    ] {
        let mut action = json!({"op":"mutate","id":"fixture:Sent:local-sent-provider"});
        action
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        assert_eq!(
            request(&p, action).await,
            json!({"requires_credentials":"fixture"})
        );
    }
    p.database
        .read(|db| {
            let remote = crate::operations::stored_mail(db, "fixture:Sent:local-sent-provider")?;
            assert!(!remote.starred && !remote.unread);
            assert_eq!(remote.folder, "Other");
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM pending_moves", [], |r| r
                    .get::<_, i64>(0))?,
                0
            );
            assert_eq!(
                db.query_row(
                    "SELECT local_edited FROM outgoing_sent WHERE id='copy'",
                    [],
                    |r| r.get::<_, i64>(0)
                )?,
                1
            );
            Ok(())
        })
        .await
        .unwrap();
    drop(p);
    let p = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let cached = request(&p, json!({"op":"page","folder":"Archive"})).await;
    assert_eq!(cached["mail"][0]["id"], local);
    assert_eq!(cached["mail"][0]["starred"], true);
    assert_eq!(cached["mail"][0]["unread"], true);
    assert_eq!(
        request(&p, json!({"op":"mutate","id":local,"folder":"Sent"})).await,
        json!({"committed":true})
    );
}

#[tokio::test]
async fn local_sent_edits_finish_with_every_provider_slot_and_account_lock_occupied() {
    let (_dir, p) = profile().await;
    let raw = setup(&p, "delivered").await;
    transport(&p);
    assert_eq!(copy(&p, true, false).await["data"]["sent"], "saved");
    let _capacity = p.operations.hold_network_capacity().await;
    let _account = p.operations.account("fixture").await;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        request(
            &p,
            json!({"op":"mutate","id":"fixture:Sent:local-sent-copy","starred":true}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(result["committed"], true);
    let remote = parse_mail("fixture", "91.4", "Sent Mail", raw, false, false).unwrap();
    p.database
        .write(move |db| crate::operations::insert_mail(db, remote, false))
        .await
        .unwrap();
    let page = request(&p, json!({"op":"page","folder":"Sent"})).await;
    assert_eq!(page["total"], 2);
    assert_eq!(
        page["mail"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["remote_id"] == "local-sent-copy")
            .unwrap()["starred"],
        true
    );
}

#[tokio::test]
async fn sent_handover_rolls_back_as_one_transaction_and_aliases_survive_reopen_and_reply() {
    let (dir, p) = profile().await;
    let raw = setup(&p, "delivered").await;
    transport(&p);
    assert_eq!(copy(&p, true, false).await["data"]["sent"], "saved");
    sql(&p,"CREATE TRIGGER reject_handover BEFORE UPDATE OF remote_id ON mail BEGIN SELECT RAISE(ABORT,'fixture handover failure'); END;").await;
    let remote = parse_mail("fixture", "91.4", "Sent Mail", raw.clone(), false, false).unwrap();
    assert!(
        p.database
            .write(move |db| crate::operations::insert_mail(db, remote, false))
            .await
            .is_err()
    );
    p.database
        .read(|db| {
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM mail", [], |r| r.get::<_, i64>(0))?,
                1
            );
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM mail_aliases", [], |r| r
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
    sql(&p, "DROP TRIGGER reject_handover;").await;
    for _ in 0..2 {
        let remote = parse_mail("fixture", "91.4", "Sent Mail", raw.clone(), false, false).unwrap();
        p.database
            .write(move |db| crate::operations::insert_mail(db, remote, false))
            .await
            .unwrap();
    }
    drop(p);
    let p = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let page = request(&p, json!({"op":"page","folder":"Sent"})).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["mail"][0]["id"], "fixture:Sent:local-sent-copy");
    assert_eq!(page["mail"][0]["folder"], "Sent Mail");
    assert_eq!(
        page["aliases"]["fixture:Sent Mail:91.4"],
        "fixture:Sent:local-sent-copy"
    );
    assert_eq!(page["folder_membership"]["fixture"], json!(["Sent Mail"]));
    let a = request(
        &p,
        json!({"op":"reply","id":"fixture:Sent:local-sent-copy","all":true}),
    )
    .await;
    let b = request(
        &p,
        json!({"op":"reply","id":"fixture:Sent Mail:91.4","all":true}),
    )
    .await;
    assert_eq!(a["body"], b["body"]);
    assert_eq!(a["in_reply_to"], b["in_reply_to"]);
    let bytes = p
        .database
        .read(|db| {
            Ok(
                db.query_row("SELECT raw FROM outgoing WHERE id='copy'", [], |r| {
                    r.get::<_, Vec<u8>>(0)
                })?,
            )
        })
        .await
        .unwrap();
    assert_eq!(bytes, raw);
}

struct SyncHandover {
    raw: Vec<u8>,
    entered: Notify,
    release: Notify,
    flags: Mutex<Vec<(Mail, shep_mail_core::mail_actions::Flags)>>,
}
#[async_trait]
impl shep_mail_core::providers::MailProvider for SyncHandover {
    async fn sync(
        &self,
        account: &Account,
        _password: &SecretString,
        _known: &std::collections::HashSet<String>,
        output: tokio::sync::mpsc::Sender<MailSyncItem>,
    ) -> anyhow::Result<Vec<String>> {
        self.entered.notify_one();
        self.release.notified().await;
        output
            .send(MailSyncItem::SentFolder(
                account.id.clone(),
                Some("Sent Mail".into()),
            ))
            .await?;
        output
            .send(MailSyncItem::Message(parse_mail(
                &account.id,
                "91.4",
                "Sent Mail",
                self.raw.clone(),
                false,
                false,
            )?))
            .await?;
        Ok(vec![])
    }
    async fn move_mail(
        &self,
        _account: &Account,
        _password: &SecretString,
        _mail: &Mail,
        _folder: &str,
    ) -> anyhow::Result<Option<String>> {
        panic!("This fixture must not move mail")
    }
    async fn set_flags(
        &self,
        _account: &Account,
        _password: &SecretString,
        mail: &Mail,
        flags: shep_mail_core::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        self.flags.lock().unwrap().push((mail.clone(), flags));
        Ok(())
    }
}

#[tokio::test]
async fn queued_provider_action_resolves_the_retained_id_after_sent_handover() {
    let (_dir, p) = profile().await;
    let raw = setup(&p, "delivered").await;
    // Cache a server row before a separate Sent lookup finishes, so a user can
    // legitimately have selected the provider ID before adoption happens.
    let remote = parse_mail("fixture", "91.4", "Sent Mail", raw.clone(), false, false).unwrap();
    p.database
        .write(move |db| crate::operations::insert_mail(db, remote, false))
        .await
        .unwrap();
    // Resume a durable checkpoint after saving the receipt, before adopting
    // the cached server row. The subsequent real Sync dispatcher does adoption.
    p.database.write(|db|{
        crate::outgoing::local_sent(db,"copy",None)?;
        db.execute("UPDATE outgoing_sent SET state='saved',folder='Sent Mail',receipt=?1 WHERE id='copy'",[serde_json::to_string(&SentReceipt{folder:"Sent Mail".into(),remote_id:Some("91.4".into())})?])?;
        Ok(())
    }).await.unwrap();
    assert_eq!(
        request(&p, json!({"op":"detail","id":"fixture:Sent Mail:91.4"})).await["summary"]["id"],
        "fixture:Sent Mail:91.4"
    );
    let server = Arc::new(SyncHandover {
        raw,
        entered: Notify::new(),
        release: Notify::new(),
        flags: Mutex::new(vec![]),
    });
    *p.operations.provider.lock().unwrap() = Some(server.clone());
    let p = Arc::new(p);
    let sync_profile = p.clone();
    let syncing = tokio::spawn(async move {
        request(
            &sync_profile,
            json!({"op":"sync","account":"fixture","password":"synthetic"}),
        )
        .await
    });
    wait(&server.entered).await;
    let action_profile = p.clone();
    let action = tokio::spawn(async move {
        request(&action_profile,json!({"op":"mutate","id":"fixture:Sent Mail:91.4","starred":true,"password":"synthetic"})).await
    });
    wait(&p.operations.mutation_waiting).await;
    assert!(server.flags.lock().unwrap().is_empty());
    server.release.notify_one();
    syncing.await.unwrap();
    assert_eq!(action.await.unwrap()["committed"], true);
    {
        let flags = server.flags.lock().unwrap();
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].0.id, "fixture:Sent:local-sent-copy");
        assert_eq!(flags[0].0.remote_id, "91.4");
        assert_eq!(flags[0].0.folder, "Sent Mail");
        assert_eq!(flags[0].1.starred, Some(true));
    }
    let page = request(&p, json!({"op":"page","folder":"Sent"})).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["mail"][0]["starred"], true);
}

#[tokio::test]
async fn sent_handover_refuses_a_different_acknowledged_uid_or_ambiguous_submission() {
    for ambiguous in [false, true] {
        let (_dir, p) = profile().await;
        let raw = setup(&p, "delivered").await;
        transport(&p);
        assert_eq!(copy(&p, true, false).await["data"]["sent"], "saved");
        if ambiguous {
            sql(&p,"INSERT INTO outgoing SELECT 'copy-two','draft-two',state,account_id,message_id,raw,json_set(draft,'$.id','draft-two') FROM outgoing WHERE id='copy'; INSERT INTO outgoing_sent(id,account,state,folder,receipt) SELECT 'copy-two',account,state,folder,receipt FROM outgoing_sent WHERE id='copy';").await;
        } else {
            p.database
                .write(|db| {
                    db.execute(
                        "UPDATE outgoing_sent SET receipt=?1 WHERE id='copy'",
                        [serde_json::to_string(&SentReceipt {
                            folder: "Sent Mail".into(),
                            remote_id: Some("92.4".into()),
                        })?],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
        }
        let remote = parse_mail("fixture", "91.4", "Sent Mail", raw, false, false).unwrap();
        p.database
            .write(move |db| crate::operations::insert_mail(db, remote, false))
            .await
            .unwrap();
        let page = request(&p, json!({"op":"page","folder":"Sent"})).await;
        assert_eq!(page["total"], 2);
        assert_eq!(page["aliases"], json!({}));
        assert_eq!(
            request(
                &p,
                json!({"op":"detail","id":"fixture:Sent:local-sent-copy"})
            )
            .await["summary"]["remote_id"],
            "local-sent-copy"
        );
    }
}

#[tokio::test]
async fn version_five_sent_folder_migration_is_atomic_and_preserves_original_mail() {
    let (dir, p) = profile().await;
    setup(&p, "delivered").await;
    transport(&p);
    assert_eq!(copy(&p, true, false).await["data"]["sent"], "saved");
    sql(&p,"DELETE FROM known_sent_folders; PRAGMA user_version=5; CREATE TRIGGER reject_sent_migration BEFORE INSERT ON known_sent_folders BEGIN SELECT RAISE(ABORT,'fixture migration failure'); END;").await;
    drop(p);
    let path = dir.path().join("mail.sqlite3");
    assert!(
        MobileProfile::open(path.to_str().unwrap().into())
            .await
            .is_err()
    );
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            5
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM mail", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        db.execute_batch("DROP TRIGGER reject_sent_migration;")
            .unwrap();
    }
    let p = MobileProfile::open(path.to_str().unwrap().into())
        .await
        .unwrap();
    p.database
        .read(|db| {
            assert_eq!(
                db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
                6
            );
            assert_eq!(
                db.query_row(
                    "SELECT folder FROM known_sent_folders WHERE account_id='fixture'",
                    [],
                    |r| r.get::<_, String>(0)
                )?,
                "Sent Mail"
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_pop_uidl_prefix_cannot_mark_another_accounts_sent_copy_as_locally_edited() {
    let (_dir, p) = profile().await;
    let raw = setup(&p, "delivered").await;
    transport(&p);
    assert_eq!(copy(&p, true, false).await["data"]["sent"], "saved");
    let mut pop = account();
    pop.id = "another-account".into();
    request(&p, json!({"op":"save_account","account":pop})).await;
    let incoming = parse_mail(
        "another-account",
        "local-sent-copy",
        "INBOX",
        raw.clone(),
        false,
        false,
    )
    .unwrap();
    let id = incoming.summary.id.clone();
    p.database
        .write(move |db| crate::operations::insert_mail(db, incoming, true))
        .await
        .unwrap();
    assert_eq!(
        request(&p, json!({"op":"mutate","id":id,"starred":true})).await["committed"],
        true
    );
    let remote = parse_mail("fixture", "91.4", "Sent Mail", raw, false, false).unwrap();
    p.database
        .write(move |db| crate::operations::insert_mail(db, remote, false))
        .await
        .unwrap();
    let page = request(&p, json!({"op":"page","folder":"Sent"})).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["mail"][0]["remote_id"], "91.4");
    assert_eq!(page["mail"][0]["id"], "fixture:Sent:local-sent-copy");
}
