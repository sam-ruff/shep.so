use super::*;
use async_trait::async_trait;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct Secrets {
    entries: Mutex<Vec<(BackupTarget, SecretString)>>,
    fail_write: AtomicBool,
}
#[async_trait]
impl backup::PassphraseStore for Secrets {
    async fn read(&self, target: &BackupTarget) -> anyhow::Result<SecretString> {
        self.entries
            .lock()
            .unwrap()
            .iter()
            .find(|(key, _)| key == target)
            .map(|(_, secret)| secret.clone())
            .context("No saved passphrase")
    }
    async fn write(&self, target: &BackupTarget, secret: SecretString) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.fail_write.load(Ordering::Relaxed),
            "Keychain unavailable"
        );
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|(key, _)| key != target);
        entries.push((target.clone(), secret));
        Ok(())
    }
}
fn engine(secrets: Arc<Secrets>) -> Engine {
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
        passphrases: secrets,
        restore_credentials: Arc::new(backup::restore::OsCredentialRestorer::default()),
        backup_uploads: Default::default(),
        mail_sync_settings: Default::default(),
        provider_slots: Default::default(),
        printing: Default::default(),
        bulk_control: Default::default(),
    }
}
fn passphrase() -> SecretString {
    SecretString::from("a test backup passphrase")
}

#[tokio::test]
async fn manual_copy_prepares_automatic_backup_even_when_schedule_starts_disabled() {
    let secrets = Arc::new(Secrets::default());
    let engine = engine(secrets.clone());
    let directory = tempfile::tempdir().unwrap();
    let prefs = Preferences {
        backup_folder: directory.path().to_string_lossy().into(),
        ..Default::default()
    };
    engine.store.save_preferences(prefs.clone()).await.unwrap();
    let target = BackupTarget::from_preferences(&prefs);
    let (mut output, mut events) = futures::channel::mpsc::channel(32);
    engine
        .run_backup(target.clone(), Some(passphrase()), &mut output)
        .await
        .unwrap();
    assert!(matches!(
        events.next().await,
        Some(Event::BackupSaved(_, _))
    ));
    let saved = engine
        .store
        .get::<Preferences>("preferences")
        .await
        .unwrap();
    assert!(saved.backup_ready && saved.last_backup.is_some());
    assert!(!saved.auto_backup);
    assert_eq!(secrets.entries.lock().unwrap()[0].0, target);
    engine
        .store
        .save_preferences(Preferences {
            auto_backup: true,
            ..saved
        })
        .await
        .unwrap();
    engine.run_backup(target, None, &mut output).await.unwrap();
    let provider = backup::LocalBackup {
        directory: directory.path().into(),
    };
    let copies = provider.list().await.unwrap();
    assert_eq!(copies.len(), 2);
    for copy in copies {
        backup::decrypt(&provider.download(&copy.id).await.unwrap(), &passphrase()).unwrap();
    }
}

#[tokio::test]
async fn keychain_failure_keeps_the_committed_copy_and_explains_schedule_setup() {
    let secrets = Arc::new(Secrets::default());
    secrets.fail_write.store(true, Ordering::Relaxed);
    let engine = engine(secrets);
    let directory = tempfile::tempdir().unwrap();
    let prefs = Preferences {
        backup_folder: directory.path().to_string_lossy().into(),
        auto_backup: true,
        ..Default::default()
    };
    engine.store.save_preferences(prefs.clone()).await.unwrap();
    let (mut output, events) = futures::channel::mpsc::channel(32);
    engine
        .run_backup(
            BackupTarget::from_preferences(&prefs),
            Some(passphrase()),
            &mut output,
        )
        .await
        .unwrap();
    drop(output);
    let events: Vec<_> = events.collect().await;
    assert!(matches!(events.first(), Some(Event::BackupSaved(..))));
    assert!(events.iter().any(|e| matches!(e, Event::Error(message) if message.starts_with("Encrypted backup saved.") && message.contains("OS keychain"))));
    let saved = engine
        .store
        .get::<Preferences>("preferences")
        .await
        .unwrap();
    assert!(saved.last_backup.is_some());
    assert!(!saved.backup_ready);
    assert_eq!(
        backup::LocalBackup {
            directory: directory.path().into()
        }
        .list()
        .await
        .unwrap()
        .len(),
        1
    );
}

struct FailedListing;
#[async_trait]
impl BackupProvider for FailedListing {
    async fn list(&self) -> anyhow::Result<Vec<BackupCopy>> {
        anyhow::bail!("Drive listing unavailable")
    }
    async fn upload(&self, _: &str, _: Vec<u8>) -> anyhow::Result<String> {
        panic!("must not upload again")
    }
    async fn download(&self, _: &str) -> anyhow::Result<Vec<u8>> {
        unreachable!()
    }
    async fn delete(&self, _: &str) -> anyhow::Result<()> {
        panic!("must not delete after failed listing")
    }
}
fn copy() -> BackupCopy {
    BackupCopy {
        id: "committed".into(),
        name: "saved-copy".into(),
        created_at: "2026-09-06".into(),
    }
}

#[tokio::test]
async fn committed_backup_survives_retention_failure_and_destination_change() {
    let engine = engine(Arc::new(Secrets::default()));
    let old = Preferences {
        backup_folder: "/first".into(),
        ..Default::default()
    };
    engine.store.save_preferences(old.clone()).await.unwrap();
    engine
        .store
        .save_preferences(Preferences {
            backup_folder: "/second".into(),
            backup_copies: 17,
            reader_split: 0.6,
            ..old.clone()
        })
        .await
        .unwrap();
    let (mut output, events) = futures::channel::mpsc::channel(32);
    engine
        .finish_backup(
            &FailedListing,
            &old,
            BackupTarget::from_preferences(&old),
            copy(),
            passphrase(),
            &mut output,
        )
        .await
        .unwrap();
    drop(output);
    let events: Vec<_> = events.collect().await;
    assert!(
        matches!(events.first(), Some(Event::BackupSaved(BackupTarget::Local(folder), _)) if folder == "/first")
    );
    assert!(events.iter().any(|e| matches!(e, Event::Error(message) if message.starts_with("Encrypted backup saved.") && message.contains("cleanup"))));
    let saved = engine
        .store
        .get::<Preferences>("preferences")
        .await
        .unwrap();
    assert_eq!(saved.backup_folder, "/second");
    assert_eq!(saved.backup_copies, 17);
    assert_eq!(saved.reader_split, 0.6);
    assert!(saved.last_backup.is_none() && !saved.backup_ready);
}

#[tokio::test]
async fn committed_backup_is_acknowledged_even_when_local_metadata_cannot_be_saved() {
    let engine = engine(Arc::new(Secrets::default()));
    engine
        .store
        .run(|c| {
            c.execute_batch("DROP TABLE kv")?;
            Ok(())
        })
        .await
        .unwrap();
    let prefs = Preferences::default();
    let (mut output, events) = futures::channel::mpsc::channel(32);
    engine
        .finish_backup(
            &FailedListing,
            &prefs,
            BackupTarget::from_preferences(&prefs),
            copy(),
            passphrase(),
            &mut output,
        )
        .await
        .unwrap();
    drop(output);
    let events: Vec<_> = events.collect().await;
    assert!(matches!(events.first(), Some(Event::BackupSaved(..))));
    assert!(events.iter().any(|e| matches!(e, Event::Error(message) if message.starts_with("Encrypted backup saved.") && message.contains("record backup history"))));
}

#[tokio::test]
async fn unavailable_automatic_passphrase_pauses_with_recovery_and_never_creates_a_copy() {
    let engine = engine(Arc::new(Secrets::default()));
    let directory = tempfile::tempdir().unwrap();
    let prefs = Preferences {
        backup_folder: directory.path().to_string_lossy().into(),
        auto_backup: true,
        ..Default::default()
    };
    engine.store.save_preferences(prefs.clone()).await.unwrap();
    let target = BackupTarget::from_preferences(&prefs);
    engine
        .store
        .record_backup(target.clone(), 1, true)
        .await
        .unwrap();
    // Keep the receiver alive while the worker sends updated preferences.
    let (mut output, _events) = futures::channel::mpsc::channel(32);
    let error = engine
        .run_backup(target, None, &mut output)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Back up now"));
    assert!(
        !engine
            .store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .backup_ready
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    // A command queued for another destination must fail before reading secrets.
    let error = engine
        .run_backup(
            BackupTarget::Local("/old".into()),
            Some(passphrase()),
            &mut output,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("destination changed"));
}

#[tokio::test]
async fn engine_recovers_lost_local_commit_after_reopen_without_reencrypting_or_duplicating() {
    let directory = tempfile::tempdir().unwrap();
    let store_path = directory.path().join("mail.sqlite");
    let backup_path = directory.path().join("copies");
    let secrets = Arc::new(Secrets::default());
    let prefs = Preferences {
        backup_folder: backup_path.to_string_lossy().into(),
        ..Default::default()
    };
    let target = BackupTarget::from_preferences(&prefs);
    let provider = backup::LocalBackup {
        directory: backup_path.clone(),
    };
    let archive = backup::encrypt(
        &Snapshot {
            version: 1,
            created_at: 1,
            messages: Vec::new(),
            accounts: Vec::new(),
            calendars: Vec::new(),
            preferences: prefs.clone(),
            credentials: Vec::new(),
        },
        &passphrase(),
    )
    .unwrap();
    let filename = format!("shep-20260906T120000Z-{}.shepbackup", uuid::Uuid::nil());
    {
        let mut first = engine(secrets.clone());
        first.store = Store::open(&store_path).unwrap();
        first.store.save_preferences(prefs.clone()).await.unwrap();
        let journal = first.backup_journal().await.unwrap();
        journal
            .prepare(
                &target,
                provider.reserve(&filename, &archive).await.unwrap(),
                archive.clone(),
            )
            .await
            .unwrap();
        provider.upload(&filename, archive.clone()).await.unwrap();
        assert!(directory.path().join("backup-uploads.sqlite").exists());
        assert!(
            !first
                .store
                .run(|connection| Ok(connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='uploads')",
                    [],
                    |row| row.get::<_, bool>(0)
                )?))
                .await
                .unwrap()
        );
    }
    let mut resumed = engine(secrets.clone());
    resumed.store = Store::open(&store_path).unwrap();
    // New mail arrives after the staged snapshot. Recovery must use the exact
    // original ciphertext, then allow a later backup to include that new mail.
    resumed
        .store
        .upsert(vec![
            parse_mail(
                "fixture",
                "1",
                "INBOX",
                b"From: a@example.com\r\nSubject: New after interruption\r\n\r\nnew mail".to_vec(),
                true,
                false,
            )
            .unwrap(),
        ])
        .await
        .unwrap();
    let (mut output, _events) = futures::channel::mpsc::channel(32);
    assert!(
        resumed
            .run_backup(
                target.clone(),
                Some(SecretString::from("a different passphrase")),
                &mut output
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("original passphrase")
    );
    assert!(secrets.entries.lock().unwrap().is_empty());
    resumed
        .run_backup(target.clone(), Some(passphrase()), &mut output)
        .await
        .unwrap();
    assert_eq!(provider.list().await.unwrap().len(), 1);
    assert_eq!(provider.download(&filename).await.unwrap(), archive);
    assert!(
        resumed
            .backup_journal()
            .await
            .unwrap()
            .pending(&target)
            .await
            .unwrap()
            .is_none()
    );
    resumed
        .run_backup(target, Some(passphrase()), &mut output)
        .await
        .unwrap();
    let copies = provider.list().await.unwrap();
    assert_eq!(copies.len(), 2);
    let new = copies.iter().find(|copy| copy.id != filename).unwrap();
    let snapshot =
        backup::decrypt(&provider.download(&new.id).await.unwrap(), &passphrase()).unwrap();
    assert_eq!(snapshot.messages.len(), 1);
}

#[tokio::test]
async fn retry_after_postcommit_failure_finishes_setup_without_uploading_another_copy() {
    let secrets = Arc::new(Secrets::default());
    secrets.fail_write.store(true, Ordering::Relaxed);
    let engine = engine(secrets.clone());
    let directory = tempfile::tempdir().unwrap();
    let prefs = Preferences {
        backup_folder: directory.path().to_string_lossy().into(),
        ..Default::default()
    };
    engine.store.save_preferences(prefs.clone()).await.unwrap();
    let target = BackupTarget::from_preferences(&prefs);
    let (mut output, _events) = futures::channel::mpsc::channel(32);
    engine
        .run_backup(target.clone(), Some(passphrase()), &mut output)
        .await
        .unwrap();
    let journal = engine.backup_journal().await.unwrap();
    let id = journal.pending(&target).await.unwrap().unwrap().upload.id;
    assert!(journal.pending(&target).await.unwrap().unwrap().committed);
    secrets.fail_write.store(false, Ordering::Relaxed);
    engine
        .run_backup(target.clone(), Some(passphrase()), &mut output)
        .await
        .unwrap();
    let provider = backup::LocalBackup {
        directory: directory.path().into(),
    };
    let copies = provider.list().await.unwrap();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].id, id);
    assert!(journal.pending(&target).await.unwrap().is_none());
    assert!(
        engine
            .store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .backup_ready
    );
}

#[tokio::test]
async fn in_memory_test_workspaces_have_independent_upload_journals() {
    let first = engine(Arc::new(Secrets::default()));
    let second = engine(Arc::new(Secrets::default()));
    let target = BackupTarget::Local("same-fixture-target".into());
    let upload = backup::PreparedUpload::new(
        "one".into(),
        format!("shep-20260906T120000Z-{}.shepbackup", uuid::Uuid::nil()),
        b"fixture",
    );
    first
        .backup_journal()
        .await
        .unwrap()
        .prepare(&target, upload, b"fixture".to_vec())
        .await
        .unwrap();
    assert!(
        second
            .backup_journal()
            .await
            .unwrap()
            .pending(&target)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn multiple_backup_targets_upload_and_retain_independently_with_distinct_passphrases() {
    use backup::config;
    let secrets = Arc::new(Secrets::default());
    let engine = engine(secrets.clone());
    let directory = tempfile::tempdir().unwrap();
    let mut prefs = Preferences {
        backup_folder: directory.path().join("one").to_string_lossy().into(),
        backup_copies: 1,
        auto_backup: true,
        ..Default::default()
    };
    let first = BackupTarget::from_preferences(&prefs);
    config::add(&mut prefs).unwrap();
    prefs.backup_folder = directory.path().join("two").to_string_lossy().into();
    prefs.backup_copies = 2;
    prefs.auto_backup = true;
    config::capture_editor(&mut prefs);
    let second = BackupTarget::from_preferences(&prefs);
    engine.store.save_preferences(prefs).await.unwrap();
    let second_secret = SecretString::from("another isolated passphrase");
    let (mut output, events) = futures::channel::mpsc::channel(32);
    let observer = tokio::spawn(async move { events.collect::<Vec<_>>().await });
    // Both destinations are admitted concurrently, through independent journal rows.
    let mut other_output = output.clone();
    let (one, two) = tokio::join!(
        engine.run_backup(first.clone(), Some(passphrase()), &mut output),
        engine.run_backup(
            second.clone(),
            Some(second_secret.clone()),
            &mut other_output
        )
    );
    one.unwrap();
    two.unwrap();
    drop(other_output);
    engine
        .run_backup(first.clone(), None, &mut output)
        .await
        .unwrap();
    engine
        .run_backup(second.clone(), None, &mut output)
        .await
        .unwrap();
    engine
        .run_backup(second.clone(), None, &mut output)
        .await
        .unwrap();
    let saved: Preferences = engine.store.get("preferences").await.unwrap();
    let first_provider = backup::LocalBackup {
        directory: directory.path().join("one"),
    };
    let second_provider = backup::LocalBackup {
        directory: directory.path().join("two"),
    };
    let copies = first_provider.list().await.unwrap();
    assert_eq!(copies.len(), 1);
    let bytes = first_provider.download(&copies[0].id).await.unwrap();
    backup::decrypt(&bytes, &passphrase()).unwrap();
    assert!(backup::decrypt(&bytes, &second_secret).is_err());
    let copies = second_provider.list().await.unwrap();
    assert_eq!(copies.len(), 2);
    for copy in copies {
        let bytes = second_provider.download(&copy.id).await.unwrap();
        backup::decrypt(&bytes, &second_secret).unwrap();
        assert!(backup::decrypt(&bytes, &passphrase()).is_err());
    }
    assert!(config::resolve(&saved, &first).unwrap().backup_ready);
    assert!(config::resolve(&saved, &second).unwrap().backup_ready);
    assert_eq!(secrets.entries.lock().unwrap().len(), 2);
    // Missing one passphrase pauses only its schedule, without blocking the other.
    secrets
        .entries
        .lock()
        .unwrap()
        .retain(|(target, _)| *target != first);
    assert!(
        engine
            .run_backup(first.clone(), None, &mut output)
            .await
            .is_err()
    );
    let saved: Preferences = engine.store.get("preferences").await.unwrap();
    assert!(!config::resolve(&saved, &first).unwrap().backup_ready);
    assert!(config::resolve(&saved, &second).unwrap().backup_ready);
    engine
        .run_backup(second.clone(), None, &mut output)
        .await
        .unwrap();
    drop(output);
    let events = observer.await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::BackupSaved(..)))
            .count(),
        6
    );
    let journal = engine.backup_journal().await.unwrap();
    assert!(journal.pending(&first).await.unwrap().is_none());
    assert!(journal.pending(&second).await.unwrap().is_none());
}
