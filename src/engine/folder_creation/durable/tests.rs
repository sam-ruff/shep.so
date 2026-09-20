use super::*;

async fn fixture() -> (Store, CreationJob) {
    let store = Store::memory().expect("store");
    let account:Account=serde_json::from_value(serde_json::json!({"id":"folders","name":"Folders","email":"folders@example.test","protocol":"Imap","host":"example.test","port":993,"username":"folders","smtp_host":"example.test","smtp_port":465})).expect("account");
    store.save_account(account.clone()).await.expect("account");
    let job = store
        .admit_folder_creation(
            uuid::Uuid::new_v4().to_string(),
            account.id.clone(),
            crate::mail_actions::connection_key(&account),
            None,
            "Receipts".into(),
        )
        .await
        .expect("admit");
    (store, job)
}
fn target() -> Mailbox {
    Mailbox {
        delimiter: Some('.'),
        ..Mailbox::flat("INBOX.Receipts".into())
    }
}
fn api() -> MockCreationApi {
    let mut api = MockCreationApi::new();
    api.expect_plan().times(1).returning(|_, _| Ok(target()));
    api.expect_inspect().times(1).returning(|_| Ok(None));
    api
}

#[tokio::test]
async fn durable_running_and_receipt_progress_arrive_before_completion() {
    use futures::StreamExt;
    let (store, job) = fixture().await;
    let mut provider = api();
    provider
        .expect_create()
        .times(1)
        .returning(|_| Ok(CreateOutcome::Acknowledged));
    provider
        .expect_catalog()
        .times(1)
        .returning(|| Ok(vec![target()]));
    let (mut output, mut events) = futures::channel::mpsc::channel(4);
    let saved = execute_observed(
        &store,
        job,
        &provider,
        &Default::default(),
        Some(&mut output),
    )
    .await
    .expect("complete");
    assert_eq!(saved.stage, CreationStage::Succeeded);
    let Some(Event::CreationChanged(running)) = events.next().await else {
        panic!("running progress")
    };
    assert_eq!(running.stage, CreationStage::Running);
    assert!(running.receipt.is_none());
    let Some(Event::CreationChanged(repair)) = events.next().await else {
        panic!("receipt progress")
    };
    assert_eq!(repair.stage, CreationStage::Repair);
    assert!(repair.provider_acknowledged);
    assert!(repair.revision > running.revision);
}

#[tokio::test]
async fn cancelled_identity_remains_idempotent_after_a_new_same_name_request() {
    let (store, original) = fixture().await;
    let cancelled = store
        .decide_creation(original.id.clone(), original.revision, true)
        .await
        .expect("cancel");
    let next = store
        .admit_folder_creation(
            uuid::Uuid::new_v4().to_string(),
            original.account.clone(),
            original.connection.clone(),
            original.parent.clone(),
            original.name.clone(),
        )
        .await
        .expect("new request");
    let replay = store
        .admit_folder_creation(
            original.id,
            original.account,
            original.connection,
            original.parent,
            original.name,
        )
        .await
        .expect("old acknowledgement");
    assert_eq!(replay, cancelled);
    assert_eq!(
        store
            .creation_job(next.id)
            .await
            .expect("new still queued")
            .stage,
        CreationStage::Queued
    );
}

#[tokio::test]
async fn lost_durable_receipt_after_create_requires_inspection_after_restart() {
    let (store, job) = fixture().await;
    store.run(|c|{c.execute_batch("CREATE TRIGGER fail_receipt BEFORE UPDATE OF data ON folder_creations WHEN json_extract(NEW.data,'$.stage')='repair' BEGIN SELECT RAISE(ABORT,'receipt unavailable'); END;")?;Ok(())}).await.expect("receipt failure");
    let mut first = api();
    first
        .expect_create()
        .times(1)
        .returning(|_| Ok(CreateOutcome::Acknowledged));
    first.expect_catalog().never();
    assert!(
        execute(&store, job.clone(), &first, &Default::default())
            .await
            .is_err()
    );
    assert_eq!(
        store
            .creation_job(job.id.clone())
            .await
            .expect("dispatch retained")
            .stage,
        CreationStage::Running
    );
    store
        .run(|c| {
            c.execute_batch("DROP TRIGGER fail_receipt")?;
            Ok(())
        })
        .await
        .expect("storage restored");
    store.recover_creations().await.expect("restart");
    let unknown = store.creation_job(job.id).await.expect("unknown");
    assert_eq!(unknown.stage, CreationStage::Uncertain);
    let checking = store
        .decide_creation(unknown.id, unknown.revision, false)
        .await
        .expect("explicit check");
    let mut check = MockCreationApi::new();
    check.expect_create().never();
    check
        .expect_inspect()
        .times(1)
        .returning(|_| Ok(Some(target())));
    check
        .expect_catalog()
        .times(1)
        .returning(|| Ok(vec![target()]));
    assert_eq!(
        execute(&store, checking, &check, &Default::default())
            .await
            .expect("verified")
            .stage,
        CreationStage::Succeeded
    );
}

#[tokio::test]
async fn local_admission_failure_rolls_back_and_removal_review_tracks_saved_requests() {
    let (store, job) = fixture().await;
    let target = crate::store::ConnectionRef {
        kind: crate::store::ConnectionKind::Account,
        id: job.account.clone(),
    };
    let before = store.removal_preview(target.clone()).await.expect("review");
    assert_eq!(before.transfers, 1);
    let cancelled = store
        .decide_creation(job.id.clone(), job.revision, true)
        .await
        .expect("cancel");
    assert!(store.remove_connection(before, true).await.is_err());
    let current = store.removal_preview(target).await.expect("updated review");
    assert_eq!(current.transfers, 0);
    store.run(|c|{c.execute_batch("CREATE TRIGGER fail_creation BEFORE INSERT ON folder_creations BEGIN SELECT RAISE(ABORT,'disk full'); END;")?;Ok(())}).await.expect("fixture");
    let id = uuid::Uuid::new_v4().to_string();
    assert!(
        store
            .admit_folder_creation(
                id.clone(),
                job.account,
                job.connection,
                None,
                "Other".into()
            )
            .await
            .is_err()
    );
    assert!(store.creation_job(id).await.is_err());
    store
        .remove_connection(current, true)
        .await
        .expect("remove");
    assert!(store.creation_job(cancelled.id).await.is_err());
}

#[tokio::test]
async fn imported_unknown_target_can_stop_tracking_without_claiming_provider_success() {
    let (store, job) = fixture().await;
    store
        .run(|c| crate::store::folder_creation::fence_import(c, "Imported review"))
        .await
        .expect("fence");
    let imported = store.creation_job(job.id).await.expect("imported");
    assert!(imported.target.is_none());
    let dismissed = store
        .dismiss_creation(imported.id, imported.revision)
        .await
        .expect("stop tracking");
    assert_eq!(dismissed.stage, CreationStage::Dismissed);
    assert!(dismissed.receipt.is_none());
    assert!(
        dismissed
            .error
            .as_deref()
            .is_some_and(|e| e.contains("without confirming"))
    );
}

#[tokio::test]
async fn held_provider_capacity_leaves_durable_admission_queued_and_close_never_dispatches() {
    let (store, job) = fixture().await;
    let mut engine = crate::engine::calendar_tests::engine();
    engine.store = store.clone();
    engine.demo = true;
    let mut slots = Vec::new();
    for _ in 0..8 {
        slots.push(engine.provider_slots.acquire().await);
    }
    let worker = engine.clone();
    let id = job.id.clone();
    let (output, _events) = futures::channel::mpsc::channel(32);
    let mut pending = Box::pin(worker.execute_creation_work(id, output));
    assert!(futures::poll!(&mut pending).is_pending());
    assert_eq!(
        store
            .creation_job(job.id.clone())
            .await
            .expect("saved")
            .stage,
        CreationStage::Queued
    );
    engine.bulk_control.stopping.set(true);
    assert!(
        !tokio::time::timeout(Duration::from_secs(1), pending)
            .await
            .expect("close does not wait for capacity")
    );
    assert_eq!(
        store
            .creation_job(job.id)
            .await
            .expect("still queued")
            .stage,
        CreationStage::Queued
    );
    drop(slots);
}

#[tokio::test]
async fn cache_failure_rolls_back_catalog_but_retains_receipt_without_provider_replay() {
    let (store, job) = fixture().await;
    store.run(|c|{c.execute_batch("CREATE TRIGGER fail_catalog BEFORE INSERT ON kv WHEN NEW.key='account_folders' BEGIN SELECT RAISE(ABORT,'disk full'); END;")?;Ok(())}).await.expect("failure trigger");
    let mut first = api();
    first
        .expect_create()
        .times(1)
        .returning(|_| Ok(CreateOutcome::Acknowledged));
    first
        .expect_catalog()
        .times(1)
        .returning(|| Ok(vec![target()]));
    assert!(
        execute(&store, job.clone(), &first, &Default::default())
            .await
            .is_err()
    );
    let saved = store.creation_job(job.id).await.expect("saved receipt");
    assert_eq!(saved.stage, CreationStage::Repair);
    assert!(saved.provider_acknowledged);
    assert_eq!(saved.receipt, Some(target()));
    store
        .run(|c| {
            c.execute_batch("DROP TRIGGER fail_catalog")?;
            Ok(())
        })
        .await
        .expect("restore storage");
    let mut repair = MockCreationApi::new();
    repair.expect_create().never();
    repair
        .expect_catalog()
        .times(1)
        .returning(|| Ok(vec![target()]));
    assert_eq!(
        execute(&store, saved, &repair, &Default::default())
            .await
            .expect("repair")
            .stage,
        CreationStage::Succeeded
    );
}

#[tokio::test]
async fn acknowledged_create_saves_receipt_before_catalog_and_never_replays_on_repair() {
    let (store, job) = fixture().await;
    let mut first = api();
    first
        .expect_create()
        .times(1)
        .returning(|_| Ok(CreateOutcome::Acknowledged));
    first
        .expect_catalog()
        .times(1)
        .returning(|| anyhow::bail!("LIST response lost"));
    let saved = execute(&store, job, &first, &Default::default())
        .await
        .expect("receipt saved");
    assert_eq!(saved.stage, CreationStage::Repair);
    assert_eq!(
        store
            .creation_job(saved.id.clone())
            .await
            .expect("durable")
            .receipt,
        Some(target())
    );
    let mut repair = MockCreationApi::new();
    repair.expect_create().never();
    repair.expect_plan().never();
    repair
        .expect_catalog()
        .times(1)
        .returning(|| Ok(vec![target()]));
    let finished = execute(&store, saved, &repair, &Default::default())
        .await
        .expect("cache repaired");
    assert_eq!(finished.stage, CreationStage::Succeeded);
    assert_eq!(
        store
            .current_folder_catalog(finished.account)
            .await
            .expect("catalog"),
        vec![target()]
    );
}

#[tokio::test]
async fn uncertain_create_requires_explicit_exact_inspection_and_never_a_second_create() {
    let (store, job) = fixture().await;
    let mut first = api();
    first
        .expect_create()
        .times(1)
        .returning(|_| Ok(CreateOutcome::Uncertain("reply lost".into())));
    first.expect_catalog().never();
    let saved = execute(&store, job, &first, &Default::default())
        .await
        .expect("uncertain saved");
    assert_eq!(saved.stage, CreationStage::Uncertain);
    assert!(
        store
            .decide_creation(saved.id.clone(), saved.revision, true)
            .await
            .is_err()
    );
    let checking = store
        .decide_creation(saved.id, saved.revision, false)
        .await
        .expect("explicit check");
    let mut check = MockCreationApi::new();
    check.expect_create().never();
    check.expect_plan().never();
    check
        .expect_inspect()
        .with(mockall::predicate::eq(target()))
        .times(1)
        .returning(|_| Ok(Some(target())));
    check
        .expect_catalog()
        .times(1)
        .returning(|| Ok(vec![target()]));
    assert_eq!(
        execute(&store, checking, &check, &Default::default())
            .await
            .expect("checked")
            .stage,
        CreationStage::Succeeded
    );
}

#[tokio::test]
async fn definite_refusal_can_retry_same_target_but_stale_decision_cannot_dispatch() {
    let (store, job) = fixture().await;
    let mut first = api();
    first
        .expect_create()
        .times(1)
        .returning(|_| Ok(CreateOutcome::Rejected("NO".into())));
    let rejected = execute(&store, job, &first, &Default::default())
        .await
        .expect("rejected saved");
    assert_eq!(rejected.stage, CreationStage::Rejected);
    let retry = store
        .decide_creation(rejected.id.clone(), rejected.revision, false)
        .await
        .expect("retry");
    assert_eq!(retry.target, Some(target()));
    assert!(
        store
            .decide_creation(rejected.id, rejected.revision, false)
            .await
            .is_err()
    );
    let cancelled = store
        .decide_creation(retry.id, retry.revision, true)
        .await
        .expect("cancel before dispatch");
    assert_eq!(cancelled.stage, CreationStage::Cancelled);
}

#[tokio::test]
async fn invalid_folder_plan_requires_review_without_provider_create() {
    let (store, job) = fixture().await;
    let mut api = MockCreationApi::new();
    api.expect_plan().times(1).returning(|_, _| {
        Err(crate::folder_actions::creation::PlanRejected(
            "The parent cannot contain folders.".into(),
        )
        .into())
    });
    api.expect_inspect().never();
    api.expect_create().never();
    let saved = execute(&store, job, &api, &Default::default())
        .await
        .expect("retained review");
    assert_eq!(saved.stage, CreationStage::Rejected);
    assert!(saved.target.is_none());
    assert!(saved.receipt.is_none());
}

#[tokio::test]
async fn planning_failure_waits_without_create_and_stopped_owner_leaves_queued_work() {
    let (store, job) = fixture().await;
    let mut api = MockCreationApi::new();
    api.expect_plan()
        .times(1)
        .returning(|_, _| anyhow::bail!("offline"));
    api.expect_create().never();
    let saved = execute(&store, job, &api, &Default::default())
        .await
        .expect("waiting");
    assert_eq!(saved.stage, CreationStage::Waiting);
    let queued = store
        .decide_creation(saved.id, saved.revision, false)
        .await
        .expect("retry");
    let stopped = crate::lifecycle::Signal::default();
    stopped.set(true);
    assert_eq!(
        execute(&store, queued.clone(), &MockCreationApi::new(), &stopped)
            .await
            .expect("stop"),
        queued
    );
}

#[tokio::test]
async fn abandoned_dispatch_becomes_uncertain_and_import_cannot_authorise_unsent_work() {
    let (store, job) = fixture().await;
    let job = store
        .update_creation(job, CreationStage::Running, Some(target()), None, None)
        .await
        .expect("dispatch");
    store.recover_creations().await.expect("restart");
    assert_eq!(
        store.creation_job(job.id).await.expect("saved").stage,
        CreationStage::Uncertain
    );
    let (other, queued) = fixture().await;
    other
        .run(|c| crate::store::folder_creation::fence_import(c, "Imported review"))
        .await
        .expect("fence");
    assert_eq!(
        other
            .creation_job(queued.id)
            .await
            .expect("saved import")
            .stage,
        CreationStage::Uncertain
    );
}
