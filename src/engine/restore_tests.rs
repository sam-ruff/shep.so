use super::*;
use async_trait::async_trait;
use std::{collections::HashMap, sync::Mutex};

#[derive(Default)]
struct Credentials {
    entries: Mutex<HashMap<String, SecretString>>,
    calls: Mutex<Vec<String>>,
    locked: Mutex<HashSet<String>>,
}
#[async_trait]
impl backup::restore::CredentialRestorer for Credentials {
    async fn restore_missing(&self, id: &str, secret: SecretString) -> anyhow::Result<()> {
        self.calls.lock().unwrap().push(id.into());
        anyhow::ensure!(
            !self.locked.lock().unwrap().contains(id),
            "Fixture keychain is locked"
        );
        self.entries
            .lock()
            .unwrap()
            .entry(id.into())
            .or_insert(secret);
        Ok(())
    }
}

fn fixture() -> Snapshot {
    Snapshot {
        version: 1,
        created_at: 1788700000,
        accounts: vec![serde_json::from_value(serde_json::json!({
            "id": "work", "name": "Work", "email": "sam@example.com", "protocol": "Imap",
            "host": "imap.example.com", "port": 993, "username": "sam@example.com",
            "smtp_host": "smtp.example.com", "smtp_port": 465, "smtp_separate_password": true
        })).unwrap()],
        calendars: vec![CalendarSource { access: Default::default(),
            id: "home".into(), name: "Home".into(), kind: CalendarKind::CalDav,
            url: "https://calendar.example.com/dav/sam/".into(), username: "sam".into(),
        }, CalendarSource { access: Default::default(),
            id: "google:work@example.com".into(), name: "Shared".into(), kind: CalendarKind::Google,
            url: "work@example.com".into(), username: String::new(),
        }],
        messages: (0..2).map(|n| parse_mail("work", &format!("9:{n}"), "INBOX",
            format!("From: Sam <sam@example.com>\r\nSubject: Restored {n}\r\nDate: Sun, 6 Sep 2026 12:00:00 +0000\r\n\r\nUseful original {n}").into_bytes(),
            true, false).unwrap()).collect(),
        preferences: Preferences::default(),
        credentials: [("work", "incoming-original"), ("work:smtp", "smtp-original"), ("home", "calendar-original")]
            .into_iter().map(|(id, value)| (id.into(), value.into())).collect(),
    }
}

fn engine(credentials: Arc<Credentials>) -> Engine {
    Engine {
        profiles: None,
        credentials: Default::default(),
        store: Store::memory().unwrap(),
        google: Default::default(),
        demo: false,
        account_work: Default::default(),
        calendar_work: Default::default(),
        calendar_setup_lock: Default::default(),
        connection_lifecycle_lock: Default::default(),
        secret_remover: Arc::new(removals::OsSecretRemover::default()),
        outbound: Arc::new(providers::outgoing::Servers::default()),
        google_connection_lock: Default::default(),
        passphrases: Arc::new(backup::OsPassphraseStore::default()),
        restore_credentials: credentials,
        backup_uploads: Default::default(),
        mail_sync_settings: Default::default(),
        provider_slots: Default::default(),
        printing: Default::default(),
        bulk_control: Default::default(),
    }
}

#[tokio::test]
async fn discovered_caldav_ids_and_passwords_survive_encrypted_backup_and_restore() {
    let mut snapshot = fixture();
    let id = format!("caldav:{}", "a".repeat(64));
    snapshot.calendars[0].id = id.clone();
    snapshot
        .credentials
        .iter_mut()
        .find(|(owner, _)| owner == "home")
        .unwrap()
        .0 = id.clone();
    let passphrase = SecretString::from("Synthetic transfer passphrase");
    let ciphertext = backup::encrypt(&snapshot, &passphrase).unwrap();
    let decoded = backup::decrypt(&ciphertext, &passphrase).unwrap();
    let credentials = Arc::new(Credentials::default());
    let engine = engine(credentials.clone());
    let (mut output, _events) = futures::channel::mpsc::channel(32);
    engine.import_snapshot(decoded, &mut output).await.unwrap();
    assert!(
        engine
            .store
            .workspace()
            .await
            .unwrap()
            .calendars
            .iter()
            .any(|c| c.id == id)
    );
    assert_eq!(
        credentials.entries.lock().unwrap()[&id].expose_secret(),
        "calendar-original"
    );
    assert_eq!(
        engine.store.query(Default::default()).await.unwrap().total,
        2
    );
}

#[tokio::test]
async fn restored_mail_deleted_on_the_server_survives_sync_and_store_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("restore.sqlite");
    let store = Store::open(&path).unwrap();
    let snapshot = fixture();
    let ids: Vec<_> = snapshot
        .messages
        .iter()
        .map(|message| message.summary.id.clone())
        .collect();
    store.restore_snapshot(snapshot).await.unwrap();
    let normal = parse_mail(
        "work",
        "9:99",
        "INBOX",
        b"From: sam@example.com\r\nSubject: Normal sync\r\n\r\nOriginal".to_vec(),
        true,
        false,
    )
    .unwrap();
    store.upsert(vec![normal]).await.unwrap();
    drop(store);
    let store = Store::open(path).unwrap();
    store
        .apply_sync(MailSyncItem::Reconcile {
            account: "work".into(),
            folder: "INBOX".into(),
            live_ids: HashSet::new(),
        })
        .await
        .unwrap();
    assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 2);
    // One restored identity is now confirmed on the server, and follows its
    // lifecycle again. The other remains protected because it exists only here.
    store
        .apply_sync(MailSyncItem::Reconcile {
            account: "work".into(),
            folder: "INBOX".into(),
            live_ids: HashSet::from([ids[0].clone()]),
        })
        .await
        .unwrap();
    store
        .apply_sync(MailSyncItem::Reconcile {
            account: "work".into(),
            folder: "INBOX".into(),
            live_ids: HashSet::new(),
        })
        .await
        .unwrap();
    let page = store.query(MailQuery::default()).await.unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].id, ids[1]);
    store.remove(ids[1].clone()).await.unwrap();
    assert_eq!(
        store
            .run(|c| Ok(
                c.query_row("SELECT COUNT(*) FROM restored_messages", [], |r| r
                    .get::<_, i64>(0))?
            ))
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn restore_rejects_all_invalid_references_before_any_local_or_keychain_mutation() {
    type Mutation = fn(&mut Snapshot);
    let mutations: &[Mutation] = &[
        |s| {
            s.credentials
                .push(("google-oauth".into(), "replacement".into()))
        },
        |s| s.accounts[0].id = "google-oauth".into(),
        |s| s.accounts[0].id = "GOOGLE-OAUTH".into(),
        |s| s.accounts[0].id = "backup-passphrase".into(),
        |s| s.accounts[0].id = "backup-passphrase:drive-target".into(),
        |s| s.accounts[0].id = "home:smtp".into(),
        |s| {
            s.credentials
                .push(("unrelated".into(), "replacement".into()))
        },
        |s| {
            s.credentials
                .push(("google:work@example.com".into(), "token".into()))
        },
        |s| s.credentials.push(s.credentials[0].clone()),
        |s| s.accounts[0].smtp_separate_password = false,
        |s| s.calendars[0].id = "work".into(),
        |s| s.calendars[0].id = "WORK".into(),
        |s| s.calendars[0].url = "https://user:secret@example.com/dav".into(),
        |s| s.calendars[0].url = "http://calendar.example.com/dav".into(),
        |s| s.calendars[0].url.push_str("#fragment"),
        |s| s.calendars[1].id = "google:another".into(),
        |s| s.accounts.push(s.accounts[0].clone()),
        |s| s.messages.push(s.messages[0].clone()),
        |s| s.messages[0].raw.clear(),
        |s| s.messages[0].summary.folder.push('\n'),
        |s| s.messages[0].summary.timestamp = i64::MAX,
        |s| s.version = 99,
    ];
    for mutate in mutations {
        let credentials = Arc::new(Credentials::default());
        let engine = engine(credentials.clone());
        let mut snapshot = fixture();
        mutate(&mut snapshot);
        let (mut output, _) = futures::channel::mpsc::channel(32);
        assert!(engine.import_snapshot(snapshot, &mut output).await.is_err());
        assert!(engine.store.workspace().await.unwrap().accounts.is_empty());
        assert_eq!(
            engine
                .store
                .query(MailQuery::default())
                .await
                .unwrap()
                .total,
            0
        );
        assert!(credentials.calls.lock().unwrap().is_empty());
    }
    // An authenticated archive still needs semantic validation after decryption.
    let mut snapshot = fixture();
    snapshot
        .credentials
        .push(("google-oauth".into(), "replacement".into()));
    let passphrase = SecretString::from("a fixture restore passphrase");
    let encrypted = backup::encrypt(&snapshot, &passphrase).unwrap();
    assert!(backup::decrypt(&encrypted, &passphrase).is_err());
}

#[tokio::test]
async fn restore_rolls_back_accounts_calendars_mail_and_search_on_late_insert_failure() {
    let credentials = Arc::new(Credentials::default());
    let engine = engine(credentials.clone());
    let prefs = Preferences {
        google_client_id: "current-client".into(),
        ..Default::default()
    };
    engine.store.save_preferences(prefs.clone()).await.unwrap();
    engine.store.run(|c| {
        c.execute_batch("CREATE TRIGGER fail_restore BEFORE INSERT ON messages WHEN new.subject = 'Restored 1' BEGIN SELECT RAISE(ABORT, 'Fixture disk failure'); END;")?;
        Ok(())
    }).await.unwrap();
    let (mut output, _) = futures::channel::mpsc::channel(32);
    assert!(
        engine
            .import_snapshot(fixture(), &mut output)
            .await
            .is_err()
    );
    let workspace = engine.store.workspace().await.unwrap();
    assert!(workspace.accounts.is_empty() && workspace.calendars.is_empty());
    assert_eq!(workspace.preferences, prefs);
    assert_eq!(
        engine
            .store
            .query(MailQuery::default())
            .await
            .unwrap()
            .total,
        0
    );
    assert_eq!(
        engine
            .store
            .query(MailQuery {
                search: "Useful".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        0
    );
    assert!(credentials.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn restore_preserves_newer_mail_settings_and_drafts_and_rebuilds_mime_metadata() {
    let credentials = Arc::new(Credentials::default());
    let engine = engine(credentials.clone());
    let mut snapshot = fixture();
    let first = snapshot.messages[0].clone();
    engine.store.upsert(vec![first.clone()]).await.unwrap();
    let mut current = first.summary.clone();
    current.unread = false;
    current.starred = true;
    engine.store.flags(current.clone()).await.unwrap();
    engine
        .store
        .move_local(current.id.clone(), "Projects".into())
        .await
        .unwrap();
    let mut account = snapshot.accounts[0].clone();
    account.host = "new-server.example.com".into();
    engine.store.save_account(account.clone()).await.unwrap();
    let mut calendar = snapshot.calendars[0].clone();
    calendar.url = "https://new-calendar.example.com/dav/".into();
    engine.store.save_source(calendar.clone()).await.unwrap();
    let prefs = Preferences {
        appearance: Appearance::Dark,
        google_connection_id: "drive:current".into(),
        backup_ready: true,
        last_backup: Some(1788700010),
        ..Default::default()
    };
    engine
        .store
        .put("preferences", prefs.clone())
        .await
        .unwrap();
    engine
        .store
        .save_draft(Draft {
            id: "unsent".into(),
            body: "Keep this draft".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    // Snapshot display fields are not authoritative. Preserve a moved stable ID.
    snapshot.messages[1].summary.subject = "Forged cached subject".into();
    snapshot.messages[1].text = "Forged search index".into();
    snapshot.messages[1].summary.folder = "Archive".into();
    let restored_id = snapshot.messages[1].summary.id.clone();
    let (mut output, events) = futures::channel::mpsc::channel(32);
    engine.import_snapshot(snapshot, &mut output).await.unwrap();
    let workspace = engine.store.workspace().await.unwrap();
    assert_eq!(workspace.accounts, [account]);
    assert_eq!(workspace.calendars[0], calendar);
    assert_eq!(workspace.preferences, prefs);
    assert_eq!(workspace.preferences_revision, 1);
    assert_eq!(workspace.drafts[0].body, "Keep this draft");
    let kept = engine.store.detail(current.id).await.unwrap();
    assert_eq!(kept.summary.folder, "Projects");
    assert!(kept.summary.starred && !kept.summary.unread);
    let restored = engine.store.detail(restored_id).await.unwrap();
    assert_eq!(restored.summary.folder, "Archive");
    assert_eq!(restored.summary.subject, "Restored 1");
    assert_eq!(
        engine
            .store
            .query(MailQuery {
                search: "Forged".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        0
    );
    assert!(credentials.calls.lock().unwrap().is_empty());
    drop(output);
    assert!(events.collect::<Vec<_>>().await.iter().any(
        |event| matches!(event, Event::Notice(text) if text.contains("settings have changed"))
    ));
}

#[tokio::test]
async fn restore_conflicting_existing_owners_and_message_ids_roll_back_before_passwords() {
    for mail_conflict in [false, true] {
        let credentials = Arc::new(Credentials::default());
        let engine = engine(credentials.clone());
        let mut snapshot = fixture();
        if mail_conflict {
            let mut other = snapshot.messages[1].clone();
            other.summary.account_id = "another".into();
            engine.store.upsert(vec![other]).await.unwrap();
        } else {
            let mut calendar = snapshot.calendars[0].clone();
            calendar.id = "work".into();
            engine.store.save_source(calendar).await.unwrap();
        }
        // Valid independent archive; collision can only be detected in the store.
        snapshot.credentials.clear();
        let (mut output, _) = futures::channel::mpsc::channel(32);
        assert!(engine.import_snapshot(snapshot, &mut output).await.is_err());
        assert!(engine.store.workspace().await.unwrap().accounts.is_empty());
        assert!(credentials.calls.lock().unwrap().is_empty());
        assert_eq!(
            engine
                .store
                .query(MailQuery::default())
                .await
                .unwrap()
                .total,
            usize::from(mail_conflict)
        );
    }
}

#[tokio::test]
async fn encrypted_local_restore_retries_missing_passwords_without_replacing_current_secrets() {
    let credentials = Arc::new(Credentials::default());
    credentials
        .entries
        .lock()
        .unwrap()
        .insert("work".into(), SecretString::from("current-password"));
    credentials.entries.lock().unwrap().insert(
        "google-oauth".into(),
        SecretString::from("current-google-token"),
    );
    credentials
        .locked
        .lock()
        .unwrap()
        .insert("work:smtp".into());
    let engine = engine(credentials.clone());
    let dir = tempfile::tempdir().unwrap();
    let prefs = Preferences {
        backup_folder: dir.path().to_string_lossy().into(),
        ..Default::default()
    };
    engine.store.save_preferences(prefs.clone()).await.unwrap();
    let passphrase = SecretString::from("a fixture restore passphrase");
    let provider = backup::LocalBackup {
        directory: dir.path().into(),
    };
    let name = format!("shep-20260906T120000Z-{}.shepbackup", uuid::Uuid::new_v4());
    provider
        .upload(&name, backup::encrypt(&fixture(), &passphrase).unwrap())
        .await
        .unwrap();
    let target = BackupTarget::from_preferences(&prefs);
    let (mut output, mut events) = futures::channel::mpsc::channel(32);
    assert!(
        engine
            .run_restore(
                target.clone(),
                name.clone(),
                SecretString::from("incorrect passphrase"),
                &mut output
            )
            .await
            .is_err()
    );
    assert!(credentials.calls.lock().unwrap().is_empty());
    engine
        .run_restore(
            target.clone(),
            name.clone(),
            passphrase.clone(),
            &mut output,
        )
        .await
        .unwrap();
    assert!(matches!(events.next().await, Some(Event::Workspace(_))));
    assert!(matches!(events.next().await, Some(Event::Changed)));
    assert!(
        matches!(events.next().await, Some(Event::Error(text)) if text.starts_with("Backup restored: 2 emails added.") && text.contains("1 passwords could not"))
    );
    credentials.locked.lock().unwrap().clear();
    engine
        .run_restore(target, name, passphrase, &mut output)
        .await
        .unwrap();
    drop(output);
    assert!(events.collect::<Vec<_>>().await.iter().any(|event| matches!(event, Event::Notice(text) if text.starts_with("Backup restored: 0 emails added."))));
    assert_eq!(
        engine
            .store
            .query(MailQuery::default())
            .await
            .unwrap()
            .total,
        2
    );
    assert_eq!(engine.store.workspace().await.unwrap().accounts.len(), 1);
    let entries = credentials.entries.lock().unwrap();
    assert_eq!(entries["work"].expose_secret(), "current-password");
    assert_eq!(entries["work:smtp"].expose_secret(), "smtp-original");
    assert_eq!(entries["home"].expose_secret(), "calendar-original");
    assert_eq!(
        entries["google-oauth"].expose_secret(),
        "current-google-token"
    );
    assert!(
        !credentials
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|id| id == "google-oauth")
    );
}

#[tokio::test]
async fn explicit_backup_restore_reconnects_removed_owners_and_cancels_old_cleanup() {
    use crate::store::{ConnectionKind, ConnectionRef};
    let store = Store::memory().unwrap();
    store.restore_snapshot(fixture()).await.unwrap();
    for (kind, id) in [
        (ConnectionKind::Account, "work"),
        (ConnectionKind::Calendar, "home"),
        (ConnectionKind::Calendar, "google:work@example.com"),
    ] {
        let preview = store
            .removal_preview(ConnectionRef {
                kind,
                id: id.into(),
            })
            .await
            .unwrap();
        store.remove_connection(preview, false).await.unwrap();
    }
    assert_eq!(store.cleanup_jobs().await.unwrap().len(), 3);
    assert!(store.workspace().await.unwrap().accounts.is_empty());
    let restored = store.restore_snapshot(fixture()).await.unwrap();
    assert_eq!(restored.messages, 2);
    assert_eq!(restored.credentials.len(), 3);
    assert!(store.cleanup_jobs().await.unwrap().is_empty());
    assert!(store.removed_google_calendars().await.unwrap().is_empty());
    assert_eq!(store.workspace().await.unwrap().accounts.len(), 1);
    assert_eq!(store.workspace().await.unwrap().calendars.len(), 2);
    assert_eq!(store.export().await.unwrap().len(), 2);
    store
        .check_connection(ConnectionRef {
            kind: ConnectionKind::Account,
            id: "work".into(),
        })
        .await
        .unwrap();
}
