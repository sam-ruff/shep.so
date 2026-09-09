use super::*;
use crate::providers::test_http::{self, Reply, Server};

pub(in crate::profile_sync) fn binding() -> Binding {
    Binding::new("drive:fixture-user".into(), "so.shep.fixture".into()).unwrap()
}
pub(in crate::profile_sync) fn fixture() -> Record {
    Record::decode(
        binding().namespace(),
        include_bytes!("../../../tests/support/profile-operation.json").to_vec(),
    )
    .unwrap()
}
pub(in crate::profile_sync) fn reserved() -> ReservedUpload {
    let record = fixture();
    ReservedUpload {
        binding: binding(),
        remote: RemoteRecord {
            id: "reserved-profile".into(),
            key: record.key(),
            size: record.bytes().len() as u64,
            sha256: record.sha256.clone(),
        },
        record,
    }
}
fn file(upload: &ReservedUpload) -> Value {
    let key = upload.remote.key;
    json!({"id":upload.remote.id,"name":key.filename(),"trashed":false,"ownedByMe":true,"spaces":["appDataFolder"],"mimeType":"application/json","size":upload.remote.size.to_string(),"sha256Checksum":upload.remote.sha256,
        "properties":{"shepProfile":"1","shepNamespace":upload.binding.namespace_hash(),"shepProfileId":key.profile.to_string(),"shepGeneration":key.generation.to_string(),"shepOperation":key.operation.to_string(),"shepSha256":upload.remote.sha256}})
}
fn session(server: &Server) -> Session {
    Session {
        http: test_http::client(),
        base: server.url.clone(),
        token: SecretString::from("fixture-profile-token"),
        binding: binding(),
    }
}

#[tokio::test]
async fn profile_drive_refuses_wrong_account_namespace_or_reservation_space() {
    let durable = journal::Journal::open(None)
        .unwrap()
        .prepare(reserved())
        .await
        .unwrap();
    for binding in [
        Binding::new("drive:another-user".into(), "so.shep.fixture".into()).unwrap(),
        Binding::new("drive:fixture-user".into(), "so.shep.other".into()).unwrap(),
    ] {
        let mut server = Server::start(vec![]).await;
        let mut session = session(&server);
        session.binding = binding;
        let error = session.upload(&durable).await.unwrap_err();
        assert!(
            error
                .to_string()
                .contains("another Google account or application")
        );
        server.finish().await;
        assert!(server.requests().is_empty());
    }
    for body in [
        json!({"ids":["one"],"space":"drive"}),
        json!({"ids":["one","two"],"space":"appDataFolder"}),
        json!({"ids":["../unsafe"],"space":"appDataFolder"}),
        json!({"ids":[],"space":"appDataFolder"}),
    ] {
        let mut server = Server::start(vec![Reply::new(200, body.to_string())]).await;
        assert!(session(&server).reserve(fixture()).await.is_err());
        server.finish().await;
    }
}

#[tokio::test]
async fn profile_drive_verifies_google_account_before_access_and_never_follows_redirects() {
    for reply in [
        Reply::new(200, r#"{"user":{"permissionId":"another-user"}}"#),
        Reply::new(200, "{}"),
        Reply::new(401, "expired"),
        Reply::new(302, "").header("Location", "https://example.test/stolen"),
    ] {
        let mut server = Server::start(vec![reply]).await;
        assert!(session(&server).verify_identity().await.is_err());
        server.finish().await;
        assert_eq!(server.requests().len(), 1);
    }
    let mut server = Server::start(vec![Reply::new(
        200,
        r#"{"user":{"permissionId":"fixture-user"}}"#,
    )])
    .await;
    session(&server).verify_identity().await.unwrap();
    server.finish().await;
    assert!(
        server.requests()[0]
            .target
            .contains("/drive/v3/about?fields=user%28permissionId%29")
    );
}

#[tokio::test]
async fn profile_drive_pages_and_downloads_exact_shared_client_bytes_including_extensions() {
    let upload = reserved();
    let mut server = Server::start(vec![
        Reply::new(
            200,
            json!({"files":[],"nextPageToken":"next-1"}).to_string(),
        ),
        Reply::new(
            200,
            json!({"files":[file(&upload)],"incompleteSearch":false}).to_string(),
        ),
        Reply::new(200, file(&upload).to_string()),
        Reply::binary(200, upload.record.bytes().to_vec()),
    ])
    .await;
    let session = session(&server);
    let journal = journal::Journal::open(None).unwrap();
    let scan = journal
        .begin_scan(
            binding(),
            Some((upload.remote.key.profile, upload.remote.key.generation)),
        )
        .await
        .unwrap();
    let page = session.page_for(&scan).await.unwrap();
    assert!(page.records.is_empty());
    assert_eq!(page.next.as_deref(), Some("next-1"));
    let scan = journal.append_page(&scan, page).await.unwrap();
    assert!(!scan.complete());
    let next = session.page_for(&scan).await.unwrap();
    assert!(next.next.is_none());
    assert_eq!(next.records, vec![upload.remote.clone()]);
    let scan = journal.append_page(&scan, next).await.unwrap();
    assert!(scan.complete());
    let entries = journal.scan_entries(&scan, None).await.unwrap();
    let downloaded = session.download(&entries[0].record).await.unwrap();
    assert_eq!(downloaded.bytes(), upload.record.bytes());
    assert_eq!(
        downloaded.operation.extra["future_optional"]["nested"][3]["label"],
        "Résumé"
    );
    server.finish().await;
    for request in server.requests() {
        assert_eq!(
            request.headers["authorization"],
            "Bearer fixture-profile-token"
        );
        assert_eq!(request.method, "GET");
    }
    let request =
        url::Url::parse(&format!("http://fixture{}", server.requests()[0].target)).unwrap();
    let query: std::collections::HashMap<_, _> = request.query_pairs().into_owned().collect();
    assert_eq!(query["spaces"], "appDataFolder");
    assert!(query["q"].contains("properties has { key='shepProfile'"));
    assert!(!query["q"].contains("shepBackup"));
}

#[tokio::test]
async fn profile_drive_rejects_incomplete_repeated_duplicate_foreign_or_missing_lists() {
    let upload = reserved();
    let mut other_generation = file(&upload);
    other_generation["properties"]["shepGeneration"] = Uuid::new_v4().to_string().into();
    let mut other_namespace = file(&upload);
    other_namespace["properties"]["shepNamespace"] = "0".repeat(64).into();
    let mut duplicate_operation = file(&upload);
    duplicate_operation["id"] = "another-file-id".into();
    let mut backup = file(&upload);
    backup["properties"] = json!({"shepBackup":"1"});
    for value in [
        json!({}),
        json!({"files":null}),
        json!({"files":[],"incompleteSearch":true}),
        json!({"files":[],"incompleteSearch":"false"}),
        json!({"files":[],"nextPageToken":"again"}),
        json!({"files":[],"nextPageToken":32}),
        json!({"files":vec![file(&upload);101]}),
        json!({"files":[file(&upload),file(&upload)]}),
        json!({"files":[file(&upload),duplicate_operation]}),
        json!({"files":[other_generation]}),
        json!({"files":[other_namespace]}),
        json!({"files":[backup]}),
    ] {
        let mut server = Server::start(vec![Reply::new(200, value.to_string())]).await;
        assert!(
            session(&server)
                .list_page(
                    Some("again"),
                    Some((upload.remote.key.profile, upload.remote.key.generation))
                )
                .await
                .is_err(),
            "accepted {value}"
        );
        server.finish().await;
    }
}

#[tokio::test]
async fn profile_drive_reserves_before_journaling_and_uploads_exact_immutable_multipart() {
    let expected = reserved();
    let mut server = Server::start(vec![
        Reply::new(
            200,
            r#"{"ids":["reserved-profile"],"space":"appDataFolder"}"#,
        ),
        Reply::new(404, ""),
        Reply::new(201, file(&expected).to_string()),
    ])
    .await;
    let session = session(&server);
    let upload = session.reserve(fixture()).await.unwrap();
    let journal = journal::Journal::open(None).unwrap();
    let durable = journal.prepare(upload).await.unwrap();
    let receipt = session.upload(&durable).await.unwrap();
    journal.acknowledge(&durable, &receipt).await.unwrap();
    assert!(
        journal
            .load(&binding(), expected.remote.key)
            .await
            .unwrap()
            .unwrap()
            .acknowledged()
    );
    server.finish().await;
    let requests = server.requests();
    assert!(requests[0].target.contains("space=appDataFolder"));
    assert!(requests[2].target.contains("uploadType=multipart"));
    assert_eq!(requests[2].method, "POST");
    let content_type = &requests[2].headers["content-type"];
    let boundary = content_type
        .strip_prefix("multipart/related; boundary=")
        .unwrap();
    let parts: Vec<_> = requests[2].body.split(&format!("--{boundary}")).collect();
    let metadata: Value =
        serde_json::from_str(parts[1].split_once("\r\n\r\n").unwrap().1.trim()).unwrap();
    assert_eq!(metadata["id"], "reserved-profile");
    assert_eq!(metadata["parents"], json!(["appDataFolder"]));
    assert!(metadata.get("appProperties").is_none());
    for (key, value) in metadata["properties"].as_object().unwrap() {
        assert!(key.len() + value.as_str().unwrap().len() <= 124);
    }
    let exact = parts[2]
        .split_once("\r\n\r\n")
        .unwrap()
        .1
        .strip_suffix("\r\n")
        .unwrap();
    assert_eq!(exact.as_bytes(), fixture().bytes());
}

#[tokio::test]
async fn profile_drive_recovers_lost_reply_after_restart_without_a_new_upload_id() {
    let upload = reserved();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("profiles-upload.sqlite");
    let journal = journal::Journal::open(Some(&path)).unwrap();
    let durable = journal.prepare(upload.clone()).await.unwrap();
    let mut first = Server::start(vec![
        Reply::new(404, ""),
        Reply::disconnect(),
        Reply::new(503, "later"),
    ])
    .await;
    assert!(session(&first).upload(&durable).await.is_err());
    first.finish().await;
    drop(journal);
    let journal = journal::Journal::open(Some(&path)).unwrap();
    let durable = journal
        .load(&binding(), upload.remote.key)
        .await
        .unwrap()
        .unwrap();
    assert!(!durable.acknowledged());
    let mut metadata = file(&upload);
    metadata.as_object_mut().unwrap().remove("sha256Checksum");
    let mut resumed = Server::start(vec![
        Reply::new(200, metadata.to_string()),
        Reply::binary(200, upload.record.bytes().to_vec()),
    ])
    .await;
    let receipt = session(&resumed).upload(&durable).await.unwrap();
    journal.acknowledge(&durable, &receipt).await.unwrap();
    resumed.finish().await;
    assert!(
        resumed
            .requests()
            .iter()
            .all(|request| request.method == "GET")
    );
    assert_eq!(
        journal
            .load(&binding(), upload.remote.key)
            .await
            .unwrap()
            .unwrap()
            .upload()
            .record
            .bytes(),
        upload.record.bytes()
    );
}

#[tokio::test]
async fn profile_drive_conflict_or_lost_success_requires_exact_server_content() {
    let upload = reserved();
    for reply in [
        Reply::new(409, "exists"),
        Reply::disconnect(),
        Reply::new(200, "invalid json"),
        Reply::new(302, "").header("Location", "https://example.test/not-drive"),
    ] {
        let mut server = Server::start(vec![
            Reply::new(404, ""),
            reply,
            Reply::new(200, file(&upload).to_string()),
        ])
        .await;
        let durable = journal::Journal::open(None)
            .unwrap()
            .prepare(upload.clone())
            .await
            .unwrap();
        assert_eq!(
            session(&server).upload(&durable).await.unwrap(),
            upload.remote
        );
        server.finish().await;
        assert_eq!(server.requests().len(), 3);
    }
    let mut changed = file(&upload);
    changed["sha256Checksum"] = "0".repeat(64).into();
    let mut server = Server::start(vec![Reply::new(200, changed.to_string())]).await;
    let durable = journal::Journal::open(None)
        .unwrap()
        .prepare(upload)
        .await
        .unwrap();
    assert!(session(&server).upload(&durable).await.is_err());
    server.finish().await;
    assert!(
        server
            .requests()
            .iter()
            .all(|request| request.method == "GET")
    );
}

#[tokio::test]
async fn profile_drive_never_recreates_a_missing_previously_acknowledged_record() {
    let upload = reserved();
    let journal = journal::Journal::open(None).unwrap();
    let saved = journal.prepare(upload.clone()).await.unwrap();
    journal.acknowledge(&saved, &upload.remote).await.unwrap();
    let saved = journal
        .load(&binding(), upload.remote.key)
        .await
        .unwrap()
        .unwrap();
    let mut server = Server::start(vec![Reply::new(404, "")]).await;
    let error = session(&server).upload(&saved).await.unwrap_err();
    assert!(error.to_string().contains("previously confirmed"));
    server.finish().await;
    assert_eq!(server.requests().len(), 1);
    assert_eq!(server.requests()[0].method, "GET");
}

#[tokio::test]
async fn profile_drive_rejects_mutated_content_ownership_versions_and_bounded_responses() {
    let upload = reserved();
    for field in [
        "id",
        "name",
        "mimeType",
        "ownedByMe",
        "spaces",
        "size",
        "sha256Checksum",
    ] {
        let mut changed = file(&upload);
        changed[field] = Value::Null;
        let mut server = Server::start(vec![Reply::new(200, changed.to_string())]).await;
        assert!(
            session(&server).download(&upload.remote).await.is_err(),
            "accepted {field}"
        );
        server.finish().await;
    }
    let mut wrong = upload.record.bytes().to_vec();
    wrong[0] = b'[';
    for reply in [
        Reply::binary(200, wrong),
        Reply::binary(200, upload.record.bytes()[..100].to_vec()),
        Reply::chunked(200, vec![b' '; MAX_RECORD_BYTES + 1]),
    ] {
        let mut server =
            Server::start(vec![Reply::new(200, file(&upload).to_string()), reply]).await;
        assert!(session(&server).download(&upload.remote).await.is_err());
        server.finish().await;
    }
    let mut future: Value = serde_json::from_slice(upload.record.bytes()).unwrap();
    future["major"] = 99.into();
    let bytes = serde_json::to_vec(&future).unwrap();
    let remote = RemoteRecord {
        size: bytes.len() as u64,
        sha256: digest(&bytes),
        ..upload.remote.clone()
    };
    let mut metadata = file(&upload);
    metadata["size"] = remote.size.to_string().into();
    metadata["sha256Checksum"] = remote.sha256.clone().into();
    metadata["properties"]["shepSha256"] = remote.sha256.clone().into();
    let mut server = Server::start(vec![
        Reply::new(200, metadata.to_string()),
        Reply::binary(200, bytes),
    ])
    .await;
    let error = session(&server).download(&remote).await.unwrap_err();
    assert_eq!(
        error.downcast_ref::<shep_profile_core::Error>(),
        Some(&shep_profile_core::Error::Upgrade)
    );
    server.finish().await;
    for reply in [
        Reply::binary(200, vec![b' '; 2 * 1024 * 1024 + 1]),
        Reply::chunked(200, vec![b' '; 2 * 1024 * 1024 + 1]),
    ] {
        let mut server = Server::start(vec![reply]).await;
        assert!(session(&server).list_page(None, None).await.is_err());
        server.finish().await;
    }
}
