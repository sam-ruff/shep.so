use super::*;
#[path = "ftp_test_server.rs"]
pub(crate) mod wire_server;
const NAME: &str = "shep-20260909T120000Z-11111111-1111-4111-8111-111111111111.shepbackup";

#[tokio::test]
async fn ftp_wire_roundtrip_with_owned_directory_and_rolling_retention() {
    let fixture = wire_server::Fixture::start(Security::Plain).await;
    let provider = fixture.provider();
    provider.test_connection().await.unwrap();
    let bytes = [MAGIC.as_slice(), b"encrypted fixture archive"].concat();
    provider.upload(NAME, bytes.clone()).await.unwrap();
    assert_eq!(provider.download(NAME).await.unwrap(), bytes);
    assert_eq!(provider.list().await.unwrap().len(), 1);
    provider.delete(NAME).await.unwrap();
    assert!(provider.list().await.unwrap().is_empty());
    fixture.finish().await;
}
#[tokio::test]
async fn ftps_wire_explicit_and_implicit_tls_verify_control_and_data_connections() {
    for security in [Security::ExplicitTls, Security::ImplicitTls] {
        let fixture = wire_server::Fixture::start(security).await;
        let provider = fixture.provider();
        provider.test_connection().await.unwrap();
        provider
            .upload(NAME, [MAGIC.as_slice(), b"encrypted fixture"].concat())
            .await
            .unwrap();
        assert_eq!(provider.list().await.unwrap().len(), 1);
        assert_eq!(fixture.files.run(|f| f.clear_passwords).await, 0);
        let authenticated = fixture.files.run(|f| f.passwords).await;
        let rejected = FtpBackup::new(&fixture.settings, "fixture-password".into()).unwrap();
        assert!(rejected.test_connection().await.is_err());
        assert_eq!(fixture.files.run(|f| f.passwords).await, authenticated);
        fixture.finish().await;
    }
}

#[tokio::test]
async fn ftp_wire_interrupted_archive_and_commit_resume_only_owned_prefixes() {
    for commit in [false, true] {
        let fixture = wire_server::Fixture::start(Security::Plain).await;
        fixture
            .files
            .run(move |f| {
                if commit {
                    f.partial_commit = true;
                } else {
                    f.partial_archive = true;
                }
            })
            .await;
        let provider = fixture.provider();
        let bytes = [MAGIC.as_slice(), &[7; 90]].concat();
        let mut upload = provider.reserve(NAME, &bytes).await.unwrap();
        assert!(
            provider
                .upload_prepared(&mut upload, &bytes, &super::super::NoCheckpoint)
                .await
                .is_err()
        );
        assert!(upload.session.is_some());
        let mut upload: PreparedUpload =
            serde_json::from_slice(&serde_json::to_vec(&upload).unwrap()).unwrap();
        provider
            .upload_prepared(&mut upload, &bytes, &super::super::NoCheckpoint)
            .await
            .unwrap();
        assert_eq!(provider.download(NAME).await.unwrap(), bytes);
        assert_eq!(
            fixture
                .files
                .run(|f| f.commands.iter().filter(|c| *c == "MKD").count())
                .await,
            1
        );
        assert!(
            fixture
                .files
                .run(|f| f.commands.iter().any(|c| c == "APPE"))
                .await
        );
        fixture.finish().await;
    }
}
#[tokio::test]
async fn ftp_wire_lost_commit_and_journal_restart_recover_without_another_write() {
    use crate::backup::{
        BackupTarget,
        journal::{Checkpoint, Journal},
    };
    let fixture = wire_server::Fixture::start(Security::Plain).await;
    let target = BackupTarget::Ftp(fixture.settings.identity());
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("journal.sqlite");
    let provider = fixture.provider();
    let bytes = [MAGIC.as_slice(), b"encrypted fixture"].concat();
    {
        let journal = Journal::open(Some(&path)).unwrap();
        let mut pending = journal
            .prepare(
                &target,
                provider.reserve(NAME, &bytes).await.unwrap(),
                bytes.clone(),
            )
            .await
            .unwrap();
        fixture.files.run(|f| f.lose_commit = true).await;
        assert!(
            provider
                .upload_prepared(
                    &mut pending.upload,
                    &pending.data,
                    &Checkpoint {
                        journal: journal.clone(),
                        target: target.clone()
                    }
                )
                .await
                .is_err()
        );
    }
    let writes = fixture
        .files
        .run(|f| {
            f.commands
                .iter()
                .filter(|c| matches!(c.as_str(), "STOR" | "APPE"))
                .count()
        })
        .await;
    let journal = Journal::open(Some(&path)).unwrap();
    let mut pending = journal.pending(&target).await.unwrap().unwrap();
    provider
        .upload_prepared(
            &mut pending.upload,
            &pending.data,
            &Checkpoint {
                journal: journal.clone(),
                target: target.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        fixture
            .files
            .run(|f| f
                .commands
                .iter()
                .filter(|c| matches!(c.as_str(), "STOR" | "APPE"))
                .count())
            .await,
        writes
    );
    journal.committed(&target, NAME).await.unwrap();
    journal.remove(&target, NAME).await.unwrap();
    assert!(journal.pending(&target).await.unwrap().is_none());
    fixture.finish().await;
}
#[tokio::test]
async fn ftp_wire_unconfirmed_folder_and_foreign_files_are_never_overwritten() {
    let fixture = wire_server::Fixture::start(Security::Plain).await;
    fixture
        .files
        .run(|f| {
            f.dirs.insert(format!("/archive/{NAME}"));
            f.data.insert(
                format!("/archive/{NAME}/notes.txt"),
                b"private note".to_vec(),
            );
        })
        .await;
    let error = fixture
        .provider()
        .upload(NAME, [MAGIC.as_slice(), b"encrypted"].concat())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("unconfirmed"));
    assert!(
        fixture
            .files
            .run(|f| f
                .commands
                .iter()
                .all(|c| !matches!(c.as_str(), "STOR" | "APPE" | "DELE" | "RMD")))
            .await
    );
    fixture.finish().await;
}
#[tokio::test]
async fn ftp_wire_retention_requires_complete_listing_and_preserves_other_files() {
    let fixture = wire_server::Fixture::start(Security::Plain).await;
    let provider = fixture.provider();
    let bytes = [MAGIC.as_slice(), b"encrypted fixture"].concat();
    let older = NAME.replace("120000", "110000");
    let newer = NAME.replace("120000", "130000");
    for name in [&older, NAME, &newer] {
        provider.upload(name, bytes.clone()).await.unwrap();
    }
    fixture
        .files
        .run(|f| {
            f.data.insert("/archive/notes.txt".into(), b"keep".to_vec());
            f.repeated_listing = true;
        })
        .await;
    assert!(super::super::retain(&provider, 2, &newer).await.is_err());
    assert!(
        !fixture
            .files
            .run(|f| f.commands.iter().any(|c| c == "DELE"))
            .await
    );
    fixture.files.run(|f| f.repeated_listing = false).await;
    assert_eq!(super::super::retain(&provider, 2, &newer).await.unwrap(), 1);
    assert_eq!(provider.list().await.unwrap().len(), 2);
    assert_eq!(
        fixture
            .files
            .run(|f| f.data["/archive/notes.txt"].clone())
            .await,
        b"keep"
    );
    fixture
        .files
        .run(|f| {
            f.data
                .insert(format!("/archive/{NAME}/extra.txt"), b"keep".to_vec());
        })
        .await;
    assert!(provider.delete(NAME).await.is_err());
    fixture.finish().await;
}
#[test]
fn ftp_settings_reject_command_injection_and_isolate_security_credentials() {
    let mut settings = Settings {
        host: "BACKUPS.example.test.".into(),
        username: "fixture-user".into(),
        directory: "/archive/".into(),
        ..Default::default()
    };
    settings.validate().unwrap();
    let original = settings.identity();
    let secret = settings.secret_id();
    settings.host = "backups.example.test".into();
    settings.directory = "/archive".into();
    assert_eq!(settings.identity(), original);
    settings.security = Security::Plain;
    assert_ne!(settings.secret_id(), secret);
    for path in ["/archive\r\nDELE other", "/one/../two", "/one//two"] {
        settings.directory = path.into();
        assert!(settings.validate().is_err());
    }
    assert!(Settings::default().validate_draft().is_ok());
    settings.directory = "/archive".into();
    assert!(FtpBackup::new(&settings, "password\r\nDELE file".into()).is_err());
}

#[tokio::test]
async fn ftp_duplicate_security_aliases_and_late_receipts_preserve_settings() {
    use crate::{
        backup::{BackupTarget, config},
        model::{BackupDestination, Preferences},
        store::Store,
    };
    let store = Store::memory().unwrap();
    let first = Preferences {
        backup_destination: BackupDestination::Ftp,
        backup_ftp: Settings {
            host: "Backups.example.test.".into(),
            username: "fixture-user".into(),
            directory: "/archive".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    store.save_preferences(first.clone()).await.unwrap();
    let target = BackupTarget::from_preferences(&first);
    let mut changed = store
        .record_backup(target.clone(), 1, true)
        .await
        .unwrap()
        .value;
    changed.backup_ftp.security = Security::Plain;
    let saved = store.save_preferences(changed).await.unwrap().value;
    assert!(!saved.backup_ready);
    assert!(saved.last_backup.is_none());
    assert!(
        store
            .record_backup(target, 2, true)
            .await
            .unwrap()
            .value
            .last_backup
            .is_none()
    );
    let mut duplicate = first;
    config::add(&mut duplicate).unwrap();
    duplicate.backup_destination = BackupDestination::Ftp;
    duplicate.backup_ftp = Settings {
        host: "backups.example.test".into(),
        username: "another-user".into(),
        directory: "/archive/".into(),
        security: Security::Plain,
        ..Default::default()
    };
    config::capture_editor(&mut duplicate);
    assert!(
        duplicate
            .validate()
            .unwrap_err()
            .to_string()
            .contains("already configured")
    );
}

#[tokio::test]
async fn ftp_directory_alias_is_rejected_before_upload_mutations() {
    let fixture = wire_server::Fixture::start(Security::Plain).await;
    fixture
        .files
        .run(|f| {
            f.dirs.insert("/fixture-alias".into());
        })
        .await;
    let mut settings = fixture.settings.clone();
    settings.directory = "/fixture-alias".into();
    let provider = FtpBackup::new(&settings, "fixture-password".into()).unwrap();
    assert!(
        provider
            .upload(NAME, [MAGIC.as_slice(), b"encrypted fixture"].concat())
            .await
            .is_err()
    );
    assert!(
        fixture
            .files
            .run(|f| f
                .commands
                .iter()
                .all(|c| !matches!(c.as_str(), "MKD" | "STOR" | "APPE" | "DELE")))
            .await
    );
    fixture.finish().await;
}

#[test]
fn ftp_callbacks_bound_data_and_control_and_observe_cancellation() {
    let (live, receiver) = oneshot::channel();
    let mut transfer = Transfer {
        bytes: Vec::new(),
        limit: 8,
        source: None,
        offset: 0,
        headers: 0,
        live,
        expected_directory: "/archive".into(),
        cwd_accepted: false,
        canonical: false,
    };
    assert_eq!(transfer.write(b"12345678").unwrap(), 8);
    assert_eq!(transfer.write(b"9").unwrap(), 0);
    assert_eq!(transfer.bytes, b"12345678");
    assert!(transfer.header(b"250 Directory accepted\r\n"));
    assert!(!transfer.header(b"257 \"/different\"\r\n"));
    assert!(transfer.progress(0., 0., 0., 0.));
    drop(receiver);
    assert!(!transfer.progress(0., 0., 0., 0.));
    transfer.canonical = true;
    transfer.headers = 128 * 1024;
    assert!(!transfer.header(b"200 More\r\n"));
}

#[tokio::test]
async fn ftp_wire_backup_format_options_restore_and_owned_retention() {
    use crate::backup::format::{self, Protection};
    let fixture = wire_server::Fixture::start(Security::ExplicitTls).await;
    let provider = fixture.provider();
    for protection in [Protection::None, Protection::Passphrase] {
        let bytes = format::tests::wire_fixture(protection);
        let mut upload = provider.reserve(NAME, &bytes).await.unwrap();
        provider
            .upload_prepared(&mut upload, &bytes, &crate::backup::NoCheckpoint)
            .await
            .unwrap();
        let received = provider.download(NAME).await.unwrap();
        assert_eq!(received, bytes);
        let password = secrecy::SecretString::from("a format fixture passphrase");
        assert_eq!(
            format::decode(
                &received,
                (protection == Protection::Passphrase).then_some(&password)
            )
            .unwrap()
            .created_at,
            1
        );
        assert_eq!(provider.list().await.unwrap().len(), 1);
        provider.delete(NAME).await.unwrap();
        assert!(provider.list().await.unwrap().is_empty());
    }
    fixture.finish().await;
}
