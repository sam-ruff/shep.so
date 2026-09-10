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
        calendar_setup: Default::default(),
        connection_lifecycle: Default::default(),
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

#[derive(Default)]
struct S3Vault(std::collections::HashMap<String, SecretString>);
impl crate::credentials::Backend for S3Vault {
    fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>> {
        Ok(self.0.get(key).cloned())
    }
    fn write(&mut self, key: &str, secret: SecretString) -> anyhow::Result<()> {
        self.0.insert(key.into(), secret);
        Ok(())
    }
    fn delete(&mut self, key: &str) -> anyhow::Result<()> {
        self.0.remove(key);
        Ok(())
    }
}

#[tokio::test]
async fn s3_connection_only_saves_verified_keys_and_rejects_a_destination_removed_during_test() {
    use crate::providers::test_http::{Reply, Server};
    let mut engine = engine(Arc::new(Secrets::default()));
    engine.credentials = crate::credentials::Credentials::with_backend(
        crate::credentials::Scope::Legacy,
        S3Vault::default(),
    );
    let prefs = Preferences {
        backup_destination: BackupDestination::S3,
        backup_s3: backup::s3::Settings {
            bucket: "fixture-backups".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let target = BackupTarget::from_preferences(&prefs);
    let id = prefs.backup_s3.identity().secret_id();
    engine.store.save_preferences(prefs.clone()).await.unwrap();
    let xml = "<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><EncodingType>url</EncodingType><IsTruncated>false</IsTruncated></ListBucketResult>";
    let mut server = Server::start(vec![Reply::new(200, xml)]).await;
    let mut endpoint = server.url.clone();
    endpoint.set_path("/");
    engine
        .connect_s3(
            &target,
            Some(("fixture-key".into(), "fixture-secret".into())),
            |settings, secret| {
                let mut settings = settings.clone();
                settings.endpoint = endpoint.to_string();
                backup::s3::S3Backup::fixture(&settings, secret)
            },
        )
        .await
        .unwrap();
    server.finish().await;
    let stored = engine.credentials.read(&id).await.unwrap();
    assert!(stored.expose_secret().contains("fixture-secret"));
    let mut failed = Server::start(vec![Reply::new(403, "private-error-body")]).await;
    let mut endpoint = failed.url.clone();
    endpoint.set_path("/");
    assert!(
        engine
            .connect_s3(
                &target,
                Some(("new-key".into(), "new-secret".into())),
                |settings, secret| {
                    let mut settings = settings.clone();
                    settings.endpoint = endpoint.to_string();
                    backup::s3::S3Backup::fixture(&settings, secret)
                }
            )
            .await
            .is_err()
    );
    failed.finish().await;
    assert_eq!(
        engine.credentials.read(&id).await.unwrap().expose_secret(),
        stored.expose_secret()
    );
    let (reply, observed, release) = Reply::new(200, xml).held();
    let mut held = Server::start(vec![reply]).await;
    let mut endpoint = held.url.clone();
    endpoint.set_path("/");
    let operation = engine.connect_s3(
        &target,
        Some(("new-key".into(), "new-secret".into())),
        |settings, secret| {
            let mut settings = settings.clone();
            settings.endpoint = endpoint.to_string();
            backup::s3::S3Backup::fixture(&settings, secret)
        },
    );
    let change = async {
        observed.await.unwrap();
        engine
            .store
            .save_preferences(Preferences::default())
            .await
            .unwrap();
        release.send(()).unwrap();
    };
    let (result, ()) = tokio::join!(operation, change);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("destination changed")
    );
    held.finish().await;
    assert_eq!(
        engine.credentials.read(&id).await.unwrap().expose_secret(),
        stored.expose_secret()
    );
}

#[tokio::test]
async fn sftp_verified_connection_stores_keys_only_after_success_and_keeps_later_settings() {
    use crate::backup::sftp::tests::wire_server::Fixture;
    let fixture = Fixture::start().await;
    let mut engine = engine(Arc::new(Secrets::default()));
    engine.credentials = crate::credentials::Credentials::with_backend(
        crate::credentials::Scope::Legacy,
        S3Vault::default(),
    );
    let prefs = Preferences {
        backup_destination: BackupDestination::Sftp,
        backup_sftp: fixture.settings.clone(),
        ..Default::default()
    };
    engine.store.save_preferences(prefs.clone()).await.unwrap();
    let target = BackupTarget::from_preferences(&prefs);
    let id = prefs.backup_sftp.secret_id();
    engine
        .connect_sftp(&target, Some("fixture-password".into()))
        .await
        .unwrap();
    assert_eq!(
        engine.credentials.read(&id).await.unwrap().expose_secret(),
        "fixture-password"
    );
    assert!(
        engine
            .connect_sftp(&target, Some("wrong-password".into()))
            .await
            .is_err()
    );
    assert_eq!(
        engine.credentials.read(&id).await.unwrap().expose_secret(),
        "fixture-password"
    );
    let (observed_tx, observed) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    fixture
        .files
        .run(|files| files.hold_list = Some((observed_tx, released)))
        .await;
    let work = engine.connect_sftp(&target, None);
    let changed = async {
        observed.await.unwrap();
        engine
            .store
            .save_preferences(Preferences::default())
            .await
            .unwrap();
        release.send(()).unwrap();
    };
    let (result, ()) = tokio::join!(work, changed);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("destination changed")
    );
    assert_eq!(
        engine.credentials.read(&id).await.unwrap().expose_secret(),
        "fixture-password"
    );
    fixture.finish().await;
}

#[tokio::test]
async fn ftp_verified_connection_stores_keys_only_after_success_and_keeps_later_settings() {
    use crate::backup::ftp::tests::wire_server::Fixture;
    let fixture = Fixture::start(crate::backup::ftp::Security::Plain).await;
    let mut engine = engine(Arc::new(Secrets::default()));
    engine.credentials = crate::credentials::Credentials::with_backend(
        crate::credentials::Scope::Legacy,
        S3Vault::default(),
    );
    let prefs = Preferences {
        backup_destination: BackupDestination::Ftp,
        backup_ftp: fixture.settings.clone(),
        ..Default::default()
    };
    engine.store.save_preferences(prefs.clone()).await.unwrap();
    let target = BackupTarget::from_preferences(&prefs);
    let id = prefs.backup_ftp.secret_id();
    engine
        .connect_ftp(&target, Some("fixture-password".into()))
        .await
        .unwrap();
    assert_eq!(
        engine.credentials.read(&id).await.unwrap().expose_secret(),
        "fixture-password"
    );
    assert!(
        engine
            .connect_ftp(&target, Some("wrong-password".into()))
            .await
            .is_err()
    );
    assert_eq!(
        engine.credentials.read(&id).await.unwrap().expose_secret(),
        "fixture-password"
    );
    let (observed_tx, observed) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    fixture
        .files
        .run(|files| files.hold_list = Some((observed_tx, released)))
        .await;
    let work = engine.connect_ftp(&target, None);
    let changed = async {
        observed.await.unwrap();
        engine
            .store
            .save_preferences(Preferences::default())
            .await
            .unwrap();
        release.send(()).unwrap();
    };
    let (result, ()) = tokio::join!(work, changed);
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("destination changed")
    );
    assert_eq!(
        engine.credentials.read(&id).await.unwrap().expose_secret(),
        "fixture-password"
    );
    fixture.finish().await;
}

#[tokio::test]
async fn backup_all_saved_secrets_work_without_enabling_schedules_and_fail_independently() {
    use backup::{PassphraseStore, config, run::Status};
    let secrets = Arc::new(Secrets::default());
    let engine = engine(secrets.clone());
    let directory = tempfile::tempdir().unwrap();
    let mut prefs = Preferences {
        backup_folder: directory.path().join("first").to_string_lossy().into(),
        ..Default::default()
    };
    config::add(&mut prefs).unwrap();
    prefs.backup_folder = directory.path().join("second").to_string_lossy().into();
    config::capture_editor(&mut prefs);
    let targets: Vec<_> = prefs
        .backup_destinations
        .iter()
        .map(|d| (d.id.clone(), d.target(&prefs)))
        .collect();
    engine.store.save_preferences(prefs).await.unwrap();
    for (_, target) in &targets {
        engine
            .store
            .record_backup(target.clone(), 1, true)
            .await
            .unwrap();
    }
    secrets.write(&targets[0].1, passphrase()).await.unwrap();
    // A missing second passphrase must not prevent the first from uploading.
    let (mut output, events) = futures::channel::mpsc::channel(32);
    let observer = tokio::spawn(async move { events.collect::<Vec<_>>().await });
    let mut second_output = output.clone();
    let (first, second) = tokio::join!(
        engine.backup_included(1, targets[0].0.clone(), targets[0].1.clone(), &mut output),
        engine.backup_included(
            2,
            targets[1].0.clone(),
            targets[1].1.clone(),
            &mut second_output
        )
    );
    first.unwrap();
    second.unwrap();
    drop(second_output);
    drop(output);
    let events = observer.await.unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::BackupRun(1, _, Status::Saved)))
    );
    assert!(events.iter().any(|e| matches!(e, Event::BackupRun(2, _, Status::Failed(message)) if message.contains("keychain"))));
    let provider = backup::LocalBackup {
        directory: directory.path().join("first"),
    };
    let copies = provider.list().await.unwrap();
    assert_eq!(copies.len(), 1);
    backup::decrypt(
        &provider.download(&copies[0].id).await.unwrap(),
        &passphrase(),
    )
    .unwrap();
    let second_secret = SecretString::from("separate second passphrase");
    secrets
        .write(&targets[1].1, second_secret.clone())
        .await
        .unwrap();
    let (mut output, events) = futures::channel::mpsc::channel(32);
    let observer = tokio::spawn(async move { events.collect::<Vec<_>>().await });
    engine
        .backup_included(3, targets[1].0.clone(), targets[1].1.clone(), &mut output)
        .await
        .unwrap();
    drop(output);
    assert!(
        observer
            .await
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::BackupRun(3, _, Status::Saved)))
    );
    assert_eq!(
        provider.list().await.unwrap()[0].id,
        copies[0].id,
        "Retry must not touch the successful target"
    );
    let provider = backup::LocalBackup {
        directory: directory.path().join("second"),
    };
    let copies = provider.list().await.unwrap();
    assert_eq!(copies.len(), 1);
    let bytes = provider.download(&copies[0].id).await.unwrap();
    backup::decrypt(&bytes, &second_secret).unwrap();
    assert!(backup::decrypt(&bytes, &passphrase()).is_err());
    let saved: Preferences = engine.store.get("preferences").await.unwrap();
    assert!(
        config::configurations(&saved)
            .iter()
            .all(|p| !p.auto_backup)
    );
}

#[test]
fn backup_all_rechecks_included_identity_and_first_copy_after_queueing() {
    let mut prefs = Preferences {
        backup_folder: "/first".into(),
        ..Default::default()
    };
    backup::config::add(&mut prefs).unwrap();
    let destination = prefs.backup_destinations[0].clone();
    let target = destination.target(&prefs);
    assert!(
        Engine::check_included(&prefs, &destination.id, &target)
            .unwrap_err()
            .to_string()
            .contains("first copy")
    );
    prefs.backup_destinations[0].ready = true;
    Engine::check_included(&prefs, &destination.id, &target).unwrap();
    prefs.backup_destinations[0].included = false;
    assert!(Engine::check_included(&prefs, &destination.id, &target).is_err());
    prefs.backup_destinations[0].included = true;
    prefs.backup_destinations[0].folder = "/changed".into();
    assert!(Engine::check_included(&prefs, &destination.id, &target).is_err());
    prefs.backup_destinations.remove(0);
    assert!(Engine::check_included(&prefs, &destination.id, &target).is_err());
}

#[tokio::test]
async fn backup_all_exclusion_while_keychain_waits_prevents_a_late_upload() {
    struct HeldSecrets(tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<SecretString>>);
    #[async_trait]
    impl backup::PassphraseStore for HeldSecrets {
        async fn read(&self, _: &BackupTarget) -> anyhow::Result<SecretString> {
            let (reply, result) = tokio::sync::oneshot::channel();
            self.0.send(reply).await?;
            Ok(result.await?)
        }
        async fn write(&self, _: &BackupTarget, _: SecretString) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let (queries, mut reads) = tokio::sync::mpsc::channel(1);
    let mut engine = engine(Arc::new(Secrets::default()));
    engine.passphrases = Arc::new(HeldSecrets(queries));
    let directory = tempfile::tempdir().unwrap();
    let mut prefs = Preferences {
        backup_folder: directory.path().join("first").to_string_lossy().into(),
        ..Default::default()
    };
    backup::config::add(&mut prefs).unwrap();
    let destination = prefs.backup_destinations[0].clone();
    let target = destination.target(&prefs);
    engine.store.save_preferences(prefs).await.unwrap();
    engine
        .store
        .record_backup(target.clone(), 1, true)
        .await
        .unwrap();
    let (mut output, events) = futures::channel::mpsc::channel(32);
    let observed = tokio::spawn(async move { events.collect::<Vec<_>>().await });
    let worker = engine.clone();
    let id = destination.id.clone();
    let target_clone = target.clone();
    let running = tokio::spawn(async move {
        worker
            .backup_included(1, id, target_clone, &mut output)
            .await
    });
    let reply = reads.recv().await.unwrap();
    engine
        .store
        .update_preferences(move |p| {
            p.backup_destinations
                .iter_mut()
                .find(|d| d.id == destination.id)
                .unwrap()
                .included = false
        })
        .await
        .unwrap();
    reply.send(passphrase()).unwrap();
    running.await.unwrap().unwrap();
    assert!(observed.await.unwrap().iter().any(|e| matches!(e, Event::BackupRun(1, _, backup::run::Status::Failed(error)) if error.contains("excluded"))));
    assert!(
        engine
            .backup_journal()
            .await
            .unwrap()
            .pending(&target)
            .await
            .unwrap()
            .is_none()
    );
    assert!(!directory.path().join("first").exists());
}

#[tokio::test]
async fn backup_format_plain_copies_and_schedules_never_need_a_keychain() {
    struct Unavailable;
    #[async_trait]
    impl backup::PassphraseStore for Unavailable {
        async fn read(&self, _: &BackupTarget) -> anyhow::Result<SecretString> {
            panic!("Unencrypted backups must not read a passphrase")
        }
        async fn write(&self, _: &BackupTarget, _: SecretString) -> anyhow::Result<()> {
            panic!("Unencrypted backups must not write a passphrase")
        }
    }
    let mut engine = engine(Arc::new(Secrets::default()));
    engine.passphrases = Arc::new(Unavailable);
    let directory = tempfile::tempdir().unwrap();
    let prefs = Preferences {
        backup_folder: directory.path().to_string_lossy().into(),
        auto_backup: true,
        backup_format: backup::format::Options {
            compression: backup::format::Compression::None,
            protection: backup::format::Protection::None,
        },
        ..Default::default()
    };
    let target = BackupTarget::from_preferences(&prefs);
    engine.store.save_preferences(prefs).await.unwrap();
    let (mut output, events) = futures::channel::mpsc::channel(32);
    let observed = tokio::spawn(async move { events.collect::<Vec<_>>().await });
    engine
        .run_backup(target.clone(), Some("".into()), &mut output)
        .await
        .unwrap();
    let saved: Preferences = engine.store.get("preferences").await.unwrap();
    assert!(saved.backup_ready && saved.auto_backup);
    engine.run_backup(target, None, &mut output).await.unwrap();
    drop(output);
    assert!(
        observed
            .await
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::Notice(message) if message == "Unencrypted backup saved."))
    );
    let provider = backup::LocalBackup {
        directory: directory.path().into(),
    };
    let copies = provider.list().await.unwrap();
    assert_eq!(copies.len(), 2);
    let bytes = provider.download(&copies[0].id).await.unwrap();
    let restored = backup::format::decode(&bytes, None).unwrap();
    assert!(restored.credentials.is_empty());
    assert!(!backup::format::options(&bytes).unwrap().encrypted());
}

#[tokio::test]
async fn backup_format_pending_plain_bytes_cannot_be_relabelled_encrypted_by_later_settings() {
    let secrets = Arc::new(Secrets::default());
    let engine = engine(secrets.clone());
    let directory = tempfile::tempdir().unwrap();
    let format = backup::format::Options {
        compression: backup::format::Compression::None,
        protection: backup::format::Protection::None,
    };
    let mut prefs = Preferences {
        backup_folder: directory.path().to_string_lossy().into(),
        backup_format: format,
        ..Default::default()
    };
    let target = BackupTarget::from_preferences(&prefs);
    engine.store.save_preferences(prefs.clone()).await.unwrap();
    let provider = backup::LocalBackup {
        directory: directory.path().into(),
    };
    let name = format!("shep-20260909T120000Z-{}.shepbackup", uuid::Uuid::new_v4());
    let snapshot = Snapshot {
        version: 1,
        created_at: 1,
        messages: vec![],
        accounts: vec![],
        calendars: vec![],
        preferences: prefs.clone(),
        credentials: vec![],
    };
    let original = backup::format::encode(&snapshot, format, None).unwrap();
    let journal = engine.backup_journal().await.unwrap();
    journal
        .prepare(
            &target,
            provider.reserve(&name, &original).await.unwrap(),
            original.clone(),
        )
        .await
        .unwrap();
    prefs.backup_format = Default::default();
    engine.store.save_preferences(prefs).await.unwrap();
    let (mut output, events) = futures::channel::mpsc::channel(32);
    let observed = tokio::spawn(async move { events.collect::<Vec<_>>().await });
    engine
        .run_backup(target.clone(), Some(passphrase()), &mut output)
        .await
        .unwrap();
    assert_eq!(provider.download(&name).await.unwrap(), original);
    assert!(secrets.entries.lock().unwrap().is_empty());
    let saved: Preferences = engine.store.get("preferences").await.unwrap();
    assert_eq!(saved.backup_format, backup::format::Options::default());
    assert!(
        !saved.backup_ready,
        "the old plaintext receipt must not authorize encrypted schedules"
    );
    engine
        .run_backup(target, Some(passphrase()), &mut output)
        .await
        .unwrap();
    drop(output);
    observed.await.unwrap();
    assert!(
        engine
            .store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .backup_ready
    );
    assert_eq!(provider.list().await.unwrap().len(), 2);
    assert_eq!(provider.download(&name).await.unwrap(), original);
}

#[tokio::test]
async fn backup_format_pending_encrypted_copy_uses_original_key_after_encryption_disabled() {
    let secrets = Arc::new(Secrets::default());
    let engine = engine(secrets.clone());
    let directory = tempfile::tempdir().unwrap();
    let mut prefs = Preferences {
        backup_folder: directory.path().to_string_lossy().into(),
        ..Default::default()
    };
    let target = BackupTarget::from_preferences(&prefs);
    engine.store.save_preferences(prefs.clone()).await.unwrap();
    let provider = backup::LocalBackup {
        directory: directory.path().into(),
    };
    let name = format!("shep-20260909T120000Z-{}.shepbackup", uuid::Uuid::new_v4());
    let snapshot = Snapshot {
        version: 1,
        created_at: 1,
        messages: vec![],
        accounts: vec![],
        calendars: vec![],
        preferences: prefs.clone(),
        credentials: vec![],
    };
    let original =
        backup::format::encode(&snapshot, prefs.backup_format, Some(&passphrase())).unwrap();
    engine
        .backup_journal()
        .await
        .unwrap()
        .prepare(
            &target,
            provider.reserve(&name, &original).await.unwrap(),
            original.clone(),
        )
        .await
        .unwrap();
    prefs.backup_format.protection = backup::format::Protection::None;
    engine.store.save_preferences(prefs).await.unwrap();
    let (mut output, events) = futures::channel::mpsc::channel(32);
    let observed = tokio::spawn(async move { events.collect::<Vec<_>>().await });
    // Missing original key is a recoverable error, never a new plaintext upload.
    assert!(
        engine
            .run_backup(target.clone(), Some("".into()), &mut output)
            .await
            .is_err()
    );
    assert!(provider.list().await.unwrap().is_empty());
    backup::PassphraseStore::write(secrets.as_ref(), &target, passphrase())
        .await
        .unwrap();
    engine
        .run_backup(target.clone(), Some("".into()), &mut output)
        .await
        .unwrap();
    assert_eq!(provider.download(&name).await.unwrap(), original);
    assert!(
        !engine
            .store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .backup_ready
    );
    engine
        .run_backup(target, Some("".into()), &mut output)
        .await
        .unwrap();
    assert!(
        engine
            .store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .backup_ready
    );
    assert_eq!(provider.list().await.unwrap().len(), 2);
    assert_eq!(provider.download(&name).await.unwrap(), original);
    drop(output);
    observed.await.unwrap();
}

#[tokio::test]
async fn backup_history_keychain_warning_retains_confirmed_copy_then_retry_reuses_it() {
    use backup::history::Outcome;
    let secrets = Arc::new(Secrets::default());
    secrets.fail_write.store(true, Ordering::Relaxed);
    let engine = engine(secrets.clone());
    let directory = tempfile::tempdir().unwrap();
    let prefs = Preferences {
        backup_folder: directory.path().to_string_lossy().into(),
        ..Default::default()
    };
    let target = BackupTarget::from_preferences(&prefs);
    engine.store.save_preferences(prefs).await.unwrap();
    let (mut output, events) = futures::channel::mpsc::channel(32);
    let observed = tokio::spawn(async move { events.collect::<Vec<_>>().await });
    engine
        .run_backup(target.clone(), Some(passphrase()), &mut output)
        .await
        .unwrap();
    let rows = engine.store.backup_history(target.clone()).await.unwrap();
    assert_eq!(rows[0].outcome, Outcome::SavedWithWarning);
    assert!(rows[0].detail.contains("OS keychain"));
    let provider = backup::LocalBackup {
        directory: directory.path().into(),
    };
    let copies = provider.list().await.unwrap();
    assert_eq!(copies.len(), 1);
    let original = provider.download(&copies[0].id).await.unwrap();
    secrets.fail_write.store(false, Ordering::Relaxed);
    engine
        .execute(
            Command::RetryBackupHistory(rows[0].id.clone(), target.clone(), passphrase()),
            output.clone(),
        )
        .await
        .unwrap();
    let latest = engine.store.backup_history(target.clone()).await.unwrap();
    assert_eq!(latest[0].outcome, Outcome::Saved);
    assert_eq!(latest[0].copy, rows[0].copy);
    assert_eq!(provider.list().await.unwrap().len(), 1);
    assert_eq!(provider.download(&copies[0].id).await.unwrap(), original);
    // A stale Retry cannot start a new archive after a newer attempt succeeded.
    assert!(
        engine
            .execute(
                Command::RetryBackupHistory(rows[0].id.clone(), target, passphrase()),
                output.clone()
            )
            .await
            .is_err()
    );
    assert_eq!(provider.list().await.unwrap().len(), 1);
    drop(output);
    observed.await.unwrap();
}

#[tokio::test]
async fn backup_history_imported_or_missing_receipt_cannot_start_a_duplicate_copy() {
    let engine = engine(Arc::new(Secrets::default()));
    let directory = tempfile::tempdir().unwrap();
    let prefs = Preferences {
        backup_folder: directory.path().to_string_lossy().into(),
        ..Default::default()
    };
    let target = BackupTarget::from_preferences(&prefs);
    engine.store.save_preferences(prefs).await.unwrap();
    let mut row =
        backup::history::Entry::new(target.clone(), "Imported device".into(), Default::default());
    row.copy = Some("another-device-reserved-copy".into());
    row.outcome = backup::history::Outcome::NeedsReview;
    engine
        .store
        .write_backup_history(row.clone())
        .await
        .unwrap();
    let (output, _events) = futures::channel::mpsc::channel(32);
    let error = engine
        .execute(
            Command::RetryBackupHistory(row.id.clone(), target.clone(), passphrase()),
            output,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("no matching pending upload"));
    assert_eq!(
        engine.store.backup_history(target).await.unwrap()[0].id,
        row.id
    );
    assert!(
        std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .is_none()
    );
}
