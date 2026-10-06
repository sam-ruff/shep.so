use crate::tests::{profile, request, seed};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

async fn groups(p: &crate::api::MobileProfile, command: Value) -> Value {
    request(p, json!({"op":"groups","command":command})).await
}
async fn failure(p: &crate::api::MobileProfile, request: Value) -> String {
    let result: Value =
        serde_json::from_str(&p.request(request.to_string()).await.unwrap()).unwrap();
    result["error"]
        .as_str()
        .unwrap_or_else(|| panic!("expected an error: {result}"))
        .to_owned()
}
async fn group_failure(p: &crate::api::MobileProfile, command: Value) -> String {
    failure(p, json!({"op":"groups","command":command})).await
}
async fn page(p: &crate::api::MobileProfile, folder: &str) -> Value {
    request(p, json!({"op":"page","folder":folder})).await
}
async fn select_all(p: &crate::api::MobileProfile, id: &str) -> Value {
    request(p,json!({"op":"selection","command":{"kind":"capture","id":id,"revision":0,"all":true,"scope":{"folder":"Inbox"}}})).await
}
async fn review(p: &crate::api::MobileProfile, job: &str, action: Value) -> Value {
    let captured = select_all(p, &format!("{job}-selection")).await;
    let review = groups(p,json!({"kind":"prepare","id":job,"selection":format!("{job}-selection"),"expected":captured["revision"],"action":action,"scope":{"folder":"Inbox"}})).await;
    request(
        p,
        json!({"op":"selection","command":{"kind":"release","id":captured["id"]}}),
    )
    .await;
    review
}
async fn approve(p: &crate::api::MobileProfile, job: &str) -> Value {
    groups(p, json!({"kind":"approve","id":job})).await
}
async fn step(p: &crate::api::MobileProfile, password: Option<&str>) -> Value {
    groups(p, json!({"kind":"step","password":password})).await
}
async fn run_all(p: &crate::api::MobileProfile, password: Option<&str>) -> Vec<Value> {
    let mut steps = Vec::new();
    loop {
        let step = step(p, password).await;
        if step["idle"] == true {
            return steps;
        }
        assert!(step["requires_credentials"].is_null(), "{step}");
        steps.push(step);
        assert!(steps.len() < 1000, "runaway execution");
    }
}
async fn job(p: &crate::api::MobileProfile, id: &str) -> Value {
    groups(p, json!({"kind":"history"})).await["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["id"] == id)
        .cloned()
        .unwrap_or(Value::Null)
}
async fn items(p: &crate::api::MobileProfile, id: &str, after: Option<i64>) -> Value {
    groups(p, json!({"kind":"items","id":id,"after":after})).await
}
fn outcomes(steps: &[Value], outcome: &str) -> usize {
    steps.iter().filter(|s| s["outcome"] == outcome).count()
}
async fn make_imap(p: &crate::api::MobileProfile) {
    p.database
        .write(|db| {
            db.execute(
                "UPDATE accounts SET settings=json_set(settings,'$.protocol','Imap') WHERE id='fixture'",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

/// Scripted provider: each move/flag call consumes the next scripted reply.
struct Scripted {
    moves: AtomicUsize,
    flags: AtomicUsize,
    replies: Mutex<Vec<Result<Option<String>, String>>>,
    started: tokio::sync::Notify,
    gate: Mutex<Option<Arc<tokio::sync::Notify>>>,
}
impl Scripted {
    fn new(replies: Vec<Result<Option<String>, String>>) -> Arc<Self> {
        Arc::new(Self {
            moves: AtomicUsize::new(0),
            flags: AtomicUsize::new(0),
            replies: Mutex::new(replies),
            started: tokio::sync::Notify::new(),
            gate: Mutex::new(None),
        })
    }
    async fn reply(&self) -> anyhow::Result<Option<String>> {
        self.started.notify_one();
        let gate = self.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.notified().await;
        }
        let next = {
            let mut replies = self.replies.lock().unwrap();
            if replies.is_empty() {
                Ok(Some("900".to_owned()))
            } else {
                replies.remove(0)
            }
        };
        next.map_err(|text| anyhow::anyhow!(text))
    }
}
#[async_trait::async_trait]
impl shep_mail_core::providers::MailProvider for Scripted {
    async fn sync(
        &self,
        _account: &shep_mail_core::model::Account,
        _password: &secrecy::SecretString,
        _known: &std::collections::HashSet<String>,
        _output: tokio::sync::mpsc::Sender<shep_mail_core::model::MailSyncItem>,
    ) -> anyhow::Result<Vec<String>> {
        Ok(vec![])
    }
    async fn move_mail(
        &self,
        _a: &shep_mail_core::model::Account,
        _p: &secrecy::SecretString,
        mail: &shep_mail_core::model::Mail,
        _folder: &str,
    ) -> anyhow::Result<Option<String>> {
        let n = self.moves.fetch_add(1, Ordering::SeqCst);
        let reply = self.reply().await?;
        Ok(reply.map(|_| format!("moved-{}-{n}", mail.remote_id)))
    }
    async fn set_flags(
        &self,
        _a: &shep_mail_core::model::Account,
        _p: &secrecy::SecretString,
        _m: &shep_mail_core::model::Mail,
        _changes: shep_mail_core::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        self.flags.fetch_add(1, Ordering::SeqCst);
        match self.reply().await {
            Err(error) if error.to_string() == "NO STORE failed" => {
                Err(shep_mail_core::mail_actions::FlagsRejected(error.to_string()).into())
            }
            result => result.map(|_| ()),
        }
    }
}

#[tokio::test]
async fn frozen_review_stages_exact_membership_and_executes_every_step_locally() {
    let (_dir, p) = profile().await;
    seed(&p, 125).await;
    let reviewed = review(&p, "archive", json!({"kind":"archive"})).await;
    assert_eq!(reviewed["state"], "review");
    assert_eq!(reviewed["total"], 125);
    assert_eq!(reviewed["groups"][0]["account"], "fixture");
    assert_eq!(reviewed["groups"][0]["folder"], "INBOX");
    assert_eq!(reviewed["groups"][0]["total"], 125);
    assert_eq!(reviewed["groups"][0]["unread"], 63);
    assert_eq!(reviewed["counts"]["pending"], 125);
    // The frozen review does not paint until it is approved.
    assert_eq!(page(&p, "Inbox").await["total"], 125);
    assert_eq!(step(&p, None).await["idle"], true);
    let approved = approve(&p, "archive").await;
    assert_eq!(approved["state"], "running");
    // Approved intent paints before any step runs, in the page and its counts.
    let painted = page(&p, "Inbox").await;
    assert_eq!(painted["total"], 0);
    assert_eq!(painted["unread"], 0);
    assert_eq!(page(&p, "Archive").await["total"], 125);
    let steps = run_all(&p, None).await;
    assert_eq!(steps.len(), 125);
    assert_eq!(outcomes(&steps, "done"), 125);
    let finished = job(&p, "archive").await;
    assert_eq!(finished["state"], "finished");
    assert_eq!(finished["counts"]["done"], 125);
    assert_eq!(finished["undo"], false);
    assert_eq!(page(&p, "Inbox").await["total"], 0);
    assert_eq!(page(&p, "Archive").await["total"], 125);
    let first = items(&p, "archive", None).await;
    assert_eq!(first["rows"].as_array().unwrap().len(), 50);
    assert_eq!(first["rows"][0]["state"], "done");
    assert_eq!(first["rows"][0]["receipt"]["dispatch"]["folder"], "INBOX");
    assert_eq!(first["rows"][0]["receipt"]["after"]["folder"], "Archive");
    assert_eq!(first["rows"][0]["receipt"]["applied"]["folder"], "Archive");
    assert!(
        first["rows"][0]["subject"]
            .as_str()
            .unwrap()
            .starts_with("Message")
    );
    let after = first["next_after"].as_i64().unwrap();
    let second = items(&p, "archive", Some(after)).await;
    assert_eq!(second["rows"].as_array().unwrap().len(), 50);
    let third = items(&p, "archive", second["next_after"].as_i64()).await;
    assert_eq!(third["rows"].as_array().unwrap().len(), 25);
    assert!(third["next_after"].is_null());
    // Only the temporary frozen copy is released during preparation.
    let released = failure(
        &p,
        json!({"op":"selection","command":{"kind":"observe","id":"archive-frozen"}}),
    )
    .await;
    assert!(released.contains("no longer available"), "{released}");
}

#[tokio::test]
async fn declined_review_preserves_the_exact_original_capture_across_pages() {
    let (_dir, p) = profile().await;
    seed(&p, 125).await;
    let captured = select_all(&p, "declined-selection").await;
    let reviewed = groups(&p,json!({"kind":"prepare","id":"declined","selection":"declined-selection","expected":captured["revision"],"action":{"kind":"archive"},"scope":{"folder":"Inbox"}})).await;
    let inspect = groups(&p, json!({"kind":"inspect","id":"declined"})).await;
    assert_eq!(inspect, reviewed);
    groups(&p, json!({"kind":"decline","id":"declined"})).await;
    let observed = request(
        &p,
        json!({"op":"selection","command":{"kind":"observe","id":"declined-selection"}}),
    )
    .await;
    assert_eq!(observed["selected"], 125);
    let mut after = None;
    let mut total = 0;
    loop {
        let selected = request(&p, json!({"op":"selection","command":{"kind":"page","id":"declined-selection","expected":observed["revision"],"after":after}})).await;
        let rows = selected["rows"].as_array().expect("selected page");
        assert!(rows.len() <= 50);
        total += rows.len();
        after = selected["next_after"].as_u64();
        if after.is_none() {
            break;
        }
    }
    assert_eq!(total, 125);
    let next = groups(&p, json!({"kind":"prepare","id":"next","selection":"declined-selection","expected":observed["revision"],"action":{"kind":"flag"},"scope":{"folder":"Inbox"}})).await;
    assert_eq!(next["total"], 125);
    let approved = approve(&p, "next").await;
    assert_eq!(
        groups(&p, json!({"kind":"inspect","id":"next"})).await,
        approved
    );
    let repeated = group_failure(&p, json!({"kind":"approve","id":"next"})).await;
    assert!(repeated.contains("no longer open"), "{repeated}");
    request(
        &p,
        json!({"op":"selection","command":{"kind":"release","id":"declined-selection"}}),
    )
    .await;
    let released = failure(
        &p,
        json!({"op":"selection","command":{"kind":"observe","id":"declined-selection"}}),
    )
    .await;
    assert!(released.contains("no longer available"), "{released}");
}

#[tokio::test]
async fn failed_prepare_preserves_the_original_capture_for_retry() {
    let (_dir, p) = profile().await;
    seed(&p, 125).await;
    let captured = select_all(&p, "retry-selection").await;
    let rejected = group_failure(&p, json!({"kind":"prepare","id":"bad","selection":"retry-selection","expected":999,"action":{"kind":"archive"},"scope":{"folder":"Inbox"}})).await;
    assert!(rejected.contains("selection changed"), "{rejected}");
    let observed = request(
        &p,
        json!({"op":"selection","command":{"kind":"observe","id":"retry-selection"}}),
    )
    .await;
    assert_eq!(observed["id"], captured["id"]);
    assert_eq!(observed["revision"], captured["revision"]);
    assert_eq!(observed["selected"], 125);
    p.database
        .write(|db| {
            db.execute_batch("CREATE TRIGGER refuse_review_page BEFORE INSERT ON group_items BEGIN SELECT RAISE(ABORT,'review storage fixture failure'); END")?;
            Ok(())
        })
        .await
        .expect("install staging failure");
    let failed = group_failure(&p,json!({"kind":"prepare","id":"failed-copy","selection":"retry-selection","expected":observed["revision"],"action":{"kind":"archive"},"scope":{"folder":"Inbox"}})).await;
    assert!(!failed.is_empty());
    let interrupted = groups(&p, json!({"kind":"inspect","id":"failed-copy"})).await;
    assert_eq!(interrupted["state"], "interrupted");
    assert_eq!(interrupted["total"], 0);
    let frozen = failure(
        &p,
        json!({"op":"selection","command":{"kind":"observe","id":"failed-copy-frozen"}}),
    )
    .await;
    assert!(frozen.contains("no longer available"), "{frozen}");
    let retained = request(
        &p,
        json!({"op":"selection","command":{"kind":"observe","id":"retry-selection"}}),
    )
    .await;
    assert_eq!(retained["revision"], captured["revision"]);
    assert_eq!(retained["selected"], 125);
    p.database
        .write(|db| {
            db.execute_batch("DROP TRIGGER refuse_review_page")?;
            Ok(())
        })
        .await
        .expect("restore staging writes");
    let retried = groups(&p, json!({"kind":"prepare","id":"retry","selection":"retry-selection","expected":observed["revision"],"action":{"kind":"archive"},"scope":{"folder":"Inbox"}})).await;
    assert_eq!(retried["total"], 125);
}

#[tokio::test]
async fn newer_individual_intent_and_applied_values_become_distinct_skips() {
    let (_dir, p) = profile().await;
    seed(&p, 10).await;
    let inbox = page(&p, "Inbox").await;
    let unread: Vec<String> = inbox["mail"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["unread"] == true)
        .map(|m| m["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(unread.len(), 5);
    review(&p, "read", json!({"kind":"read"})).await;
    approve(&p, "read").await;
    assert_eq!(page(&p, "Inbox").await["unread"], 0);
    // An individual choice made after approval owns its field.
    request(&p, json!({"op":"mutate","id":unread[0],"unread":true})).await;
    let steps = run_all(&p, None).await;
    assert_eq!(steps.len(), 10);
    assert_eq!(outcomes(&steps, "done"), 4);
    assert_eq!(outcomes(&steps, "skipped"), 6);
    let newer = steps
        .iter()
        .find(|s| s["mail"] == unread[0] || s["reason"].as_str().unwrap_or("").contains("newer"));
    assert!(
        newer.unwrap()["reason"]
            .as_str()
            .unwrap()
            .contains("newer change")
    );
    assert_eq!(
        steps
            .iter()
            .filter(|s| s["reason"] == "Already up to date")
            .count(),
        5
    );
    let after = page(&p, "Inbox").await;
    assert_eq!(after["unread"], 1);
    let kept = after["mail"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == unread[0])
        .unwrap();
    assert_eq!(kept["unread"], true);
    let finished = job(&p, "read").await;
    assert_eq!(finished["counts"]["done"], 4);
    assert_eq!(finished["counts"]["skipped"], 6);
}

#[tokio::test]
async fn undo_cancels_unsent_steps_and_reverses_receipts_to_their_baseline() {
    let (_dir, p) = profile().await;
    seed(&p, 6).await;
    review(&p, "delete", json!({"kind":"delete"})).await;
    approve(&p, "delete").await;
    for _ in 0..3 {
        assert_eq!(step(&p, None).await["outcome"], "done");
    }
    assert_eq!(page(&p, "Trash").await["total"], 6);
    let undone = groups(&p, json!({"kind":"undo","id":"delete"})).await;
    assert_eq!(undone["state"], "undoing");
    assert_eq!(undone["undo"], true);
    assert_eq!(undone["counts"]["cancelled"], 3);
    assert_eq!(undone["counts"]["undoing"], 3);
    // The Undo decision paints the baseline before its inverse steps run.
    assert_eq!(page(&p, "Inbox").await["total"], 6);
    assert_eq!(page(&p, "Trash").await["total"], 0);
    let steps = run_all(&p, None).await;
    assert_eq!(steps.len(), 3);
    assert_eq!(outcomes(&steps, "undone"), 3);
    let finished = job(&p, "delete").await;
    assert_eq!(finished["state"], "finished");
    assert_eq!(finished["counts"]["undone"], 3);
    assert_eq!(finished["counts"]["cancelled"], 3);
    assert_eq!(page(&p, "Inbox").await["total"], 6);
    let rows = items(&p, "delete", None).await;
    let restored = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["state"] == "undone")
        .unwrap();
    assert_eq!(restored["receipt"]["after"]["folder"], "INBOX");
    assert_eq!(restored["receipt"]["dispatch"]["folder"], "INBOX");
    let cancelled = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["state"] == "cancelled")
        .unwrap();
    assert_eq!(cancelled["reason"], "Cancelled before sending");
    assert!(
        group_failure(&p, json!({"kind":"undo","id":"delete"}))
            .await
            .contains("already")
    );
}

#[tokio::test]
async fn provider_steps_need_credentials_pause_on_uncertainty_and_never_repeat() {
    let (_dir, p) = profile().await;
    seed(&p, 4).await;
    make_imap(&p).await;
    let provider = Scripted::new(vec![
        Err("connection reset".into()),
        Ok(Some("1".into())),
        Ok(Some("2".into())),
        Ok(Some("3".into())),
    ]);
    *p.operations.provider.lock().unwrap() = Some(provider.clone());
    review(&p, "archive", json!({"kind":"archive"})).await;
    approve(&p, "archive").await;
    let asked = step(&p, None).await;
    assert_eq!(asked["requires_credentials"], "fixture");
    assert_eq!(provider.moves.load(Ordering::SeqCst), 0);
    let first = step(&p, Some("secret")).await;
    assert_eq!(first["outcome"], "uncertain");
    assert!(
        first["reason"]
            .as_str()
            .unwrap()
            .contains("will not repeat")
    );
    let paused = job(&p, "archive").await;
    assert_eq!(paused["state"], "paused");
    assert_eq!(paused["counts"]["uncertain"], 1);
    assert_eq!(step(&p, Some("secret")).await["idle"], true);
    assert_eq!(provider.moves.load(Ordering::SeqCst), 1);
    let position = first["position"].as_i64().unwrap();
    let refused = group_failure(
        &p,
        json!({"kind":"retry","id":"archive","position":position}),
    )
    .await;
    assert!(refused.contains("never repeats"), "{refused}");
    let accepted = groups(
        &p,
        json!({"kind":"accept","id":"archive","position":position}),
    )
    .await;
    assert_eq!(accepted["counts"]["accepted"], 1);
    assert_eq!(accepted["state"], "paused");
    // Accepting retires only the local intent: the cache row is unchanged.
    assert_eq!(
        page(&p, "Inbox").await["mail"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["folder"] == "INBOX")
            .count(),
        1
    );
    let resumed = groups(&p, json!({"kind":"resume","id":"archive"})).await;
    assert_eq!(resumed["state"], "running");
    let rest = run_all(&p, Some("secret")).await;
    assert_eq!(outcomes(&rest, "done"), 3);
    assert_eq!(provider.moves.load(Ordering::SeqCst), 4);
    let finished = job(&p, "archive").await;
    assert_eq!(finished["state"], "finished");
    assert_eq!(finished["counts"]["accepted"], 1);
    assert_eq!(finished["counts"]["done"], 3);
    let rows = items(&p, "archive", None).await;
    let moved = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["state"] == "done")
        .unwrap();
    assert!(
        moved["receipt"]["after"]["remote_id"]
            .as_str()
            .unwrap()
            .starts_with("moved-")
    );
    assert_eq!(moved["receipt"]["dispatch"]["folder"], "INBOX");
}

#[tokio::test]
async fn definite_flag_failures_are_retried_explicitly_and_undo_uses_inverse_receipts() {
    let (_dir, p) = profile().await;
    seed(&p, 3).await;
    make_imap(&p).await;
    let provider = Scripted::new(vec![Err("NO STORE failed".into())]);
    *p.operations.provider.lock().unwrap() = Some(provider.clone());
    review(&p, "flag", json!({"kind":"flag"})).await;
    approve(&p, "flag").await;
    let steps = run_all(&p, Some("secret")).await;
    assert_eq!(outcomes(&steps, "failed"), 1);
    assert_eq!(outcomes(&steps, "done"), 1);
    assert_eq!(outcomes(&steps, "skipped"), 1);
    let failed = steps.iter().find(|s| s["outcome"] == "failed").unwrap();
    let position = failed["position"].as_i64().unwrap();
    assert_eq!(job(&p, "flag").await["state"], "finished");
    let retried = groups(&p, json!({"kind":"retry","id":"flag","position":position})).await;
    assert_eq!(retried["state"], "running");
    let again = run_all(&p, Some("secret")).await;
    assert_eq!(outcomes(&again, "done"), 1);
    assert_eq!(provider.flags.load(Ordering::SeqCst), 3);
    let undone = groups(&p, json!({"kind":"undo","id":"flag"})).await;
    assert_eq!(undone["counts"]["undoing"], 2);
    assert_eq!(undone["counts"]["skipped"], 1);
    let inverse = run_all(&p, Some("secret")).await;
    assert_eq!(outcomes(&inverse, "undone"), 2);
    assert_eq!(provider.flags.load(Ordering::SeqCst), 5);
    let inbox = page(&p, "Inbox").await;
    assert_eq!(
        inbox["mail"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["starred"] == true)
            .count(),
        1
    );
}

#[tokio::test]
async fn restart_marks_claimed_steps_uncertain_pauses_and_retires_abandoned_reviews() {
    let (dir, p) = profile().await;
    let path = dir.path().join("mail.sqlite3").to_str().unwrap().to_owned();
    seed(&p, 5).await;
    review(&p, "abandoned", json!({"kind":"read"})).await;
    review(&p, "archive", json!({"kind":"archive"})).await;
    approve(&p, "archive").await;
    assert_eq!(step(&p, None).await["outcome"], "done");
    p.database
        .write(|db| {
            db.execute("UPDATE group_items SET state='sending',attempt='lost' WHERE job='archive' AND position=1", [])?;
            Ok(())
        })
        .await
        .unwrap();
    drop(p);
    let p = crate::api::MobileProfile::open(path).await.unwrap();
    let history = groups(&p, json!({"kind":"history"})).await;
    let ids: Vec<&str> = history["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["archive"]);
    assert_eq!(history["runnable"], false);
    let paused = job(&p, "archive").await;
    assert_eq!(paused["state"], "paused");
    assert_eq!(paused["counts"]["uncertain"], 1);
    assert_eq!(paused["counts"]["done"], 1);
    assert_eq!(paused["counts"]["pending"], 3);
    assert_eq!(step(&p, None).await["idle"], true);
    let rows = items(&p, "archive", None).await;
    assert!(
        rows["rows"][1]["reason"]
            .as_str()
            .unwrap()
            .contains("stopped before the server confirmed")
    );
    assert!(
        group_failure(&p, json!({"kind":"approve","id":"abandoned"}))
            .await
            .contains("no longer")
    );
    groups(&p, json!({"kind":"accept","id":"archive","position":1})).await;
    groups(&p, json!({"kind":"resume","id":"archive"})).await;
    let steps = run_all(&p, None).await;
    assert_eq!(outcomes(&steps, "done"), 3);
    assert_eq!(page(&p, "Archive").await["total"], 4);
}

#[tokio::test]
async fn account_removal_takes_the_group_fence_and_discards_only_reviewed_work() {
    let (_dir, p) = profile().await;
    seed(&p, 4).await;
    make_imap(&p).await;
    let provider = Scripted::new(vec![]);
    let gate = Arc::new(tokio::sync::Notify::new());
    *provider.gate.lock().unwrap() = Some(gate.clone());
    *p.operations.provider.lock().unwrap() = Some(provider.clone());
    review(&p, "archive", json!({"kind":"archive"})).await;
    review(&p, "later", json!({"kind":"read"})).await;
    approve(&p, "archive").await;
    let held = {
        let profile = crate::api::MobileProfile {
            database: p.database.clone(),
            operations: p.operations.clone(),
        };
        tokio::spawn(async move { step(&profile, Some("secret")).await })
    };
    provider.started.notified().await;
    let live = groups(&p, json!({"kind":"inspect","id":"archive"})).await;
    assert_eq!(live["state"], "running");
    assert_eq!(live["counts"]["sending"], 1);
    assert!(live["counts"]["uncertain"].is_null());
    assert_eq!(provider.moves.load(Ordering::SeqCst), 1);
    let preview = request(&p, json!({"op":"account_removal_preview","id":"fixture"})).await;
    assert_eq!(preview["groups"], 8);
    let blocked = failure(
        &p,
        json!({"op":"remove_account","review":preview,"discard_unresolved":true}),
    )
    .await;
    assert!(blocked.contains("step is in progress"), "{blocked}");
    gate.notify_one();
    assert_eq!(held.await.unwrap()["outcome"], "done");
    let preview = request(&p, json!({"op":"account_removal_preview","id":"fixture"})).await;
    assert_eq!(preview["groups"], 7);
    let refused = failure(
        &p,
        json!({"op":"remove_account","review":preview,"discard_unresolved":false}),
    )
    .await;
    assert!(refused.contains("group action"), "{refused}");
    request(
        &p,
        json!({"op":"remove_account","review":preview,"discard_unresolved":true}),
    )
    .await;
    let finished = job(&p, "archive").await;
    assert_eq!(finished["state"], "finished");
    assert_eq!(finished["counts"]["done"], 1);
    assert_eq!(finished["counts"]["cancelled"], 3);
    assert_eq!(
        items(&p, "archive", None).await["rows"][1]["reason"],
        "Account removed from this device"
    );
    assert!(job(&p, "later").await.is_null());
    let stopped = groups(&p, json!({"kind":"inspect","id":"later"})).await;
    assert_eq!(stopped["state"], "cancelled");
    assert!(
        group_failure(&p, json!({"kind":"approve","id":"later"}))
            .await
            .contains("no longer")
    );
    assert_eq!(step(&p, Some("secret")).await["idle"], true);
    assert_eq!(provider.moves.load(Ordering::SeqCst), 1);
    groups(&p, json!({"kind":"decline","id":"later"})).await;
    let removed = group_failure(&p, json!({"kind":"inspect","id":"later"})).await;
    assert!(!removed.is_empty());
    assert_eq!(
        groups(&p, json!({"kind":"inspect","id":"archive"})).await,
        finished
    );
}

#[tokio::test]
async fn inspection_does_not_create_missing_reviews_or_admit_invalid_identities() {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    let before = groups(&p, json!({"kind":"history"})).await;
    let invalid = group_failure(&p, json!({"kind":"inspect","id":"../other"})).await;
    assert!(invalid.contains("Invalid group identity"), "{invalid}");
    let missing = group_failure(&p, json!({"kind":"inspect","id":"missing"})).await;
    assert!(!missing.is_empty());
    assert_eq!(groups(&p, json!({"kind":"history"})).await, before);
    assert_eq!(step(&p, None).await["idle"], true);
    assert_eq!(page(&p, "Inbox").await["total"], 1);
}

#[tokio::test]
async fn history_pages_twenty_groups_retains_completed_receipts_and_bounds_active_reviews() {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    for n in 0..20 {
        let id = format!("job-{n:02}");
        review(&p, &id, json!({"kind":if n%2==0 {"unflag"} else {"flag"}})).await;
        approve(&p, &id).await;
        run_all(&p, None).await;
    }
    let history = groups(&p, json!({"kind":"history"})).await;
    assert_eq!(history["jobs"].as_array().unwrap().len(), 20);
    assert_eq!(history["jobs"][0]["id"], "job-19");
    // Admission never deletes an older completed receipt.
    review(&p, "job-20", json!({"kind":"unflag"})).await;
    let history = groups(&p, json!({"kind":"history"})).await;
    assert_eq!(history["jobs"].as_array().unwrap().len(), 20);
    let older = groups(
        &p,
        json!({"kind":"history","before":history["next_before"]}),
    )
    .await;
    assert_eq!(older["jobs"][0]["id"], "job-00");
    assert_eq!(older["jobs"][0]["counts"]["done"], 1);
    assert!(older["next_before"].is_null());
    assert_eq!(
        groups(
            &p,
            json!({"kind":"history","before":older["previous_before"]})
        )
        .await["jobs"][0]["id"],
        "job-20"
    );
    assert_eq!(history["jobs"][0]["id"], "job-20");
    groups(&p, json!({"kind":"decline","id":"job-20"})).await;
    assert!(job(&p, "job-20").await.is_null());
    let remaining: i64 = p
        .database
        .read(|db| {
            Ok(db.query_row(
                "SELECT COUNT(*) FROM group_items WHERE job='job-20'",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    for n in 0..20 {
        groups(&p, json!({"kind":"remove","id":format!("job-{n:02}")})).await;
    }
    assert!(
        groups(&p, json!({"kind":"history"})).await["jobs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Twenty open reviews block a further review until one is declined.
    for n in 0..20 {
        review(&p, &format!("open-{n:02}"), json!({"kind":"flag"})).await;
    }
    let captured = select_all(&p, "blocked-selection").await;
    let blocked = group_failure(&p,json!({"kind":"prepare","id":"blocked","selection":"blocked-selection","expected":captured["revision"],"action":{"kind":"flag"},"scope":{"folder":"Inbox"}})).await;
    assert!(blocked.contains("already 20 active"), "{blocked}");
    let running = failure(
        &p,
        json!({"op":"groups","command":{"kind":"remove","id":"open-00"}}),
    )
    .await;
    assert!(running.contains("finish or undo"), "{running}");
}

#[tokio::test]
async fn history_upgrade_preserves_receipts_and_exact_item_cursor_boundaries() {
    let (dir, p) = profile().await;
    seed(&p, 100).await;
    review(&p, "kept", json!({"kind":"archive"})).await;
    approve(&p, "kept").await;
    run_all(&p, None).await;
    let first = items(&p, "kept", None).await;
    assert_eq!(first["rows"].as_array().expect("page").len(), 50);
    assert_eq!(first["has_previous"], false);
    let second = items(&p, "kept", first["next_after"].as_i64()).await;
    assert_eq!(second["rows"].as_array().expect("page").len(), 50);
    assert!(second["next_after"].is_null());
    assert_eq!(second["has_previous"], true);
    assert_eq!(
        items(&p, "kept", second["previous_after"].as_i64()).await,
        first
    );
    p.database.write(|db| {
        db.execute_batch("DROP INDEX group_history_cursor; DROP INDEX group_item_attention; PRAGMA user_version=24")?;
        Ok(())
    }).await.expect("legacy profile");
    drop(p);
    let path = dir.path().join("mail.sqlite3");
    let reopened = crate::api::MobileProfile::open(path.to_string_lossy().to_string())
        .await
        .expect("upgrade");
    let kept = items(&reopened, "kept", None).await;
    assert_eq!(kept["rows"][0]["receipt"], first["rows"][0]["receipt"]);
    let version = reopened
        .database
        .read(|db| Ok(db.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?))
        .await
        .expect("version");
    assert_eq!(version, 25);
}

#[tokio::test]
async fn completed_history_does_not_delay_active_seeks_or_hide_older_attention() {
    use rusqlite::StatementStatus;
    let (_dir, p) = profile().await;
    p.database.write(|db| {
        db.execute_batch("BEGIN; WITH RECURSIVE n(v) AS (VALUES(1) UNION ALL SELECT v+1 FROM n WHERE v<100000) INSERT INTO group_jobs(id,action,fields,state,scope,created,total) SELECT 'finished-'||v,'{\"kind\":\"flag\"}','{}','finished','{}',0,1 FROM n; INSERT INTO group_items(job,position,mail,account,folder,remote_id,unread,starred,state,receipt) SELECT id,0,'fixture','fixture','INBOX','1',0,0,'done','{\"proof\":true}' FROM group_jobs; UPDATE group_items SET state='uncertain' WHERE job='finished-1'; INSERT INTO group_jobs(id,action,fields,state,scope,created,total) VALUES('ready','{\"kind\":\"flag\"}','{}','running','{}',0,1); INSERT INTO group_items(job,position,mail,account,folder,remote_id,unread,starred,state) VALUES('ready',0,'fixture','fixture','INBOX','1',0,0,'pending'); COMMIT")?;
        let mut next=db.prepare(crate::groups::NEXT_ITEM_QUERY)?;
        assert_eq!(next.query_row([],|r|r.get::<_,String>(0))?,"ready");
        assert!(next.get_status(StatementStatus::VmStep)<1000);
        assert!(next.get_status(StatementStatus::FullscanStep)<30);
        println!("next-item: VM={}, fullscan={}",next.get_status(StatementStatus::VmStep),next.get_status(StatementStatus::FullscanStep));
        let plans=db.prepare(&format!("EXPLAIN QUERY PLAN {}",crate::groups::NEXT_ITEM_QUERY))?.query_map([],|r|r.get::<_,String>(3))?.collect::<rusqlite::Result<Vec<_>>>()?;
        println!("next-item plan: {plans:?}");
        let queries=[
            crate::groups::ACTIVE_CAPACITY_QUERY,
            crate::groups::ATTENTION_COUNT_QUERY,
        ];
        for sql in queries {
            let mut statement=db.prepare(sql)?;
            assert_eq!(statement.query_row([],|r|r.get::<_,i64>(0))?,1);
            assert!(statement.get_status(StatementStatus::VmStep)<1000);
            assert!(statement.get_status(StatementStatus::FullscanStep)<30);
            println!("{sql}: VM={}, fullscan={}",statement.get_status(StatementStatus::VmStep),statement.get_status(StatementStatus::FullscanStep));
        }
        let mut attention=db.prepare(crate::groups::ATTENTION_TARGET_QUERY)?;
        assert_eq!(attention.query_row([],|r|r.get::<_,String>(0))?,"finished-1");
        assert!(attention.get_status(StatementStatus::VmStep)<1000);
        println!("attention-target: VM={}, fullscan={}",attention.get_status(StatementStatus::VmStep),attention.get_status(StatementStatus::FullscanStep));
        Ok(())
    }).await.expect("bounded seeks");
    let observed = groups(&p, json!({"kind":"history"})).await;
    assert_eq!(observed["jobs"].as_array().expect("page").len(), 20);
    assert_eq!(observed["active"][0]["id"], "ready");
    assert_eq!(observed["attention"], 1);
    assert_eq!(observed["attention_job"]["id"], "finished-1");
    let count = p
        .database
        .read(|db| {
            Ok(db.query_row("SELECT COUNT(*) FROM group_jobs", [], |r| {
                r.get::<_, i64>(0)
            })?)
        })
        .await
        .expect("retained history");
    assert_eq!(count, 100001);
}

#[tokio::test]
async fn an_empty_cursor_page_keeps_its_surviving_boundary_reachable() {
    let (_dir, p) = profile().await;
    seed(&p, 50).await;
    review(&p, "boundary", json!({"kind":"archive"})).await;
    approve(&p, "boundary").await;
    run_all(&p, None).await;
    let latest = groups(&p, json!({"kind":"history"})).await;
    let empty = groups(
        &p,
        json!({"kind":"history","before":latest["jobs"][0]["seq"]}),
    )
    .await;
    assert!(empty["jobs"].as_array().expect("empty page").is_empty());
    assert_eq!(empty["has_previous"], true);
    assert_eq!(
        groups(
            &p,
            json!({"kind":"history","before":empty["previous_before"]})
        )
        .await["jobs"][0]["id"],
        "boundary"
    );
    let empty_items = items(&p, "boundary", Some(49)).await;
    assert!(
        empty_items["rows"]
            .as_array()
            .expect("empty details")
            .is_empty()
    );
    assert_eq!(empty_items["has_previous"], true);
    assert_eq!(
        items(&p, "boundary", empty_items["previous_after"].as_i64()).await["rows"][49]["position"],
        49
    );
}
