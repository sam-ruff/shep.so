use super::*;
use crate::{model::*, store::Store};
use serde_json::json;
use shep_profile_core::{
    drive::catalog::Scope,
    history::{Command as HistoryCommand, Reply, Worker},
};

fn scope() -> Scope {
    Scope {
        namespace: "so.shep.fixture".into(),
        principal: "drive:fixture".into(),
    }
}
async fn store(count: usize) -> Store {
    let store = Store::memory().unwrap();
    let accounts: Vec<Account> = (0..count)
        .map(|n| {
            serde_json::from_value(json!({
                "id": if n==0 { Uuid::from_u128(999).to_string() } else { format!("local-{n}") },
                "name":format!("Account {n}"),"email":"same@example.test","protocol":"Imap",
                "host":format!("imap-{n}.example.test"),"port":993,"username":format!("user-{n}"),
                "smtp_host":"smtp.example.test","smtp_port":465
            }))
            .unwrap()
        })
        .collect();
    store.put("accounts", accounts).await.unwrap();
    store
        .save_preferences(Preferences {
            appearance: Appearance::Light,
            ..Default::default()
        })
        .await
        .unwrap();
    store
}
fn spec() -> Specification {
    Specification {
        name: "My setup".into(),
        include_accounts: true,
        settings: BTreeMap::from([
            (SettingKey::Appearance, json!("Light")),
            (SettingKey::Tooltips, json!(true)),
        ]),
    }
}
async fn reviewed(store: &Store) -> Review {
    let mut review = store
        .publication_prepare(scope(), Uuid::new_v4(), spec())
        .await
        .unwrap();
    while review.phase == Phase::Preparing {
        review = store
            .publication_prepare_step(scope().storage_key().unwrap(), review.id)
            .await
            .unwrap();
    }
    review
}
#[tokio::test]
async fn frozen_publication_prepares_and_reviews_bounded_pages_without_changing_local_accounts() {
    let store = store(75).await;
    let original: Vec<Account> = store.get("accounts").await.unwrap();
    let key = scope().storage_key().unwrap();
    let id = Uuid::new_v4();
    let created = store
        .publication_prepare(scope(), id, spec())
        .await
        .unwrap();
    assert_eq!(
        (created.accounts, created.prepared, created.total),
        (75, 0, 78)
    );
    assert_eq!(
        created,
        store
            .publication_prepare(scope(), id, spec())
            .await
            .unwrap()
    );
    let first = store
        .publication_prepare_step(key.clone(), id)
        .await
        .unwrap();
    assert_eq!((first.prepared, first.phase), (50, Phase::Preparing));
    assert!(
        store
            .publication_approve(key.clone(), id, spec().settings)
            .await
            .is_err()
    );
    let ready = store
        .publication_prepare_step(key.clone(), id)
        .await
        .unwrap();
    assert_eq!((ready.prepared, ready.phase), (75, Phase::Review));
    let page = store
        .publication_accounts(key.clone(), id, 0)
        .await
        .unwrap();
    assert_eq!(page.len(), 50);
    let next = store
        .publication_accounts(key.clone(), id, page.last().unwrap().position)
        .await
        .unwrap();
    assert_eq!(next.len(), 25);
    assert_eq!(page[0].account, original[0]);
    assert_eq!(next.last().unwrap().account, original[74]);
    let accepted = store
        .publication_approve(key.clone(), id, spec().settings)
        .await
        .unwrap();
    assert_eq!(accepted.phase, Phase::Staging);
    assert_eq!(
        accepted,
        store
            .publication_approve(key, id, spec().settings)
            .await
            .unwrap()
    );
    let (mapped, first_shared) = store
        .run(move |db| {
            Ok((
                db.query_row("SELECT count(*) FROM profile_account_mappings", [], |r| {
                    r.get::<_, i64>(0)
                })?,
                db.query_row(
                    "SELECT shared_id FROM profile_account_mappings WHERE local_id=?",
                    [Uuid::from_u128(999).to_string()],
                    |r| r.get::<_, String>(0),
                )?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(mapped, 75);
    assert_eq!(first_shared, Uuid::from_u128(999).to_string());
    assert_eq!(
        store.get::<Vec<Account>>("accounts").await.unwrap(),
        original
    );
    assert!(
        store
            .publication_cancel(scope().storage_key().unwrap(), id)
            .await
            .is_err()
    );
}
#[tokio::test]
async fn changed_accounts_or_preferences_cannot_approve_old_review_and_other_scopes_cannot_read_it()
{
    for accounts in [false, true] {
        let store = store(2).await;
        let review = reviewed(&store).await;
        let key = scope().storage_key().unwrap();
        let mut foreign = scope();
        foreign.principal = "drive:other-fixture".into();
        assert!(
            store
                .publication_review(foreign.storage_key().unwrap(), review.id)
                .await
                .is_err()
        );
        if accounts {
            let mut changed: Vec<Account> = store.get("accounts").await.unwrap();
            changed.remove(0);
            store.put("accounts", changed).await.unwrap();
        } else {
            let mut changed: Preferences = store.get("preferences").await.unwrap();
            changed.appearance = Appearance::Dark;
            store.save_preferences(changed).await.unwrap();
        }
        assert!(
            store
                .publication_approve(key.clone(), review.id, spec().settings)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .publication_review(key.clone(), review.id)
                .await
                .unwrap()
                .phase,
            Phase::Review
        );
        assert_eq!(
            store
                .publication_cancel(key.clone(), review.id)
                .await
                .unwrap()
                .phase,
            Phase::Cancelled
        );
        assert!(
            store
                .publication_approve(key, review.id, spec().settings)
                .await
                .is_err()
        );
    }
}
#[tokio::test]
async fn lost_staging_receipt_retries_exact_saved_request_and_reopens_original_history() {
    let store = store(2).await;
    let review = reviewed(&store).await;
    let key = scope().storage_key().unwrap();
    let review = store
        .publication_approve(key.clone(), review.id, spec().settings)
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.sqlite");
    let worker = Worker::open(path.clone(), review.binding.clone())
        .await
        .unwrap();
    store.run(|db|{
        db.execute_batch("CREATE TEMP TRIGGER fail_profile_receipt BEFORE UPDATE OF review ON profile_publications
            WHEN json_extract(NEW.review,'$.staged')>json_extract(OLD.review,'$.staged')
            BEGIN SELECT RAISE(ABORT,'synthetic lost receipt'); END;")?;
        Ok(())
    }).await.unwrap();
    assert!(stage(&store, &key, &review, &worker).await.is_err());
    let Reply::State(state) = worker.request(HistoryCommand::State).await.unwrap() else {
        panic!()
    };
    assert_eq!(state.operations, 1);
    assert_eq!(
        store
            .publication_review(key.clone(), review.id)
            .await
            .unwrap()
            .staged,
        0
    );
    worker.close().await.unwrap();
    store
        .run(|db| {
            db.execute_batch("DROP TRIGGER fail_profile_receipt")?;
            Ok(())
        })
        .await
        .unwrap();
    let worker = Worker::open(path, review.binding.clone()).await.unwrap();
    let mut current = stage(&store, &key, &review, &worker).await.unwrap();
    assert_eq!(current.staged, 1);
    while current.phase == Phase::Staging {
        current = stage(&store, &key, &current, &worker).await.unwrap();
    }
    let Reply::State(state) = worker.request(HistoryCommand::State).await.unwrap() else {
        panic!()
    };
    assert_eq!(state.operations, 5);
    assert_eq!(state.queued, 5);
    assert!(state.initialized);
    worker.close().await.unwrap();
}

#[cfg(feature = "test-support")]
#[tokio::test]
async fn approved_publication_reopens_after_lost_drive_receipt_without_duplicate_post() {
    use crate::profiles::{
        discovery::{Action as DiscoveryAction, Grant, Session},
        fixture::Fixture,
    };
    let fixture = Fixture::start(0, true, std::time::Duration::ZERO)
        .await
        .unwrap();
    let drive = fixture
        .connect(scope().namespace, &scope().principal)
        .await
        .unwrap();
    let mut prefs = Preferences {
        appearance: Appearance::Light,
        ..Default::default()
    };
    prefs.google_grant.id = "fixture-grant".into();
    prefs.google_grant.access.known = true;
    prefs.google_grant.access.drive = true;
    prefs.google_connection_id = scope().principal;
    let grant = Grant::from_preferences(&prefs);
    let path = fixture.root.path().join("mail.sqlite");
    let source = store(2).await;
    let accounts: Vec<Account> = source.get("accounts").await.unwrap();
    let mut store = Store::open(&path).unwrap();
    store.put("accounts", accounts.clone()).await.unwrap();
    store.save_preferences(prefs).await.unwrap();
    let root = fixture.root.path().join("catalogs");
    let mut session = Session::open(root.clone(), Uuid::new_v4(), grant.clone(), &drive)
        .await
        .unwrap();
    // Incomplete discovery cannot prepare even an empty profile.
    let id = Uuid::new_v4();
    let denied = session
        .run_publication(
            &store,
            Command::Prepare {
                id,
                specification: spec(),
            },
            None,
        )
        .await
        .unwrap();
    assert!(denied.error.is_some());
    assert!(denied.publication.review.is_none());
    for _ in 0..30 {
        let observed = session
            .run(DiscoveryAction::Advance, Some(&drive))
            .await
            .unwrap();
        let state = observed.state.unwrap();
        if observed.error.is_some() {
            session
                .run(
                    DiscoveryAction::Retry {
                        revision: state.revision,
                    },
                    None,
                )
                .await
                .unwrap();
        } else if state.phase == shep_profile_core::drive::catalog::Phase::Complete {
            break;
        }
    }
    let prepared = session
        .run_publication(
            &store,
            Command::Prepare {
                id,
                specification: spec(),
            },
            None,
        )
        .await
        .unwrap();
    assert!(prepared.error.is_none());
    session
        .run_publication(&store, Command::PrepareStep { id }, None)
        .await
        .unwrap();
    session
        .run_publication(
            &store,
            Command::Approve {
                id,
                settings: spec().settings,
            },
            None,
        )
        .await
        .unwrap();
    loop {
        let observed = session
            .run_publication(&store, Command::Step { id }, None)
            .await
            .unwrap();
        assert!(observed.error.is_none());
        if observed.publication.review.unwrap().phase == Phase::Uploading {
            break;
        }
    }
    let failed = session
        .run_publication(&store, Command::Step { id }, Some(&drive))
        .await
        .unwrap();
    assert!(failed.error.is_some());
    assert_eq!(failed.publication.review.as_ref().unwrap().uploaded, 0);
    assert_eq!(fixture.attempts.lock().unwrap().len(), 1);
    session.close().await.unwrap();
    drop(store);
    store = Store::open(&path).unwrap();
    session = Session::open(root.clone(), Uuid::new_v4(), grant.clone(), &drive)
        .await
        .unwrap();
    let saved = session
        .run_publication(&store, Command::Current, None)
        .await
        .unwrap();
    assert_eq!(saved.publication.review.unwrap().id, id);
    for _ in 0..10 {
        let observed = session
            .run_publication(&store, Command::Step { id }, Some(&drive))
            .await
            .unwrap();
        assert!(observed.error.is_none(), "{:?}", observed.error);
        if observed.publication.review.unwrap().phase == Phase::Complete {
            break;
        }
    }
    let complete = store
        .publication_current(scope().storage_key().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((complete.phase, complete.uploaded), (Phase::Complete, 5));
    let attempts = fixture.attempts.lock().unwrap().clone();
    assert_eq!(attempts.len(), 5);
    assert_eq!(
        attempts
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        5
    );
    assert_eq!(
        store.get::<Vec<Account>>("accounts").await.unwrap(),
        accounts
    );
    session.close().await.unwrap();
    // An independent catalog reads original wire records, including the causal
    // completion barrier. It does not inherit the publisher's device identity.
    let mut reader = Session::open(
        fixture.root.path().join("other-device"),
        Uuid::new_v4(),
        grant,
        &drive,
    )
    .await
    .unwrap();
    for _ in 0..80 {
        let observed = reader
            .run(DiscoveryAction::Advance, Some(&drive))
            .await
            .unwrap();
        assert!(observed.error.is_none(), "{:?}", observed.error);
        if observed.state.unwrap().phase == shep_profile_core::drive::catalog::Phase::Complete {
            break;
        }
    }
    let observed = reader.observe(None).await.unwrap();
    assert_eq!(observed.rows.len(), 1);
    let profile = &observed.rows[0];
    assert_eq!((profile.accounts, profile.settings), (2, 2));
    assert_eq!(profile.name.as_deref(), Some("My setup"));
    assert!(profile.initialized);
    reader.close().await.unwrap();
}
