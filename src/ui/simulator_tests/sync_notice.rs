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
        sequence: 100,
        origin: SyncOrigin::Background,
    };
    h.inject(Message::Backend(Event::MailSyncStarted(attempt.clone())))
        .await;
    h.inject(Message::Backend(Event::MailSyncFinished(
        attempt.clone(),
        Err("Fixture check failed".into()),
    )))
    .await;
    h.expect("notice", serde_json::Value::Null).await;
    h.age_sync_failure(&attempt);
    h.inject(Message::Tick).await;
    h.expect("notice", "Mail checks are still failing. Try Refresh.")
        .await;
    assert!(!h.snapshot().matches_hash(&before).expect("notice snapshot"));
    // The notice bar ends with its 40 px Dismiss (×) button inside 14 px of
    // padding, on the same row as the message.
    let row = h.text_center("Mail checks are still failing. Try Refresh.");
    let width = h.width();
    h.click_at(width - 14. - 20., row.y).await;
    h.expect("notice", serde_json::Value::Null).await;
    h.inject(Message::Tick).await;
    h.expect("notice", serde_json::Value::Null).await;
}

scenarios!(delayed_banner_is_visible_and_dismissible);
