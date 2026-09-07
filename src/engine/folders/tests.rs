use super::*;
use crate::folder_actions::{Action, Status};
use crate::folders::Mailbox;

async fn engine() -> Engine {
    let engine = super::super::calendar_tests::engine();
    let account:Account=serde_json::from_value(serde_json::json!({"id":"folder-test","name":"Folders","email":"folders@example.test","protocol":"Imap","host":"localhost","port":993,"username":"folders","smtp_host":"localhost","smtp_port":465})).unwrap();
    engine.store.save_account(account).await.unwrap();
    engine
        .store
        .save_folder_catalog(
            "folder-test".into(),
            ["INBOX", "Projects", "Projects/Design", "Archive"]
                .into_iter()
                .map(|name| Mailbox {
                    delimiter: Some('/'),
                    ..Mailbox::flat(name.into())
                })
                .collect(),
        )
        .await
        .unwrap();
    engine.store.upsert(vec![parse_mail("folder-test","1.42","Projects",b"From: sender@example.test\r\nSubject: Kept original\r\n\r\nBody survives a folder move".to_vec(),true,false).unwrap()]).await.unwrap();
    engine
}
#[tokio::test]
async fn folder_review_and_staging_use_local_dispatch_with_all_provider_slots_occupied() {
    let engine = engine().await;
    let mut holds = Vec::new();
    for _ in 0..8 {
        holds.push(engine.provider_slots.acquire().await);
    }
    let (tx, input) = CommandSender::channel();
    let (output, mut events) = futures::channel::mpsc::channel(32);
    let worker = tokio::spawn(engine.clone().run(input, output));
    tx.try_send(Command::Folder(Request::Review(
        7,
        "folder-test".into(),
        "Projects".into(),
        Action::Move {
            parent: Some("Archive".into()),
        },
    )))
    .unwrap();
    let preview = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(super::super::Event::Folder(Event::Review(7, result))) = events.next().await
            {
                break result.unwrap();
            }
        }
    })
    .await
    .unwrap();
    tx.try_send(Command::Folder(Request::Start(
        "queued".into(),
        preview.review.clone(),
    )))
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(super::super::Event::Folder(Event::Started(id, result))) =
                events.next().await
            {
                assert_eq!(id, "queued");
                assert!(result.is_ok());
                break;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        holds.len(),
        8,
        "Keep providers blocked throughout both local commands"
    );
    assert_eq!(
        engine
            .store
            .folder_job("queued".into())
            .await
            .unwrap()
            .steps[0]
            .status,
        Status::Queued
    );
    worker.abort();
    let _ = worker.await;
}
#[tokio::test]
async fn folder_worker_resumes_durable_jobs_and_close_stops_before_starting_another_step() {
    let engine = engine().await;
    let preview = engine
        .folder_preview(
            "folder-test".into(),
            "Projects".into(),
            Action::Move {
                parent: Some("Archive".into()),
            },
        )
        .await
        .unwrap();
    engine
        .store
        .start_folder_change("move".into(), (*preview.review).clone())
        .await
        .unwrap();
    engine.bulk_control.stopping.store(true, SeqCst);
    let (output, mut events) = futures::channel::mpsc::channel(32);
    engine
        .execute_folder_job("move".into(), output.clone())
        .await;
    assert!(matches!(
        events.next().await,
        Some(super::super::Event::BulkStopped)
    ));
    assert_eq!(
        engine.store.folder_job("move".into()).await.unwrap().steps[0].status,
        Status::Queued
    );
    engine.bulk_control.stopping.store(false, SeqCst);
    engine.drain_folder_jobs(output.clone()).await;
    let done = engine.store.folder_job("move".into()).await.unwrap();
    assert!(done.closed);
    assert_eq!(done.steps[0].status, Status::Done);
    assert!(!engine.bulk_control.active.load(SeqCst));
    assert_eq!(
        engine
            .store
            .query(MailQuery {
                account: Some("folder-test".into()),
                folder: "Archive/Projects".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .rows[0]
            .subject,
        "Kept original"
    );
    let before = done.revision;
    engine.drain_folder_jobs(output).await;
    assert_eq!(
        engine
            .store
            .folder_job("move".into())
            .await
            .unwrap()
            .revision,
        before,
        "A completed job must not execute on another wake"
    );
}

#[tokio::test]
async fn closing_a_folder_job_waiting_for_capacity_keeps_it_queued_without_waiting_for_providers() {
    let engine = engine().await;
    let preview = engine
        .folder_preview("folder-test".into(), "Projects".into(), Action::Delete)
        .await
        .unwrap();
    engine
        .store
        .start_folder_change("waiting".into(), (*preview.review).clone())
        .await
        .unwrap();
    let mut holds = Vec::new();
    for _ in 0..8 {
        holds.push(engine.provider_slots.acquire().await);
    }
    let (output, mut events) = futures::channel::mpsc::channel(32);
    let worker = tokio::spawn({
        let engine = engine.clone();
        async move { engine.execute_folder_job("waiting".into(), output).await }
    });
    assert!(matches!(
        events.next().await,
        Some(super::super::Event::Folder(Event::Update(_)))
    ));
    assert!(engine.bulk_control.active.load(SeqCst));
    engine.bulk_control.stopping.store(true, SeqCst);
    tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        holds.len(),
        8,
        "Providers stay occupied until the closing worker has returned"
    );
    let job = engine.store.folder_job("waiting".into()).await.unwrap();
    assert!(job.steps.iter().all(|step| step.status == Status::Queued));
    assert!(!engine.bulk_control.active.load(SeqCst));
    assert!(engine.bulk_control.stopping.load(SeqCst));
}
