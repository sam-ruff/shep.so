use crate::tests::{profile, request, seed};
use serde_json::{Value, json};

async fn selection(p: &crate::api::MobileProfile, command: Value, observed: &[String]) -> Value {
    request(
        p,
        json!({"op":"selection","command":command,"observed":observed}),
    )
    .await
}
fn capture(id: &str, revision: u64, all: bool) -> Value {
    json!({"kind":"capture","id":id,"revision":revision,"all":all,"scope":{"folder":"Inbox"}})
}
fn change(id: &str, expected: u64, action: Value) -> Value {
    json!({"kind":"change","id":id,"expected":expected,"change":action,"scope":{"folder":"Inbox"}})
}
async fn error(p: &crate::api::MobileProfile, command: Value) {
    let result: Value = serde_json::from_str(
        &p.request(json!({"op":"selection","command":command}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(result["error"].is_string(), "{result}");
}

#[tokio::test]
async fn selection_captures_all_pages_and_explicit_arrivals_without_passive_growth() {
    let (_dir, p) = profile().await;
    seed(&p, 125).await;
    let page = request(&p, json!({"op":"page","folder":"Inbox"})).await;
    let observed: Vec<String> = page["mail"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_owned())
        .collect();
    let first = selection(&p, capture("selection", 0, true), &observed).await;
    assert_eq!(first["selected"], 125);
    assert_eq!(first["available"], 125);
    assert_eq!(first["visible"].as_array().unwrap().len(), 50);
    assert_eq!(first["positions"][&observed[0]], 0);
    assert_eq!(first["groups"][0]["account"], "fixture");
    assert_eq!(first["groups"][0]["unread"], 63);
    seed(&p, 126).await;
    let latest = request(&p, json!({"op":"page","folder":"Inbox"})).await;
    let arrival = latest["mail"][0]["id"].as_str().unwrap().to_owned();
    let passive = selection(
        &p,
        json!({"kind":"observe","id":"selection"}),
        std::slice::from_ref(&arrival),
    )
    .await;
    assert_eq!(passive["selected"], 125);
    assert_eq!(passive["total"], 125);
    assert!(passive["visible"].as_array().unwrap().is_empty());
    let explicit = selection(
        &p,
        change(
            "selection",
            0,
            json!({"kind":"set","id":arrival,"selected":true,"clear_others":false}),
        ),
        std::slice::from_ref(&arrival),
    )
    .await;
    assert_eq!(explicit["selected"], 126);
    assert_eq!(explicit["total"], 126);
    assert_eq!(explicit["visible"], json!([arrival]));
    error(&p, change("selection", 0, json!({"kind":"clear"}))).await;
    let unchanged = selection(&p, json!({"kind":"observe","id":"selection"}), &[]).await;
    assert_eq!(unchanged["selected"], 126);
    let recaptured = selection(
        &p,
        capture("selection", 2, true),
        std::slice::from_ref(&arrival),
    )
    .await;
    assert_eq!(recaptured["positions"][&arrival], 0);
    assert_eq!(recaptured["selected"], 126);
}

#[tokio::test]
async fn selection_ranges_freeze_revision_and_bounded_metadata_survive_navigation() {
    let (_dir, p) = profile().await;
    seed(&p, 125).await;
    let a = request(&p, json!({"op":"page","folder":"Inbox"})).await["mail"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let b = request(&p, json!({"op":"page","folder":"Inbox","offset":100})).await["mail"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    selection(&p, capture("selection", 0, false), &[a.clone(), b.clone()]).await;
    let range = selection(
        &p,
        change(
            "selection",
            0,
            json!({"kind":"range","anchor":a,"target":b,"additive":false}),
        ),
        &[a.clone(), b.clone()],
    )
    .await;
    assert_eq!(range["selected"], 101);
    assert_eq!(range["positions"][&b], 100);
    let frozen = selection(
        &p,
        json!({"kind":"freeze","id":"selection","expected":1,"target":"review"}),
        &[],
    )
    .await;
    assert_eq!(frozen["frozen"], true);
    assert_eq!(frozen["selected"], 101);
    selection(&p, change("selection", 1, json!({"kind":"clear"})), &[]).await;
    error(&p, change("review", 0, json!({"kind":"clear"}))).await;
    let mut after = Value::Null;
    let mut count = 0;
    loop {
        let page = selection(
            &p,
            json!({"kind":"page","id":"review","expected":0,"after":after}),
            &[],
        )
        .await;
        let rows = page["rows"].as_array().unwrap();
        assert!(rows.len() <= 50);
        for row in rows {
            assert!(row.get("body").is_none());
            assert!(row.get("raw").is_none());
        }
        count += rows.len();
        after = page["next_after"].clone();
        if after.is_null() {
            break;
        }
    }
    assert_eq!(count, 101);
    selection(&p, json!({"kind":"release","id":"review"}), &[]).await;
    selection(&p, json!({"kind":"release","id":"review"}), &[]).await;
    error(&p, json!({"kind":"observe","id":"review"})).await;
}

#[tokio::test]
async fn selection_uses_projected_query_membership_and_recapture_is_atomic() {
    let (_dir, p) = profile().await;
    seed(&p, 125).await;
    let first = request(&p, json!({"op":"page","folder":"Inbox"})).await;
    let id = first["mail"][0]["id"].as_str().unwrap().to_owned();
    let scope = json!({"folder":"Archive","filter":"Flagged","query":"needle 124","projection":{id.clone():{"folder":"Archive","starred":true,"unread":false}}});
    let cap = selection(
        &p,
        json!({"kind":"capture","id":"projected","revision":0,"all":true,"scope":scope}),
        std::slice::from_ref(&id),
    )
    .await;
    assert_eq!(cap["selected"], 1);
    assert_eq!(cap["visible"], json!([id]));
    let cache = request(&p, json!({"op":"page","folder":"Archive"})).await;
    assert_eq!(cache["total"], 0); // A capture does not persist optimistic mail.
    error(&p, change("projected", 0, json!({"kind":"clear"}))).await;
    p.database.selection(|db| {db.execute_batch("CREATE TEMP TRIGGER reject_capture BEFORE INSERT ON selection_rows BEGIN SELECT RAISE(ABORT,'fixture capture failure');END;")?;Ok(())}).await.unwrap();
    error(
        &p,
        json!({"kind":"capture","id":"projected","revision":1,"all":false,"scope":scope}),
    )
    .await;
    let saved = selection(&p, json!({"kind":"observe","id":"projected"}), &[]).await;
    assert_eq!(saved["selected"], 1);
    assert_eq!(saved["revision"], 0);
}

#[tokio::test]
async fn selection_reconciles_aliases_and_retains_missing_members_without_persistence() {
    let (dir, p) = profile().await;
    seed(&p, 3).await;
    let first = request(&p, json!({"op":"page","folder":"Inbox"})).await;
    let old = first["mail"][0]["id"].as_str().unwrap().to_owned();
    selection(
        &p,
        capture("selection", 0, true),
        std::slice::from_ref(&old),
    )
    .await;
    let original = old.clone();
    p.database
        .write(move |db| {
            db.execute("UPDATE mail SET id='adopted-id' WHERE id=?", [&original])?;
            db.execute(
                "INSERT INTO mail_aliases VALUES(?,'adopted-id')",
                [original],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let adopted = selection(
        &p,
        json!({"kind":"observe","id":"selection"}),
        std::slice::from_ref(&old),
    )
    .await;
    assert_eq!(adopted["selected"], 3);
    assert_eq!(adopted["visible"], json!(["adopted-id"]));
    assert_eq!(adopted["aliases"][&old], "adopted-id");
    p.database
        .write(|db| {
            db.execute("DELETE FROM mail WHERE id='adopted-id'", [])?;
            Ok(())
        })
        .await
        .unwrap();
    let missing = selection(&p, json!({"kind":"observe","id":"selection"}), &[]).await;
    assert_eq!(missing["selected"], 3);
    assert_eq!(missing["available"], 2);
    assert_eq!(
        p.database
            .read(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name LIKE 'selection_%'",
                [],
                |r| r.get::<_, i64>(0)
            )?))
            .await
            .unwrap(),
        0
    );
    drop(p);
    let reopened = crate::api::MobileProfile::open(
        dir.path().join("mail.sqlite3").to_str().unwrap().to_owned(),
    )
    .await
    .unwrap();
    error(&reopened, json!({"kind":"observe","id":"selection"})).await;
}

#[tokio::test]
async fn held_selection_and_provider_capacity_do_not_occupy_cached_reads_or_writes() {
    let (_dir, p) = profile().await;
    seed(&p, 125).await;
    let _network = p.operations.hold_network_capacity().await;
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let db = p.database.clone();
    let held = tokio::spawn(async move {
        db.selection(move |_| {
            entered.send(()).unwrap();
            blocked.recv().unwrap();
            Ok(())
        })
        .await
        .unwrap()
    });
    ready.await.unwrap();
    let cached = request(&p, json!({"op":"page","folder":"Inbox","offset":50})).await;
    assert_eq!(cached["mail"].as_array().unwrap().len(), 50);
    request(&p,json!({"op":"save_draft","draft":crate::tests::draft(1,"Selection does not block saving.")})).await;
    let drafts = request(&p, json!({"op":"drafts"})).await;
    assert_eq!(drafts[0]["body"], "Selection does not block saving.");
    release.send(()).unwrap();
    held.await.unwrap();
}

#[tokio::test]
async fn selection_of_100000_messages_returns_only_observed_and_review_pages() {
    let (_dir, p) = profile().await;
    seed(&p, 1).await;
    // Synthetic metadata only: this is a cardinality/contract test, not a
    // MIME or performance claim. No full ID collection is built by the caller.
    p.database.write(|db| {
        db.execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<99999)
          INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw)
          SELECT 'large-'||x,'fixture','large-'||x,'INBOX','Fixture','reader@example.test','Selection fixture','',x,1,0,0,'',X'' FROM n;")?;
        Ok(())
    }).await.unwrap();
    let last = request(&p, json!({"op":"page","folder":"Inbox","offset":99950})).await;
    let observed: Vec<String> = last["mail"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_owned())
        .collect();
    let captured = selection(&p, capture("large", 0, true), &observed).await;
    assert_eq!(captured["total"], 100000);
    assert_eq!(captured["selected"], 100000);
    assert_eq!(captured["visible"].as_array().unwrap().len(), 50);
    assert_eq!(captured["positions"][&observed[0]], 99950);
    assert!(serde_json::to_vec(&captured).unwrap().len() < 8192);
    p.database
        .write(|db| {
            let tx = db.transaction()?;
            for index in 1..=125 {
                let old = format!("large-{index}");
                let new = format!("adopted-{index}");
                tx.execute(
                    "UPDATE mail SET id=?1 WHERE id=?2",
                    rusqlite::params![new, old],
                )?;
                tx.execute(
                    "INSERT INTO mail_aliases VALUES(?1,?2)",
                    rusqlite::params![old, new],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    let aliased = selection(
        &p,
        json!({"kind":"observe","id":"large"}),
        &["large-1".into(), "large-75".into(), "large-125".into()],
    )
    .await;
    assert_eq!(aliased["selected"], 100000);
    assert_eq!(aliased["available"], 100000);
    assert_eq!(
        aliased["visible"],
        json!(["adopted-1", "adopted-125", "adopted-75"])
    );
    let frozen = selection(
        &p,
        json!({"kind":"freeze","id":"large","expected":0,"target":"review-large"}),
        &[],
    )
    .await;
    assert_eq!(frozen["selected"], 100000);
    let page = selection(
        &p,
        json!({"kind":"page","id":"review-large","expected":0,"after":99949}),
        &[],
    )
    .await;
    assert_eq!(page["rows"].as_array().unwrap().len(), 50);
    assert_eq!(page["rows"][49]["position"], 99999);
    let too_many = serde_json::from_str::<Value>(&p.request(json!({"op":"selection","command":{"kind":"observe","id":"large"},"observed":vec!["id";51]}).to_string()).await.unwrap()).unwrap();
    assert!(too_many["error"].is_string());
}

#[tokio::test]
async fn selection_queue_is_bounded_fifo_and_cancelled_call_retains_owned_work() {
    let (_dir, p) = profile().await;
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let database = p.database.clone();
    let active = tokio::spawn(async move {
        database
            .selection(move |db| {
                db.execute_batch("CREATE TEMP TABLE selection_order_fixture(position INTEGER)")?;
                entered.send(()).unwrap();
                blocked.recv().unwrap();
                db.execute("INSERT INTO selection_order_fixture VALUES(0)", [])?;
                Ok(())
            })
            .await
    });
    ready.await.unwrap();
    let mut pending = Vec::new();
    for position in 1..32 {
        let database = p.database.clone();
        pending.push(tokio::spawn(async move {
            database
                .selection(move |db| {
                    let previous: i64 =
                        db.query_row("SELECT COUNT(*) FROM selection_order_fixture", [], |r| {
                            r.get(0)
                        })?;
                    assert_eq!(previous, position);
                    db.execute("INSERT INTO selection_order_fixture VALUES(?)", [position])?;
                    Ok(())
                })
                .await
                .unwrap();
        }));
        while p.database.pending_selections() != position as usize + 1 {
            tokio::task::yield_now().await;
        }
    }
    let full = p.database.selection(|_| Ok(())).await.unwrap_err();
    assert!(full.to_string().contains("Selection is catching up"));
    active.abort();
    assert!(active.await.unwrap_err().is_cancelled());
    assert_eq!(p.database.pending_selections(), 32);
    // Cancelling the caller must not release the FIFO before its SQLite work.
    release.send(()).unwrap();
    for task in pending {
        task.await.unwrap();
    }
    assert_eq!(p.database.pending_selections(), 0);
}
