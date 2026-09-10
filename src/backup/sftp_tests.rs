use super::*;
use russh::server::{self, Server as _};
use std::collections::BTreeMap;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};

struct HostServer {
    events: mpsc::Sender<bool>,
}
impl server::Server for HostServer {
    type Handler = HostSession;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        HostSession {
            events: self.events.clone(),
        }
    }
}
struct HostSession {
    events: mpsc::Sender<bool>,
}
impl server::Handler for HostSession {
    type Error = anyhow::Error;
    async fn auth_password(
        &mut self,
        username: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        self.events
            .send(username == "fixture-user" && password == "fixture-password")
            .await?;
        Ok(server::Auth::Reject {
            proceed_with_methods: None,
            partial_success: false,
        })
    }
}
async fn host_server() -> (
    Settings,
    mpsc::Receiver<bool>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let key =
        russh::keys::PrivateKey::random(&mut rand_sftp::rng(), russh::keys::Algorithm::Ed25519)
            .unwrap();
    let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let settings = Settings {
        host: "127.0.0.1".into(),
        port: listener.local_addr().unwrap().port(),
        username: "fixture-user".into(),
        directory: "/archive".into(),
        fingerprint,
    };
    let (events, observations) = mpsc::channel(8);
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(async move {
        let mut server = HostServer { events };
        let running = server.run_on_socket(
            Arc::new(server::Config {
                keys: vec![key],
                auth_rejection_time: Duration::ZERO,
                auth_rejection_time_initial: Some(Duration::ZERO),
                ..Default::default()
            }),
            &listener,
        );
        let handle = running.handle();
        tokio::pin!(running);
        tokio::select! {
            result = &mut running => result.unwrap(),
            _ = stopped => {
                handle.shutdown("Fixture finished".into());
                tokio::time::timeout(Duration::from_secs(2), running).await.unwrap().unwrap();
            }
        }
    });
    (settings, observations, stop, task)
}

#[tokio::test]
async fn sftp_probe_and_changed_host_key_never_send_a_password() {
    let (mut settings, mut observations, stop, task) = host_server().await;
    assert_eq!(
        probe_fingerprint(&settings).await.unwrap(),
        settings.fingerprint
    );
    assert!(observations.try_recv().is_err());
    settings.fingerprint = format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode([7; 32])
    );
    let error = SftpBackup::new(&settings, "fixture-password".into())
        .unwrap()
        .test_connection()
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("host key changed") && error.contains("No password was sent"));
    assert!(!error.contains("fixture-password"));
    assert!(observations.try_recv().is_err());
    stop.send(()).unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn sftp_password_authentication_requires_the_verified_host_key() {
    let (settings, mut observations, stop, task) = host_server().await;
    let error = SftpBackup::new(&settings, "fixture-password".into())
        .unwrap()
        .test_connection()
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("rejected the username or password"));
    assert!(!error.contains("fixture-password"));
    assert!(observations.recv().await.unwrap());
    stop.send(()).unwrap();
    task.await.unwrap();
}

#[tokio::test]
async fn sftp_framing_rejects_oversized_packets_before_reading_a_body() {
    let (client, mut server) = tokio::io::duplex(64);
    let mut client = bounded_stream(client);
    server.write_all(&u32::MAX.to_be_bytes()).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(1), client.read_u8())
        .await
        .unwrap();
    assert!(result.is_err());
    let (client, mut server) = tokio::io::duplex(64);
    let mut client = bounded_stream(client);
    let bytes = [0, 0, 0, 3, 1, 2, 3, 0, 0, 0, 1, 4];
    server.write_all(&bytes).await.unwrap();
    server.shutdown().await.unwrap();
    let mut received = Vec::new();
    client.read_to_end(&mut received).await.unwrap();
    assert_eq!(received, bytes);
}

#[test]
fn sftp_settings_require_a_verified_fingerprint_and_isolate_credentials() {
    let mut settings = Settings {
        host: "Backups.Example.TEST".into(),
        username: "first".into(),
        directory: "/archive/".into(),
        fingerprint: format!(
            "SHA256:{}",
            base64::engine::general_purpose::STANDARD_NO_PAD.encode([0; 32])
        ),
        ..Default::default()
    };
    settings.validate().unwrap();
    let original = settings.identity();
    let first_key = settings.secret_id();
    settings.host = "backups.example.test".into();
    settings.directory = "/archive".into();
    assert_eq!(settings.identity(), original);
    settings.username = "second".into();
    assert_ne!(settings.identity(), original);
    assert_ne!(settings.secret_id(), first_key);
    settings.fingerprint.clear();
    assert!(settings.validate().is_err());
    assert!(settings.validate_draft().is_ok());
    for directory in [
        "relative",
        "/one/../two",
        "/one//two",
        "/one/./two",
        "/bad\n",
    ] {
        settings.directory = directory.into();
        assert!(settings.validate_draft().is_err());
    }
}

#[path = "sftp_test_server.rs"]
pub(crate) mod wire_server;
const NAME: &str = "shep-20260909T120000Z-11111111-1111-4111-8111-111111111111.shepbackup";

#[tokio::test]
async fn sftp_wire_upload_restore_and_retention_preserve_other_files() {
    let fixture = wire_server::Fixture::start().await;
    let bytes = [MAGIC.as_slice(), b"fixture encrypted copy"].concat();
    fixture
        .files
        .run(|files| {
            files.short_reads = true;
            files.data.insert("notes.txt".into(), b"unrelated".to_vec());
        })
        .await;
    let provider = fixture.provider();
    provider.upload(NAME, bytes.clone()).await.unwrap();
    assert_eq!(provider.download(NAME).await.unwrap(), bytes);
    assert_eq!(provider.list().await.unwrap().len(), 1);
    provider.delete(NAME).await.unwrap();
    assert_eq!(
        fixture.files.run(|files| files.data.clone()).await,
        BTreeMap::from([("notes.txt".into(), b"unrelated".to_vec())])
    );
    fixture.finish().await;
}

#[tokio::test]
async fn sftp_wire_partial_upload_resumes_after_its_acknowledged_prefix() {
    let fixture = wire_server::Fixture::start().await;
    let bytes = [MAGIC.as_slice(), &vec![42; CHUNK as usize * 2]].concat();
    fixture.files.run(|files| files.fail_write = Some(2)).await;
    let provider = fixture.provider();
    let mut upload = provider.reserve(NAME, &bytes).await.unwrap();
    assert!(
        provider
            .upload_prepared(&mut upload, &bytes, &super::super::NoCheckpoint)
            .await
            .is_err()
    );
    assert!(upload.session.is_some());
    let encoded = serde_json::to_vec(&upload).unwrap();
    let mut resumed = serde_json::from_slice(&encoded).unwrap();
    provider
        .upload_prepared(&mut resumed, &bytes, &super::super::NoCheckpoint)
        .await
        .unwrap();
    let writes = fixture.files.run(|files| files.writes.clone()).await;
    assert_eq!(
        writes.iter().filter(|(_, offset, _)| *offset == 0).count(),
        1
    );
    assert_eq!(writes[2].1, CHUNK as u64);
    assert_eq!(provider.download(NAME).await.unwrap(), bytes);
    assert_eq!(fixture.files.run(|files| files.renames).await, 1);
    fixture.finish().await;
}

#[tokio::test]
async fn sftp_wire_uncertain_rename_recovers_without_another_upload() {
    let fixture = wire_server::Fixture::start().await;
    let bytes = [MAGIC.as_slice(), b"fixture encrypted archive"].concat();
    fixture.files.run(|files| files.lose_rename = true).await;
    let provider = fixture.provider();
    let mut upload = provider.reserve(NAME, &bytes).await.unwrap();
    assert!(
        provider
            .upload_prepared(&mut upload, &bytes, &super::super::NoCheckpoint)
            .await
            .is_err()
    );
    let writes = fixture.files.run(|files| files.writes.len()).await;
    provider
        .upload_prepared(&mut upload, &bytes, &super::super::NoCheckpoint)
        .await
        .unwrap();
    assert_eq!(fixture.files.run(|files| files.writes.len()).await, writes);
    assert_eq!(fixture.files.run(|files| files.renames).await, 1);
    fixture.finish().await;
}

#[tokio::test]
async fn sftp_wire_incomplete_listing_and_foreign_copy_prevent_retention() {
    let fixture = wire_server::Fixture::start().await;
    fixture
        .files
        .run(|files| {
            files.repeat_listing = true;
            files
                .data
                .insert(NAME.into(), [MAGIC.as_slice(), b"ciphertext"].concat());
        })
        .await;
    assert!(
        super::super::retain(&fixture.provider(), 1, NAME)
            .await
            .is_err()
    );
    fixture
        .files
        .run(|files| {
            files.repeat_listing = false;
            files.data.insert(NAME.into(), b"a foreign file".to_vec());
        })
        .await;
    assert!(fixture.provider().delete(NAME).await.is_err());
    assert!(fixture.files.run(|files| files.deletes.is_empty()).await);
    assert_eq!(
        fixture.files.run(|files| files.data[NAME].clone()).await,
        b"a foreign file"
    );
    fixture.finish().await;
}

#[tokio::test]
async fn sftp_durable_journal_recovers_a_committed_rename_after_reopen() {
    use super::super::{
        BackupTarget,
        journal::{Checkpoint, Journal},
    };
    let fixture = wire_server::Fixture::start().await;
    let target = BackupTarget::Sftp(fixture.settings.identity());
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("upload.sqlite");
    let bytes = [MAGIC.as_slice(), b"fixture encrypted archive"].concat();
    let provider = fixture.provider();
    {
        let journal = Journal::open(Some(&path)).unwrap();
        let upload = provider.reserve(NAME, &bytes).await.unwrap();
        let mut pending = journal
            .prepare(&target, upload, bytes.clone())
            .await
            .unwrap();
        fixture.files.run(|files| files.lose_rename = true).await;
        let checkpoint = Checkpoint {
            journal: journal.clone(),
            target: target.clone(),
        };
        assert!(
            provider
                .upload_prepared(&mut pending.upload, &pending.data, &checkpoint)
                .await
                .is_err()
        );
        assert!(!journal.pending(&target).await.unwrap().unwrap().committed);
    }
    let journal = Journal::open(Some(&path)).unwrap();
    let mut pending = journal.pending(&target).await.unwrap().unwrap();
    assert_eq!(pending.data, bytes);
    let before = fixture.files.run(|files| files.writes.len()).await;
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
    journal.committed(&target, NAME).await.unwrap();
    journal.remove(&target, NAME).await.unwrap();
    assert!(journal.pending(&target).await.unwrap().is_none());
    assert_eq!(fixture.files.run(|files| files.writes.len()).await, before);
    fixture.finish().await;
}

#[tokio::test]
async fn sftp_aliases_and_server_identity_changes_preserve_target_metadata_boundaries() {
    use crate::{
        backup::{BackupTarget, config},
        model::{BackupDestination, Preferences},
        store::Store,
    };
    let fixture = wire_server::Fixture::start().await;
    let store = Store::memory().unwrap();
    let first = Preferences {
        backup_destination: BackupDestination::Sftp,
        backup_sftp: fixture.settings.clone(),
        ..Default::default()
    };
    store.save_preferences(first.clone()).await.unwrap();
    let target = BackupTarget::from_preferences(&first);
    let mut ready = store
        .record_backup(target.clone(), 123, true)
        .await
        .unwrap()
        .value;
    ready.backup_sftp.fingerprint = format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode([9; 32])
    );
    let changed = store.save_preferences(ready).await.unwrap().value;
    assert!(!changed.backup_ready && changed.last_backup.is_none());
    assert!(
        store
            .record_backup(target, 456, true)
            .await
            .unwrap()
            .value
            .last_backup
            .is_none()
    );
    let mut duplicate = first;
    config::add(&mut duplicate).unwrap();
    duplicate.backup_destination = BackupDestination::Sftp;
    duplicate.backup_sftp = fixture.settings.clone();
    duplicate.backup_sftp.username = "another-user".into();
    duplicate.backup_sftp.directory.push('/');
    duplicate.backup_sftp.fingerprint = changed.backup_sftp.fingerprint;
    config::capture_editor(&mut duplicate);
    assert!(
        duplicate
            .validate()
            .unwrap_err()
            .to_string()
            .contains("already configured")
    );
    fixture.finish().await;
}

#[test]
fn sftp_ipv6_aliases_share_identity_and_wire_errors_hide_server_text() {
    assert_eq!(normalized_host("[::1]").unwrap(), "::1");
    assert_eq!(normalized_host("0:0:0:0:0:0:0:1").unwrap(), "::1");
    let error = wire(SftpError::Status(russh_sftp::protocol::Status {
        id: 1,
        status_code: StatusCode::PermissionDenied,
        error_message: "fixture-password and private server details".into(),
        language_tag: "en".into(),
    }));
    assert!(error.to_string().contains("PermissionDenied"));
    assert!(!error.to_string().contains("fixture-password"));
}

#[tokio::test]
async fn sftp_wire_rolling_retention_preserves_new_copy_and_unrelated_files() {
    let fixture = wire_server::Fixture::start().await;
    let provider = fixture.provider();
    let bytes = [MAGIC.as_slice(), b"encrypted fixture"].concat();
    let older = NAME.replace("120000", "100000");
    let newest = NAME.replace("120000", "130000");
    fixture
        .files
        .run(|files| {
            files.data.insert("notes.txt".into(), b"keep".to_vec());
        })
        .await;
    for name in [&older, NAME, &newest] {
        provider.upload(name, bytes.clone()).await.unwrap();
    }
    assert_eq!(
        super::super::retain(&provider, 2, &newest).await.unwrap(),
        1
    );
    let copies = provider.list().await.unwrap();
    assert_eq!(
        copies
            .iter()
            .map(|copy| copy.name.as_str())
            .collect::<Vec<_>>(),
        [newest.as_str(), NAME]
    );
    assert_eq!(
        fixture
            .files
            .run(|files| files.data["notes.txt"].clone())
            .await,
        b"keep"
    );
    assert_eq!(provider.download(&newest).await.unwrap(), bytes);
    fixture.finish().await;
}

#[tokio::test]
async fn sftp_wire_conflicting_final_or_unconfirmed_staging_are_never_overwritten() {
    let fixture = wire_server::Fixture::start().await;
    let provider = fixture.provider();
    let bytes = [MAGIC.as_slice(), b"encrypted fixture"].concat();
    fixture
        .files
        .run(|files| {
            files.data.insert(NAME.into(), b"foreign file".to_vec());
        })
        .await;
    assert!(provider.upload(NAME, bytes.clone()).await.is_err());
    assert_eq!(
        fixture.files.run(|files| files.data[NAME].clone()).await,
        b"foreign file"
    );
    let staging = format!(".shep-upload-{NAME}.part");
    let staging_copy = staging.clone();
    fixture
        .files
        .run(move |files| {
            files.data.remove(NAME);
            files.data.insert(staging_copy, b"foreign partial".to_vec());
        })
        .await;
    let error = provider.upload(NAME, bytes).await.unwrap_err().to_string();
    assert!(error.contains("unconfirmed SFTP staging"));
    assert_eq!(
        fixture
            .files
            .run(move |files| files.data[&staging].clone())
            .await,
        b"foreign partial"
    );
    assert!(
        fixture
            .files
            .run(|files| files.writes.is_empty() && files.deletes.is_empty() && files.renames == 0)
            .await
    );
    fixture.finish().await;
}

#[tokio::test]
async fn sftp_channel_confirmation_deadline_releases_setup_and_allows_retry() {
    let peer = wire_server::Fixture::start().await;
    let (entered, waiting) = oneshot::channel();
    let (release, released) = oneshot::channel();
    peer.files
        .run(move |files| files.hold_channel = Some((entered, released)))
        .await;
    let provider = peer.provider();
    let setup = tokio::spawn(async move { provider.test_connection().await });
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the real SSH handshake must reach channel confirmation")
        .unwrap();
    // Advance only after the real handshake. No fifteen-second wall-clock wait
    // and no shortened production timeout or performance budget is involved.
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(16)).await;
    let outcome = tokio::time::timeout(Duration::from_secs(1), setup).await;
    tokio::time::resume();
    let error = outcome
        .expect("a live peer must not hold channel setup indefinitely")
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("session in time"), "{error:#}");
    release.send(()).unwrap();
    peer.provider().test_connection().await.unwrap();
    peer.files
        .run(|files| {
            assert!(files.data.is_empty());
            assert!(files.writes.is_empty());
        })
        .await;
    peer.finish().await;
}

#[tokio::test]
async fn sftp_wire_backup_format_options_restore_and_owned_retention() {
    use crate::backup::format::{self, Protection};
    let fixture = wire_server::Fixture::start().await;
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
