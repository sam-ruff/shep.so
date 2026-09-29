use super::*;
use crate::engine::{SyncAttempt, SyncOrigin};

async fn delayed_banner_is_visible_and_dismissible() {
    let mut h = Harness::start().await;
    let directory = tempfile::tempdir().expect("snapshot directory");
    let before = directory.path().join("before-notice");
    assert!(
        h.snapshot()
            .matches_hash(&before)
            .expect("initial snapshot")
    );
    let attempt = SyncAttempt {
        account: "preview".into(),
        connection: String::new(),
        connection_revision: 0,
        sequence: 100,
        origin: SyncOrigin::Background,
    };
    h.inject(Message::Backend(Event::MailSyncStarted(attempt.clone())))
        .await;
    h.age_sync_failure(&attempt);
    h.inject(Message::Backend(Event::MailSyncFinished(
        attempt,
        Err("Fixture check failed".into()),
    )))
    .await;
    h.expect("notice", serde_json::Value::Null).await;
    h.inject(Message::Tick).await;
    h.expect("notice", "Mail checks are still failing. Try Refresh.")
        .await;
    assert!(!h.snapshot().matches_hash(&before).expect("notice snapshot"));
    h.inject(Message::Dismiss).await;
    h.expect("notice", serde_json::Value::Null).await;
    h.inject(Message::Tick).await;
    h.expect("notice", serde_json::Value::Null).await;
}

scenarios!(delayed_banner_is_visible_and_dismissible);
