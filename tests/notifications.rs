use shep::{
    model::{Preferences, StoredMail, parse_mail},
    store::Store,
};

fn mail(account: &str, uid: &str, message_id: Option<&str>, body: &str) -> StoredMail {
    let header = message_id
        .map(|id| format!("Message-ID: <{id}@example.test>\r\n"))
        .unwrap_or_default();
    parse_mail(
        account, uid, "INBOX",
        format!("From: Friend <friend@example.test>\r\nDate: Mon, 07 Sep 2026 10:00:00 +0000\r\nSubject: Mail update\r\n{header}\r\n{body}").into_bytes(),
        true, false,
    ).unwrap()
}

async fn ready(store: &Store, account: &str, epoch: &str) {
    store
        .begin_notification_sync(account.into(), epoch.into())
        .await
        .unwrap();
    store
        .finish_notification_sync(account.into(), epoch.into())
        .await
        .unwrap();
}

#[tokio::test]
async fn notification_controls_default_on_and_persist_independently() {
    let defaults: Preferences = serde_json::from_str("{}").unwrap();
    assert!(defaults.notifications.popups && defaults.notifications.sound);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.sqlite");
    let store = Store::open(&path).unwrap();
    let mut preferences = defaults;
    preferences.notifications.popups = false;
    preferences.notifications.show_details = false;
    store.save_preferences(preferences).await.unwrap();
    drop(store);
    let store = Store::open(path).unwrap();
    let preferences: Preferences = store.get("preferences").await.unwrap();
    assert!(!preferences.notifications.popups);
    assert!(preferences.notifications.sound);
    assert!(!preferences.notifications.show_details);
}

#[tokio::test]
async fn partial_initial_import_stays_quiet_across_restart_then_new_mail_alerts_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.sqlite");
    let store = Store::open(&path).unwrap();
    store
        .begin_notification_sync("work".into(), "imap:7".into())
        .await
        .unwrap();
    let old = mail("work", "7.1", Some("old"), "Old mail");
    assert!(store.sync_message(old.clone()).await.unwrap().is_none());
    drop(store); // The provider never completed the first import.
    let store = Store::open(&path).unwrap();
    store
        .begin_notification_sync("work".into(), "imap:7".into())
        .await
        .unwrap();
    assert!(
        store
            .sync_message(mail("work", "7.2", Some("also-old"), "Old mail"))
            .await
            .unwrap()
            .is_none()
    );
    store
        .finish_notification_sync("work".into(), "imap:7".into())
        .await
        .unwrap();
    let fresh = mail("work", "7.3", Some("new"), "New mail");
    let arrival = store.sync_message(fresh.clone()).await.unwrap().unwrap();
    assert_eq!(arrival.message, fresh.summary.id);
    assert_eq!(arrival.account, "work");
    assert!(store.sync_message(fresh.clone()).await.unwrap().is_none());
    drop(store);
    let store = Store::open(path).unwrap();
    store
        .begin_notification_sync("work".into(), "imap:7".into())
        .await
        .unwrap();
    assert!(store.sync_message(fresh).await.unwrap().is_none());
    assert!(store.sync_message(old).await.unwrap().is_none());
    assert!(
        store
            .sync_message(mail("work", "7.4", Some("newer"), "Newer mail"))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn epoch_reset_is_quiet_and_obsolete_completion_cannot_arm_it() {
    let store = Store::memory().unwrap();
    ready(&store, "work", "imap:7").await;
    assert!(
        store
            .sync_message(mail("work", "7.1", Some("received"), "Received"))
            .await
            .unwrap()
            .is_some()
    );
    store
        .begin_notification_sync("work".into(), "imap:8".into())
        .await
        .unwrap();
    store
        .finish_notification_sync("work".into(), "imap:7".into())
        .await
        .unwrap();
    assert!(
        store
            .sync_message(mail("work", "8.1", Some("old-in-reset"), "Old mail"))
            .await
            .unwrap()
            .is_none()
    );
    store
        .finish_notification_sync("work".into(), "imap:8".into())
        .await
        .unwrap();
    assert!(
        store
            .sync_message(mail("work", "8.2", Some("after-reset"), "New mail"))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .sync_message(mail("work", "8.3", Some("received"), "Received"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn imports_moves_and_read_flags_do_not_become_new_arrivals() {
    let store = Store::memory().unwrap();
    ready(&store, "work", "imap:7").await;
    let imported = mail("work", "7.1", Some("imported"), "Imported mail");
    store.upsert(vec![imported.clone()]).await.unwrap();
    assert!(store.sync_message(imported).await.unwrap().is_none());
    assert!(
        store
            .sync_message(mail("work", "7.2", Some("imported"), "Imported mail"))
            .await
            .unwrap()
            .is_none()
    );
    let mut read = mail(
        "work",
        "7.3",
        Some("already-read"),
        "Read on another device",
    );
    read.summary.unread = false;
    assert!(store.sync_message(read.clone()).await.unwrap().is_none());
    read.summary.unread = true;
    assert!(store.sync_message(read).await.unwrap().is_none());
    let mut sent = mail("work", "7.4", Some("sent"), "A sent message");
    sent.summary.folder = "Sent".into();
    assert!(store.sync_message(sent).await.unwrap().is_none());
    assert!(
        store
            .sync_message(mail("work", "7.5", Some("sent"), "A sent message"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn identity_is_account_scoped_and_does_not_treat_references_as_arrivals() {
    let store = Store::memory().unwrap();
    for account in ["work", "personal"] {
        ready(&store, account, "pop3").await;
    }
    for account in ["work", "personal"] {
        assert!(
            store
                .sync_message(mail(
                    account,
                    "uid-1",
                    Some("same-logical"),
                    "Delivered to both accounts"
                ))
                .await
                .unwrap()
                .is_some()
        );
    }
    let reply = parse_mail("work", "uid-2", "INBOX", b"Message-ID: <reply@example.test>\r\nIn-Reply-To: <later-parent@example.test>\r\n\r\nReply".to_vec(), true, false).unwrap();
    assert!(store.sync_message(reply).await.unwrap().is_some());
    assert!(
        store
            .sync_message(mail(
                "work",
                "uid-3",
                Some("later-parent"),
                "Parent arrived later"
            ))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn missing_message_id_uses_exact_content_without_conflating_equal_subjects() {
    let store = Store::memory().unwrap();
    ready(&store, "work", "pop3").await;
    assert!(
        store
            .sync_message(mail("work", "one", None, "First body"))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        store
            .sync_message(mail("work", "copy", None, "First body"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .sync_message(mail("work", "two", None, "Different body"))
            .await
            .unwrap()
            .is_some()
    );
    // Simulate an older cache, before notification identities were recorded.
    store
        .run(|c| {
            c.execute("DELETE FROM notification_seen", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        store
            .sync_message(mail("work", "legacy-copy", None, "First body"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .sync_message(mail("work", "new", None, "Another different body"))
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn failed_mail_commit_cannot_consume_an_arrival_and_payload_text_is_bounded() {
    let store = Store::memory().unwrap();
    ready(&store, "work", "imap:7").await;
    store.run(|c| { c.execute_batch("CREATE TRIGGER fail_notification BEFORE INSERT ON notification_seen BEGIN SELECT RAISE(ABORT,'fixture failure'); END;")?; Ok(()) }).await.unwrap();
    let mut fresh = mail("work", "7.1", Some("new"), "Body");
    fresh.summary.subject = format!("{}\n\x1b", "界".repeat(400));
    assert!(store.sync_message(fresh.clone()).await.is_err());
    assert!(store.detail(fresh.summary.id.clone()).await.is_err());
    store
        .run(|c| {
            c.execute_batch("DROP TRIGGER fail_notification")?;
            Ok(())
        })
        .await
        .unwrap();
    let arrival = store.sync_message(fresh).await.unwrap().unwrap();
    assert_eq!(arrival.subject.chars().count(), 200);
    assert!(!arrival.subject.chars().any(char::is_control));
}
