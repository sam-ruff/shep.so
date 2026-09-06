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
        store: Store::memory().unwrap(),
        google: Default::default(),
        demo: false,
        account_locks: Default::default(),
        calendar_locks: Default::default(),
        google_connection_lock: Default::default(),
        passphrases: secrets,
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
