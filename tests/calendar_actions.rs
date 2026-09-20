use shep::{model::*, store::Store};

#[tokio::test]
async fn safe_offline_wait_survives_restart_and_caps_automatic_retries() {
    use shep::providers::calendar::WaitReason;
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("calendar.db");
    let store = Store::open(&path).expect("store");
    prepare(&store).await;
    store
        .admit_calendar_action("one".into(), event("Keep pending"), false)
        .await
        .expect("admit");
    let first = store
        .claim_calendar_action("one".into())
        .await
        .expect("claim");
    store
        .wait_calendar_action(
            "one".into(),
            first.revision,
            WaitReason::Offline,
            "Offline".into(),
        )
        .await
        .expect("wait");
    assert!(
        store
            .next_calendar_action()
            .await
            .expect("backoff")
            .is_none()
    );
    drop(store);
    let store = Store::open(&path).expect("restart");
    store.recover_calendar_actions().await.expect("recover");
    assert_eq!(
        store
            .calendar_job("one".into())
            .await
            .expect("waiting")
            .status,
        "waiting"
    );
    for attempt in 2..=6 {
        store
            .run(|c| {
                c.execute(
                    "UPDATE calendar_actions SET data=json_set(data,'$.retry_at',0) WHERE id='one'",
                    [],
                )?;
                Ok(())
            })
            .await
            .expect("advance deadline");
        let claimed = store
            .claim_calendar_action("one".into())
            .await
            .expect("safe retry");
        assert_eq!(claimed.attempts, attempt);
        let waiting = store
            .wait_calendar_action(
                "one".into(),
                claimed.revision,
                WaitReason::Offline,
                "Still offline".into(),
            )
            .await
            .expect("wait");
        assert_eq!(waiting.retry_at.is_some(), attempt < 6);
    }
    assert!(
        store
            .next_calendar_retry()
            .await
            .expect("manual recovery")
            .is_none()
    );
    let exhausted = store.calendar_job("one".into()).await.expect("exhausted");
    let retried = store
        .retry_calendar_action("one".into(), exhausted.revision)
        .await
        .expect("manual retry");
    assert_eq!(retried.id, "one");
    assert_eq!(retried.status, "queued");
    assert_eq!(retried.attempts, 0);
}

#[tokio::test]
async fn authentication_wait_requires_explicit_retry_and_unknown_result_cannot_retry() {
    use shep::providers::calendar::WaitReason;
    let store = Store::memory().expect("store");
    prepare(&store).await;
    store
        .admit_calendar_action("one".into(), event("Retain"), false)
        .await
        .expect("admit");
    let claimed = store
        .claim_calendar_action("one".into())
        .await
        .expect("claim");
    let waiting = store
        .wait_calendar_action(
            "one".into(),
            claimed.revision,
            WaitReason::Authentication,
            "Reconnect".into(),
        )
        .await
        .expect("wait");
    assert!(waiting.retry_at.is_none());
    assert!(store.claim_calendar_action("one".into()).await.is_err());
    store
        .retry_calendar_action("one".into(), waiting.revision)
        .await
        .expect("explicit retry");
    let claimed = store
        .claim_calendar_action("one".into())
        .await
        .expect("claim");
    let unknown = store
        .fail_calendar_action(
            "one".into(),
            claimed.revision,
            true,
            "Lost write result".into(),
        )
        .await
        .expect("unknown");
    assert!(
        store
            .retry_calendar_action("one".into(), unknown.revision)
            .await
            .is_err()
    );
}

fn source() -> CalendarSource {
    CalendarSource {
        id: "home".into(),
        name: "Home".into(),
        kind: CalendarKind::CalDav,
        url: "https://calendar.example/home/".into(),
        username: "fixture".into(),
        access: Default::default(),
    }
}

fn event(title: &str) -> CalendarEvent {
    let start = chrono::Utc::now();
    CalendarEvent {
        id: "local-id".into(),
        source_id: "home".into(),
        title: title.into(),
        start,
        end: start + chrono::Duration::hours(1),
        all_day: false,
        etag: None,
        remote_url: None,
        location: String::new(),
        description: String::new(),
    }
}

async fn prepare(store: &Store) {
    store
        .put("calendars", vec![source()])
        .await
        .expect("sources");
}

#[tokio::test]
async fn acknowledged_missing_version_remains_reviewable_after_cache_and_restart() {
    let store = Store::memory().expect("store");
    prepare(&store).await;
    let original = event("Saved");
    store
        .admit_calendar_action("one".into(), original.clone(), false)
        .await
        .expect("admit");
    let claimed = store
        .claim_calendar_action("one".into())
        .await
        .expect("claim");
    let mut receipt = original;
    receipt.remote_url = Some("https://calendar.example/home/one.ics".into());
    store
        .record_calendar_receipt("one".into(), claimed.revision, receipt)
        .await
        .expect("receipt");
    let repair = store
        .apply_calendar_receipt("one".into())
        .await
        .expect("cache");
    assert_eq!(repair.status, "repair");
    assert!(repair.cache_applied);
    store.recover_calendar_actions().await.expect("restart");
    assert!(store.next_calendar_action().await.expect("next").is_none());
    assert!(store.apply_calendar_receipt("one".into()).await.is_err());
    let checked = store
        .check_calendar_action("one".into(), repair.revision, None)
        .await
        .expect("server since deleted");
    store
        .resolve_calendar_action("one".into(), checked.revision)
        .await
        .expect("accept");
    assert!(store.calendar_jobs().await.expect("jobs").is_empty());
}

#[tokio::test]
async fn restart_retains_queued_but_never_replays_unacknowledged_dispatch() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("calendar.db");
    let store = Store::open(&path).expect("store");
    prepare(&store).await;
    let request = event("Queued");
    let admitted = store
        .admit_calendar_action("one".into(), request.clone(), false)
        .await
        .expect("admit");
    assert_eq!(
        store
            .admit_calendar_action("one".into(), request, false)
            .await
            .expect("lost admission reply")
            .revision,
        admitted.revision
    );
    drop(store);
    let store = Store::open(&path).expect("reopen");
    store.recover_calendar_actions().await.expect("recover");
    assert_eq!(
        store.next_calendar_action().await.expect("next"),
        Some("one".into())
    );
    store
        .claim_calendar_action("one".into())
        .await
        .expect("claim");
    let observer = Store::open(&path).expect("observer");
    assert_eq!(
        observer
            .calendar_job("one".into())
            .await
            .expect("live observation")
            .status,
        "running"
    );
    drop(observer);
    drop(store);
    let store = Store::open(&path).expect("restart");
    store.recover_calendar_actions().await.expect("recover");
    assert_eq!(
        store.calendar_job("one".into()).await.expect("job").status,
        "uncertain"
    );
    assert!(store.next_calendar_action().await.expect("next").is_none());
}

#[tokio::test]
async fn receipt_survives_restart_and_rebinds_newer_intent_without_replaying_provider() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("calendar.db");
    let store = Store::open(&path).expect("store");
    prepare(&store).await;
    let original = event("First");
    store
        .admit_calendar_action("one".into(), original.clone(), false)
        .await
        .expect("admit");
    let first = store
        .claim_calendar_action("one".into())
        .await
        .expect("claim");
    let mut newer = original.clone();
    newer.title = "Newer".into();
    store
        .admit_calendar_action("two".into(), newer, false)
        .await
        .expect("successor");
    let mut receipt = original;
    receipt.id = "provider-id".into();
    receipt.etag = Some("v1".into());
    store
        .record_calendar_receipt("one".into(), first.revision, receipt)
        .await
        .expect("acknowledge");
    assert!(store.calendar_snapshot().await.expect("cache").1.is_empty());
    drop(store);
    let store = Store::open(&path).expect("restart");
    store.recover_calendar_actions().await.expect("recover");
    assert_eq!(
        store
            .calendar_job("one".into())
            .await
            .expect("receipt")
            .status,
        "repair"
    );
    store
        .apply_calendar_receipt("one".into())
        .await
        .expect("cache repair");
    let second = store
        .claim_calendar_action("two".into())
        .await
        .expect("next");
    assert_eq!(second.event.id, "provider-id");
    assert_eq!(second.event.etag.as_deref(), Some("v1"));
    assert_eq!(second.event.title, "Newer");
    assert_eq!(
        store.calendar_snapshot().await.expect("cache").1[0].title,
        "First"
    );
}

#[tokio::test]
async fn checked_newer_server_state_cannot_be_overwritten_by_old_repair() {
    let store = Store::memory().expect("store");
    prepare(&store).await;
    let original = event("Requested");
    store
        .admit_calendar_action("one".into(), original.clone(), false)
        .await
        .expect("admit");
    let claimed = store
        .claim_calendar_action("one".into())
        .await
        .expect("claim");
    let repair = store
        .record_calendar_receipt("one".into(), claimed.revision, original.clone())
        .await
        .expect("receipt");
    let mut current = original;
    current.title = "Later server edit".into();
    let checked = store
        .check_calendar_action("one".into(), repair.revision, Some(current))
        .await
        .expect("checked");
    assert!(store.next_calendar_action().await.expect("next").is_none());
    assert!(store.apply_calendar_receipt("one".into()).await.is_err());
    let resolved = store
        .resolve_calendar_action("one".into(), checked.revision)
        .await
        .expect("accept observed");
    assert_eq!(
        resolved.status, "cancelled",
        "accepting observed state is not provider success"
    );
    assert_eq!(
        store.calendar_snapshot().await.expect("cache").1[0].title,
        "Later server edit"
    );
}

#[tokio::test]
async fn unknown_delete_requires_checked_absence_and_keeps_newer_edit_waiting() {
    let store = Store::memory().expect("store");
    prepare(&store).await;
    let original = event("Original");
    store.save_event(original.clone()).await.expect("cache");
    store
        .admit_calendar_action("one".into(), original.clone(), true)
        .await
        .expect("delete");
    let claimed = store
        .claim_calendar_action("one".into())
        .await
        .expect("claim");
    let unknown = store
        .fail_calendar_action("one".into(), claimed.revision, true, "Lost response".into())
        .await
        .expect("unknown");
    store
        .admit_calendar_action("two".into(), original, false)
        .await
        .expect("newer edit");
    assert!(store.claim_calendar_action("two".into()).await.is_err());
    assert!(
        store
            .resolve_calendar_action("one".into(), unknown.revision)
            .await
            .is_err()
    );
    let checked = store
        .check_calendar_action("one".into(), unknown.revision, None)
        .await
        .expect("checked absence");
    store
        .resolve_calendar_action("one".into(), checked.revision)
        .await
        .expect("accept");
    assert!(store.calendar_snapshot().await.expect("cache").1.is_empty());
}
