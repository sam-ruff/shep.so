use super::*;

fn target() -> ConnectionRef {
    ConnectionRef {
        kind: ConnectionKind::Calendar,
        id: "held-removal".into(),
    }
}

async fn admission(engine: &Engine) -> RemovalJob {
    engine
        .store
        .save_source(CalendarSource {
            id: target().id,
            name: "Held calendar".into(),
            kind: CalendarKind::CalDav,
            url: "https://calendar.example.test/held/".into(),
            username: "alex".into(),
            access: Default::default(),
        })
        .await
        .expect("source");
    let review = engine
        .store
        .removal_preview(target())
        .await
        .expect("review");
    engine
        .store
        .admit_connection_removal(uuid::Uuid::new_v4().to_string(), review, false)
        .await
        .expect("admission")
}

#[tokio::test]
async fn removal_admission_hides_calendar_while_provider_drains_without_global_lock() {
    let mut engine = crate::engine::calendar_tests::engine();
    let mut remover = MockSecretRemover::new();
    remover
        .expect_remove()
        .with(mockall::predicate::eq("held-removal"))
        .times(1)
        .returning(|_| Ok(()));
    engine.secret_remover = Arc::new(remover);
    let engine = Arc::new(engine);
    let held = engine.connection_access(&target()).await;
    let job = admission(&engine).await;
    assert!(
        engine
            .store
            .workspace()
            .await
            .expect("hidden")
            .calendars
            .is_empty()
    );
    let (output, _events) = futures::channel::mpsc::channel(32);
    let worker = {
        let engine = engine.clone();
        let id = job.id.clone();
        tokio::spawn(async move { engine.execute_removal_work(id, output).await })
    };
    tokio::task::yield_now().await;
    let independent = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        engine.connection_lifecycle.write(),
    )
    .await
    .expect("unrelated connection is not blocked");
    drop(independent);
    assert!(!worker.is_finished());
    assert!(
        !engine
            .store
            .removal_job(job.id.clone())
            .await
            .expect("queued")
            .local_done
    );
    drop(held);
    assert!(worker.await.expect("worker"));
    assert_eq!(
        engine
            .store
            .removal_job(job.id)
            .await
            .expect("receipt")
            .stage,
        RemovalStage::Succeeded
    );
}

#[tokio::test]
async fn stopping_during_provider_drain_retains_admission_for_restart() {
    let mut engine = crate::engine::calendar_tests::engine();
    let mut remover = MockSecretRemover::new();
    remover.expect_remove().times(1).returning(|_| Ok(()));
    engine.secret_remover = Arc::new(remover);
    let engine = Arc::new(engine);
    let held = engine.connection_access(&target()).await;
    let job = admission(&engine).await;
    let (output, _events) = futures::channel::mpsc::channel(32);
    let worker = {
        let engine = engine.clone();
        let id = job.id.clone();
        let output = output.clone();
        tokio::spawn(async move { engine.execute_removal_work(id, output).await })
    };
    engine.bulk_control.stopping.set(true);
    assert!(!worker.await.expect("stopped worker"));
    assert_eq!(
        engine
            .store
            .removal_job(job.id.clone())
            .await
            .expect("retained"),
        job
    );
    drop(held);
    engine.bulk_control.stopping.set(false);
    assert!(engine.execute_removal_work(job.id.clone(), output).await);
    assert_eq!(
        engine
            .store
            .removal_job(job.id)
            .await
            .expect("completed")
            .stage,
        RemovalStage::Succeeded
    );
}

#[tokio::test]
async fn credential_failure_keeps_account_hidden_and_retries_same_removal() {
    let mut engine = crate::engine::calendar_tests::engine();
    let mut remover = MockSecretRemover::new();
    let mut sequence = mockall::Sequence::new();
    remover
        .expect_remove()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| anyhow::bail!("locked"));
    remover
        .expect_remove()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(()));
    engine.secret_remover = Arc::new(remover);
    let job = admission(&engine).await;
    let (output, _events) = futures::channel::mpsc::channel(32);
    assert!(
        engine
            .execute_removal_work(job.id.clone(), output.clone())
            .await
    );
    let failed = engine
        .store
        .removal_job(job.id.clone())
        .await
        .expect("failure");
    assert_eq!(failed.stage, RemovalStage::Failed);
    assert!(failed.local_done);
    assert!(
        engine
            .store
            .workspace()
            .await
            .expect("hidden")
            .calendars
            .is_empty()
    );
    engine.retry_removal(failed.clone()).await.expect("retry");
    assert!(engine.retry_removal(failed).await.is_err());
    assert!(engine.execute_removal_work(job.id.clone(), output).await);
    assert_eq!(
        engine
            .store
            .removal_job(job.id)
            .await
            .expect("complete")
            .stage,
        RemovalStage::Succeeded
    );
}

#[tokio::test]
async fn imported_removal_finishes_local_cleanup_without_touching_device_credentials() {
    let mut engine = crate::engine::calendar_tests::engine();
    let mut remover = MockSecretRemover::new();
    remover.expect_remove().times(0);
    engine.secret_remover = Arc::new(remover);
    let original = admission(&engine).await;
    engine
        .store
        .run(|c| crate::store::fence_removal_import(c))
        .await
        .expect("import fence");
    let imported = engine
        .store
        .removal_job(original.id.clone())
        .await
        .expect("imported job");
    assert!(!imported.device_credentials);
    assert!(imported.revision > original.revision);
    let (output, _events) = futures::channel::mpsc::channel(32);
    assert!(
        engine
            .execute_removal_work(original.id.clone(), output)
            .await
    );
    assert_eq!(
        engine
            .store
            .removal_job(original.id)
            .await
            .expect("local completion")
            .stage,
        RemovalStage::Succeeded
    );
    assert!(
        engine
            .store
            .cleanup_jobs()
            .await
            .expect("no device cleanup")
            .is_empty()
    );
}

#[tokio::test]
async fn local_cleanup_failure_retains_hidden_projection_and_requires_explicit_retry() {
    let engine = crate::engine::calendar_tests::engine();
    let job = admission(&engine).await;
    engine.store.run(|c| {
        c.execute_batch("CREATE TRIGGER refuse_local_removal BEFORE DELETE ON events BEGIN SELECT RAISE(FAIL,'fixture local deletion failure'); END")?;
        // Admission hid the source, so seed only the already owned cache fixture.
        let event=crate::engine::calendar_tests::event("held-removal");
        c.execute("INSERT INTO events(id,source,start,data) VALUES(?,?,0,?)",rusqlite::params![event.id,event.source_id,serde_json::to_string(&event)?])?;
        Ok(())
    }).await.expect("failure fixture");
    let (output, _events) = futures::channel::mpsc::channel(32);
    assert!(
        !engine
            .execute_removal_work(job.id.clone(), output.clone())
            .await
    );
    let failed = engine
        .store
        .removal_job(job.id.clone())
        .await
        .expect("saved failure");
    assert_eq!(failed.stage, RemovalStage::Failed);
    assert!(!failed.local_done);
    assert!(
        engine
            .store
            .events()
            .await
            .expect("hidden events")
            .is_empty()
    );
    assert!(
        engine
            .store
            .calendar_action_snapshot()
            .await
            .expect("hidden journal view")
            .2
            .is_empty()
    );
    assert!(
        !engine
            .execute_removal_work(job.id.clone(), output.clone())
            .await
    );
    engine
        .store
        .run(|c| {
            c.execute_batch("DROP TRIGGER refuse_local_removal")?;
            Ok(())
        })
        .await
        .expect("restore storage");
    engine.retry_removal(failed).await.expect("explicit retry");
    assert!(engine.execute_removal_work(job.id.clone(), output).await);
    assert_eq!(
        engine
            .store
            .removal_job(job.id)
            .await
            .expect("completed")
            .stage,
        RemovalStage::Succeeded
    );
}
