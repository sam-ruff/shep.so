use super::*;
use crate::providers::test_http::{Reply, Server};
const NAME: &str = "shep-20260909T120000Z-11111111-1111-4111-8111-111111111111.shepbackup";

fn provider(server: &Server) -> S3Backup {
    let mut endpoint = server.url.clone();
    endpoint.set_path("/");
    S3Backup::configured(
        &Settings {
            endpoint: endpoint.to_string(),
            bucket: "fixture-backups".into(),
            ..Default::default()
        },
        "fixture-key".into(),
        "fixture-secret".into(),
        true,
    )
    .unwrap()
}
fn metadata(bytes: &[u8]) -> Reply {
    Reply::new(200, "")
        .header("content-length", &bytes.len().to_string())
        .header("x-amz-meta-shep-backup", "1")
        .header(
            "x-amz-meta-shep-sha256",
            &format!("{:x}", sha2::Sha256::digest(bytes)),
        )
        .header("etag", "\"fixture-etag\"")
}

#[tokio::test]
async fn s3_lost_put_reply_recovers_exact_ciphertext_without_new_key_or_overwrite() {
    let bytes = b"fixture encrypted archive";
    let mut server = Server::start(vec![
        Reply::new(404, ""),
        Reply::disconnect(),
        metadata(bytes),
        Reply::binary(200, bytes.to_vec()),
    ])
    .await;
    let provider = provider(&server);
    let mut upload = provider.reserve(NAME, bytes).await.unwrap();
    provider
        .upload_prepared(&mut upload, bytes, &super::super::NoCheckpoint)
        .await
        .unwrap();
    server.finish().await;
    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .map(|r| r.method.as_str())
            .collect::<Vec<_>>(),
        ["HEAD", "PUT", "HEAD", "GET"]
    );
    let request = &requests[1];
    assert_eq!(request.headers["if-none-match"], "*");
    assert_eq!(request.bytes, bytes);
    assert_eq!(request.headers["x-amz-meta-shep-backup"], "1");
    assert!(
        request
            .target
            .starts_with(&format!("/fixture-backups/shep/{NAME}?"))
    );
    let url = url::Url::parse(&format!("http://fixture{}", request.target)).unwrap();
    let fields: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(fields["X-Amz-Algorithm"], "AWS4-HMAC-SHA256");
    assert!(fields["X-Amz-SignedHeaders"].contains("if-none-match"));
    assert!(fields.contains_key("X-Amz-Signature"));
    assert_eq!(requests[3].headers["if-match"], "\"fixture-etag\"");
}

#[tokio::test]
async fn s3_existing_conflict_is_never_overwritten() {
    let mut server = Server::start(vec![metadata(b"different ciphertext")]).await;
    let provider = provider(&server);
    let mut upload = provider.reserve(NAME, b"original").await.unwrap();
    assert!(
        provider
            .upload_prepared(&mut upload, b"original", &super::super::NoCheckpoint)
            .await
            .unwrap_err()
            .to_string()
            .contains("different backup")
    );
    server.finish().await;
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn s3_restore_verifies_bytes_and_retention_refuses_unowned_objects() {
    let bytes = b"original ciphertext";
    let mut server = Server::start(vec![
        metadata(bytes),
        Reply::binary(200, b"corrupted bytes!!!!".to_vec()),
        Reply::new(200, ""),
    ])
    .await;
    let provider = provider(&server);
    let id = format!("shep/{NAME}");
    assert!(provider.download(&id).await.is_err());
    assert!(
        provider
            .delete(&id)
            .await
            .unwrap_err()
            .to_string()
            .contains("not owned")
    );
    server.finish().await;
    assert_eq!(server.requests().len(), 3);
}

#[tokio::test]
async fn s3_incomplete_listing_and_redirects_keep_existing_backups() {
    for xml in [
        format!(
            "<ListBucketResult xmlns=\"{XML_NAMESPACE}\"><EncodingType>url</EncodingType><IsTruncated>true</IsTruncated></ListBucketResult>"
        ),
        format!(
            "<ListBucketResult xmlns=\"{XML_NAMESPACE}\"><EncodingType>url</EncodingType></ListBucketResult>"
        ),
    ] {
        let mut server = Server::start(vec![Reply::new(200, xml)]).await;
        assert!(provider(&server).list().await.is_err());
        server.finish().await;
        assert_eq!(server.requests().len(), 1);
    }
    let mut server = Server::start(vec![
        Reply::new(307, "secret upstream error").header("Location", "http://127.0.0.1:1/untrusted"),
    ])
    .await;
    let error = provider(&server)
        .test_connection()
        .await
        .unwrap_err()
        .to_string();
    assert!(
        !error.contains("fixture-secret")
            && !error.contains("X-Amz-Signature")
            && !error.contains("upstream")
    );
    server.finish().await;
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn s3_settings_reject_insecure_endpoints_credentials_and_escaping_prefixes() {
    let settings = Settings {
        bucket: "fixture-backups".into(),
        ..Default::default()
    };
    settings.validate().unwrap();
    for endpoint in [
        "http://example.test",
        "https://user:secret@example.test",
        "https://example.test/path",
        "https://example.test?token=private",
    ] {
        assert!(
            Settings {
                endpoint: endpoint.into(),
                ..settings.clone()
            }
            .validate()
            .is_err()
        );
    }
    for prefix in ["../other", "mail/../../other", "/absolute", "mail\\other"] {
        assert!(
            Settings {
                prefix: prefix.into(),
                ..settings.clone()
            }
            .validate()
            .is_err()
        );
    }
}

fn listing(keys: &[&str], next: Option<&str>) -> String {
    let objects = keys.iter().map(|key| format!("<Contents><Key>{key}</Key><LastModified>2026-09-09T12:00:00Z</LastModified></Contents>")).collect::<String>();
    format!(
        "<ListBucketResult xmlns=\"{XML_NAMESPACE}\"><EncodingType>url</EncodingType><IsTruncated>{}</IsTruncated>{objects}{}</ListBucketResult>",
        next.is_some(),
        next.map(|token| format!("<NextContinuationToken>{token}</NextContinuationToken>"))
            .unwrap_or_default()
    )
}

#[tokio::test]
async fn s3_journal_restart_keeps_the_reserved_key_and_exact_archive_after_uncertain_upload() {
    let bytes = b"original encrypted archive";
    let mut server = Server::start(vec![
        Reply::new(404, ""),
        Reply::disconnect(),
        Reply::new(503, ""),
        metadata(bytes),
        Reply::binary(200, bytes.to_vec()),
    ])
    .await;
    let provider = provider(&server);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("uploads.sqlite");
    let target = super::super::BackupTarget::S3(
        Settings {
            bucket: "fixture-backups".into(),
            ..Default::default()
        }
        .identity(),
    );
    let journal = super::super::journal::Journal::open(Some(&path)).unwrap();
    let upload = provider.reserve(NAME, bytes).await.unwrap();
    let mut pending = journal
        .prepare(&target, upload.clone(), bytes.to_vec())
        .await
        .unwrap();
    let checkpoint = super::super::journal::Checkpoint {
        journal: journal.clone(),
        target: target.clone(),
    };
    assert!(
        provider
            .upload_prepared(&mut pending.upload, &pending.data, &checkpoint)
            .await
            .is_err()
    );
    drop(checkpoint);
    drop(journal);
    let journal = super::super::journal::Journal::open(Some(&path)).unwrap();
    let mut pending = journal.pending(&target).await.unwrap().unwrap();
    assert_eq!(pending.upload.id, upload.id);
    assert_eq!(pending.data, bytes);
    let checkpoint = super::super::journal::Checkpoint {
        journal: journal.clone(),
        target: target.clone(),
    };
    provider
        .upload_prepared(&mut pending.upload, &pending.data, &checkpoint)
        .await
        .unwrap();
    journal.committed(&target, &upload.id).await.unwrap();
    journal.remove(&target, &upload.id).await.unwrap();
    assert!(journal.pending(&target).await.unwrap().is_none());
    server.finish().await;
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|r| r.method == "PUT")
            .count(),
        1
    );
}

#[tokio::test]
async fn s3_complete_owned_listing_drives_conditional_retention_only_in_its_prefix() {
    let bytes = b"ciphertext";
    let old_name = NAME.replace("20260909", "20260908");
    let new_id = format!("shep/{NAME}");
    let old_id = format!("shep/{old_name}");
    let mut server = Server::start(vec![
        Reply::new(
            200,
            listing(&[&new_id, "shep/unrelated.txt"], Some("page2")),
        ),
        metadata(bytes),
        Reply::new(200, listing(&[&old_id, "elsewhere/unrelated.txt"], None)),
        metadata(bytes),
        metadata(bytes),
        Reply::new(204, ""),
    ])
    .await;
    let provider = provider(&server);
    super::super::retain(&provider, 1, &new_id).await.unwrap();
    server.finish().await;
    let requests = server.requests();
    assert_eq!(requests.iter().filter(|r| r.method == "DELETE").count(), 1);
    let deletion = requests.last().unwrap();
    assert!(deletion.target.contains(&old_name));
    assert_eq!(deletion.headers["if-match"], "\"fixture-etag\"");
    assert!(requests[2].target.contains("continuation-token=page2"));
}

#[tokio::test]
async fn s3_duplicate_pages_missing_owner_and_missing_objects_cannot_trigger_retention() {
    let id = format!("shep/{NAME}");
    let bytes = b"ciphertext";
    let mut duplicate = Server::start(vec![
        Reply::new(200, listing(&[&id], Some("page2"))),
        metadata(bytes),
        Reply::new(200, listing(&[&id], None)),
    ])
    .await;
    assert!(
        super::super::retain(&provider(&duplicate), 1, &id)
            .await
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    duplicate.finish().await;
    assert!(!duplicate.requests().iter().any(|r| r.method == "DELETE"));
    let mut foreign = Server::start(vec![
        Reply::new(200, listing(&[&id], None)),
        Reply::new(200, "").header("x-amz-meta-shep-backup", "another-app"),
    ])
    .await;
    assert!(
        provider(&foreign)
            .list()
            .await
            .unwrap_err()
            .to_string()
            .contains("not owned")
    );
    foreign.finish().await;
    let mut missing = Server::start(vec![
        Reply::new(200, listing(&[&id], None)),
        Reply::new(404, ""),
    ])
    .await;
    assert!(
        super::super::retain(&provider(&missing), 1, &id)
            .await
            .is_err()
    );
    missing.finish().await;
    assert!(!missing.requests().iter().any(|r| r.method == "DELETE"));
}

#[test]
fn s3_destination_aliases_share_identity_and_cannot_be_configured_twice() {
    use crate::model::{BackupDestination, Preferences};
    let first = Settings {
        endpoint: "https://S3.Example.TEST:443".into(),
        bucket: "backups".into(),
        prefix: "shep/".into(),
        ..Default::default()
    };
    let second = Settings {
        endpoint: "https://s3.example.test/".into(),
        prefix: "shep".into(),
        path_style: false,
        region: "different-signing-region".into(),
        ..first.clone()
    };
    assert_eq!(first.identity(), second.identity());
    assert_eq!(first.identity().secret_id(), second.identity().secret_id());
    let mut prefs = Preferences {
        backup_destination: BackupDestination::S3,
        backup_s3: first,
        ..Default::default()
    };
    super::super::config::add(&mut prefs).unwrap();
    prefs.backup_destination = BackupDestination::S3;
    prefs.backup_s3 = second;
    super::super::config::capture_editor(&mut prefs);
    assert!(
        prefs
            .validate()
            .unwrap_err()
            .to_string()
            .contains("already configured")
    );
}
