use super::*;
use crate::{Action, Change, SettingKey};
mod changes;
mod discovery;
mod server;
use server::{Response as TestResponse, *};

const NAMESPACE: &str = "so.shep.fixture";
const PRINCIPAL: &str = "drive:fixture-owner";
const FILE_ID: &str = "reserved-fixture-file";

#[tokio::test]
async fn fixture_transport_rejects_non_loopback_and_ambiguous_endpoints_before_connecting() {
    for endpoint in [
        "https://127.0.0.1:1234/",
        "http://localhost:1234/",
        "http://192.0.2.1:1234/",
        "http://127.0.0.1@192.0.2.1:1234/",
        "http://user@127.0.0.1:1234/",
        "http://127.0.0.1:1234/path",
        "http://127.0.0.1:1234/?x",
        "http://127.0.0.1:1234/#x",
    ] {
        assert!(matches!(
            Drive::connect_fixture(
                Url::parse(endpoint).unwrap(),
                NAMESPACE.into(),
                Some(PRINCIPAL)
            )
            .await,
            Err(Error::Invalid)
        ));
    }
}
#[tokio::test]
async fn fixture_transport_uses_only_the_fake_token_and_verifies_the_owned_identity() {
    let server = Server::start(vec![Box::new(|request| {
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("Bearer fixture-profile-token")
        );
        TestResponse::json(json!({"user":{"permissionId":"fixture-owner"}}))
    })])
    .await;
    let drive = Drive::connect_fixture(server.base.clone(), NAMESPACE.into(), Some(PRINCIPAL))
        .await
        .unwrap();
    assert_eq!(drive.principal(), PRINCIPAL);
    assert_eq!(server.finish().await.len(), 1);
    let server = Server::start(vec![identity()]).await;
    assert!(matches!(
        Drive::connect_fixture(server.base.clone(), NAMESPACE.into(), Some("drive:another")).await,
        Err(Error::Identity)
    ));
    server.finish().await;
}
fn identity() -> Step {
    value(json!({"user":{"permissionId":"fixture-owner"}}))
}
fn binding() -> history::Binding {
    history::Binding {
        namespace: NAMESPACE.into(),
        principal: PRINCIPAL.into(),
        profile: Uuid::from_u128(101),
        generation: Uuid::from_u128(102),
    }
}
fn edit() -> history::LocalEdit {
    history::LocalEdit {
        operation: Uuid::from_u128(103),
        expected_revision: 0,
        changes: vec![Change {
            action: Action::Setting {
                key: SettingKey::Appearance,
                value: json!("Dark"),
            },
            extra: Default::default(),
        }],
        resolutions: vec![],
    }
}
async fn queued(path: &std::path::Path) -> (Worker, history::Upload) {
    let worker = Worker::open(path.into(), binding()).await.unwrap();
    worker
        .request(Command::Edit { edit: edit() })
        .await
        .unwrap();
    let Reply::Upload(Some(upload)) = worker.request(Command::NextUpload).await.unwrap() else {
        panic!()
    };
    (worker, upload)
}
fn metadata(upload: &history::Upload) -> Value {
    json!({
        "id": FILE_ID, "name": format!("shep-profile-{}.json", upload.operation),
        "trashed": false, "ownedByMe": true, "spaces": ["appDataFolder"],
        "mimeType": "application/json", "size": upload.record.len().to_string(),
        "appProperties": {
            "shepType": "profile", "shepFormat": "operation-v1",
            "shepNamespace": wire::sha256(NAMESPACE.as_bytes()),
            "shepProfile": binding().profile, "shepGeneration": binding().generation,
            "shepOperation": upload.operation, "shepSha256": upload.sha256,
        }
    })
}
fn reservation() -> Step {
    value(json!({"space":"appDataFolder", "ids":[FILE_ID]}))
}
async fn state(worker: &Worker) -> history::State {
    let Reply::State(state) = worker.request(Command::State).await.unwrap() else {
        panic!()
    };
    state
}
fn file(upload: &history::Upload) -> File {
    wire::file(&metadata(upload), PRINCIPAL, NAMESPACE).unwrap()
}

#[tokio::test]
async fn identity_is_verified_per_grant_and_wrong_account_never_lists_or_uploads() {
    let server = Server::start(vec![identity()]).await;
    assert!(matches!(
        server.connect(Some("drive:other-owner")).await,
        Err(Error::Identity)
    ));
    let requests = server.finish().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url.path(), "/drive/v3/about");
    assert_eq!(
        requests[0].headers["authorization"],
        "Bearer synthetic-access-token"
    );
    assert_eq!(
        requests[0].url.query_pairs().collect::<Vec<_>>(),
        vec![("fields".into(), "user(permissionId)".into())]
    );

    for body in [json!({}), json!({"user":{"permissionId":"../foreign"}})] {
        let server = Server::start(vec![value(body)]).await;
        assert!(matches!(server.connect(None).await, Err(Error::Invalid)));
        server.finish().await;
    }
}

#[tokio::test]
async fn failed_and_redirected_authorization_never_follow_or_expose_provider_bodies() {
    let target = Server::start(vec![]).await;
    for status in [302, 307, 401, 403, 500] {
        let server = Server::start(vec![reply(
            TestResponse::new(status, b"private provider diagnostic".to_vec())
                .header("Location", target.base.as_str()),
        )])
        .await;
        let error = server.connect(None).await.err().unwrap();
        assert!(!error.to_string().contains("private"));
        assert!(!error.to_string().contains("synthetic-access-token"));
        assert!(matches!(
            error,
            Error::Http(302 | 307 | 500) | Error::Authorization | Error::Denied
        ));
        assert_eq!(server.finish().await.len(), 1);
    }
    assert!(target.finish().await.is_empty());
}

#[tokio::test]
async fn paged_discovery_preserves_empty_partial_pages_and_validates_owned_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let (worker, upload) = queued(&directory.path().join("history.sqlite")).await;
    let server = Server::start(vec![
        identity(),
        value(json!({"incompleteSearch":false,"files":[],"nextPageToken":"next + / ="})),
        value(json!({"incompleteSearch":false,"files":[metadata(&upload)]})),
    ])
    .await;
    let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
    let page = drive.list_page(None).await.unwrap();
    assert!(page.files.is_empty());
    assert!(page.next.is_some());
    let page = drive.list_page(page.next.as_deref()).await.unwrap();
    assert!(page.next.is_none());
    assert_eq!(page.files, vec![file(&upload)]);
    let requests = server.finish().await;
    let query: std::collections::HashMap<_, _> = requests[2].url.query_pairs().collect();
    assert_eq!(query["spaces"], "appDataFolder");
    assert_eq!(query["pageSize"], "50");
    assert_eq!(query["corpora"], "user");
    assert_eq!(query["pageToken"], "next + / =");
    assert!(query["q"].contains("shepType"));
    assert!(!query["q"].contains("shepNamespace"));
    worker.close().await.unwrap();
}

#[tokio::test]
async fn incomplete_duplicate_oversized_and_invalid_pages_never_look_empty() {
    let directory = tempfile::tempdir().unwrap();
    let (worker, upload) = queued(&directory.path().join("history.sqlite")).await;
    for body in [
        json!({"incompleteSearch":true,"files":[]}),
        json!({"files":[]}),
        json!({"incompleteSearch":false}),
        json!({"incompleteSearch":false,"files":[],"nextPageToken":"same"}),
        json!({"incompleteSearch":false,"files":[],"nextPageToken":""}),
        json!({"incompleteSearch":false,"files":[],"nextPageToken":null}),
        json!({"incompleteSearch":false,"files":[metadata(&upload), metadata(&upload)]}),
        json!({"incompleteSearch":false,"files":vec![metadata(&upload); PAGE_SIZE+1]}),
    ] {
        let server = Server::start(vec![identity(), value(body)]).await;
        let drive = server.connect(None).await.unwrap();
        assert!(drive.list_page(Some("same")).await.is_err());
        assert!(drive.list_page(Some("bad\ntoken")).await.is_err());
        assert_eq!(server.finish().await.len(), 2);
    }
    worker.close().await.unwrap();
}

#[tokio::test]
async fn unrelated_namespace_future_format_unowned_files_and_lying_checksums_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let (worker, upload) = queued(&directory.path().join("history.sqlite")).await;
    for (pointer, replacement) in [
        ("/ownedByMe", json!(false)),
        ("/ownedByMe", Value::Null),
        ("/trashed", json!(true)),
        ("/spaces", json!(["drive"])),
        ("/mimeType", json!("application/vnd.google-apps.shortcut")),
        ("/id", json!("../other")),
        ("/name", json!("similar-profile.json")),
        ("/appProperties/shepType", json!("backup")),
        ("/appProperties/shepFormat", json!("operation-v2")),
        (
            "/appProperties/shepNamespace",
            json!(wire::sha256(b"another-project")),
        ),
        ("/appProperties/shepProfile", json!(Uuid::nil())),
        ("/appProperties/shepGeneration", json!(Uuid::nil())),
        ("/appProperties/shepOperation", json!(Uuid::nil())),
        ("/appProperties/shepSha256", json!("not-a-digest")),
        ("/size", json!((crate::MAX_RECORD_BYTES + 1).to_string())),
    ] {
        let mut meta = metadata(&upload);
        *meta.pointer_mut(pointer).unwrap() = replacement;
        let server = Server::start(vec![
            identity(),
            value(json!({"incompleteSearch":false,"files":[meta]})),
        ])
        .await;
        let drive = server.connect(None).await.unwrap();
        let error = drive.list_page(None).await.err().unwrap();
        if pointer.ends_with("shepNamespace") {
            assert!(matches!(error, Error::Namespace));
        }
        if pointer.ends_with("shepFormat") {
            assert!(matches!(error, Error::Record(crate::Error::Upgrade)));
        }
        server.finish().await;
    }
    let mut meta = metadata(&upload);
    meta["sha256Checksum"] = json!("0".repeat(64));
    assert!(matches!(
        wire::file(&meta, PRINCIPAL, NAMESPACE),
        Err(Error::Changed)
    ));
    worker.close().await.unwrap();
}

#[tokio::test]
async fn download_imports_exact_bytes_into_an_independent_device_after_ownership_checks() {
    let directory = tempfile::tempdir().unwrap();
    let (first, upload) = queued(&directory.path().join("first.sqlite")).await;
    let second = Worker::open(directory.path().join("second.sqlite"), binding())
        .await
        .unwrap();
    let server = Server::start(vec![
        identity(),
        value(metadata(&upload)),
        reply(TestResponse::new(200, upload.record.as_bytes().to_vec())),
    ])
    .await;
    let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
    let state = drive.import(&second, &file(&upload)).await.unwrap();
    assert_eq!(state.operations, 1);
    assert_eq!(state.queued, 0);
    assert_eq!(state.fields, 1);
    let requests = server.finish().await;
    assert!(requests.iter().all(|r| r.method == "GET"));
    assert_eq!(requests[2].url.query(), Some("alt=media"));
    let Reply::Value(change) = second
        .request(Command::Value {
            target: "setting:appearance".into(),
            operation: upload.operation,
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(change.action, edit().changes[0].action);
    first.close().await.unwrap();
    second.close().await.unwrap();
}

#[tokio::test]
async fn downloaded_hash_identity_size_and_metadata_changes_never_touch_history() {
    let directory = tempfile::tempdir().unwrap();
    let (first, upload) = queued(&directory.path().join("first.sqlite")).await;
    let second = Worker::open(directory.path().join("second.sqlite"), binding())
        .await
        .unwrap();
    for suffix in ["different", " "] {
        let bytes = format!("{}{suffix}", upload.record).into_bytes();
        let server = Server::start(vec![
            identity(),
            value(metadata(&upload)),
            reply(TestResponse::new(200, bytes)),
        ])
        .await;
        let drive = server.connect(None).await.unwrap();
        assert!(drive.import(&second, &file(&upload)).await.is_err());
        assert_eq!(state(&second).await.operations, 0);
        server.finish().await;
    }
    let mut changed = metadata(&upload);
    changed["appProperties"]["shepSha256"] = json!("0".repeat(64));
    let server = Server::start(vec![identity(), value(changed)]).await;
    let drive = server.connect(None).await.unwrap();
    assert!(matches!(
        drive.import(&second, &file(&upload)).await,
        Err(Error::Changed)
    ));
    assert_eq!(server.finish().await.len(), 2);
    assert_eq!(state(&second).await.operations, 0);

    // Valid bytes/hash can still belong to another profile than their metadata.
    let mut op = Operation::decode(upload.record.as_bytes()).unwrap();
    op.profile = Uuid::from_u128(999);
    let bytes = op.encode().unwrap();
    let foreign = history::Upload {
        record: String::from_utf8(bytes.clone()).unwrap(),
        sha256: wire::sha256(&bytes),
        ..upload.clone()
    };
    let server = Server::start(vec![
        identity(),
        value(metadata(&foreign)),
        reply(TestResponse::new(200, bytes)),
    ])
    .await;
    let drive = server.connect(None).await.unwrap();
    assert!(matches!(
        drive.import(&second, &file(&foreign)).await,
        Err(Error::Changed)
    ));
    assert_eq!(state(&second).await.operations, 0);
    server.finish().await;
    first.close().await.unwrap();
    second.close().await.unwrap();
}

#[tokio::test]
async fn upload_reservation_is_durable_before_post_and_exact_content_confirms_the_queue() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.sqlite");
    let (worker, upload) = queued(&path).await;
    let original = upload.record.as_bytes().to_vec();
    let expected_digest = upload.sha256.clone();
    let server = Server::start(vec![
        identity(),
        reservation(),
        absent(),
        Box::new(move |request| {
            let db = rusqlite::Connection::open(&path).unwrap();
            let saved: (String, String) = db
                .query_row(
                    "SELECT file_id,sha256 FROM operations WHERE local=1 AND uploaded=0",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(saved, (FILE_ID.into(), expected_digest));
            assert_eq!(request.method, "POST");
            assert_eq!(request.url.path(), "/upload/drive/v3/files");
            assert_eq!(
                request
                    .url
                    .query_pairs()
                    .find(|(k, _)| k == "uploadType")
                    .unwrap()
                    .1,
                "multipart"
            );
            let boundary = request.headers["content-type"]
                .strip_prefix("multipart/related; boundary=")
                .unwrap();
            let start =
                format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n");
            let split = format!("\r\n--{boundary}\r\nContent-Type: application/json\r\n\r\n");
            assert!(request.body.starts_with(start.as_bytes()));
            let split_at = request
                .body
                .windows(split.len())
                .position(|w| w == split.as_bytes())
                .unwrap();
            let meta: Value = serde_json::from_slice(&request.body[start.len()..split_at]).unwrap();
            assert_eq!(meta["id"], FILE_ID);
            assert_eq!(meta["parents"], json!(["appDataFolder"]));
            assert_eq!(meta["appProperties"]["shepSha256"], saved.1);
            for (key, val) in meta["appProperties"].as_object().unwrap() {
                assert!(key.len() + val.as_str().unwrap().len() <= 124);
            }
            let media_start = split_at + split.len();
            assert_eq!(
                &request.body[media_start..media_start + original.len()],
                original
            );
            assert_eq!(
                &request.body[media_start + original.len()..],
                format!("\r\n--{boundary}--\r\n").as_bytes()
            );
            TestResponse::json(json!({"id":"untrusted-upload-response-id"}))
        }),
        value(metadata(&upload)),
        reply(TestResponse::new(200, upload.record.as_bytes().to_vec())),
    ])
    .await;
    let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
    assert_eq!(
        drive.upload_next(&worker).await.unwrap(),
        Some(upload.operation)
    );
    assert_eq!(state(&worker).await.queued, 0);
    assert_eq!(drive.upload_next(&worker).await.unwrap(), None);
    assert_eq!(
        server
            .finish()
            .await
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
    worker.close().await.unwrap();
}

#[tokio::test]
async fn lost_upload_reply_reopens_and_confirms_the_same_reserved_file_without_another_post() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.sqlite");
    let (worker, upload) = queued(&path).await;
    let server = Server::start(vec![
        identity(),
        reservation(),
        absent(),
        reply(TestResponse::new(0, vec![])),
        absent(),
        identity(),
        value(metadata(&upload)),
        reply(TestResponse::new(200, upload.record.as_bytes().to_vec())),
    ])
    .await;
    let drive = server.connect(None).await.unwrap();
    assert!(matches!(
        drive.upload_next(&worker).await,
        Err(Error::Unconfirmed)
    ));
    assert_eq!(state(&worker).await.queued, 1);
    worker.close().await.unwrap();
    let worker = Worker::open(path, binding()).await.unwrap();
    let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
    assert_eq!(
        drive.upload_next(&worker).await.unwrap(),
        Some(upload.operation)
    );
    assert_eq!(state(&worker).await.queued, 0);
    let requests = server.finish().await;
    assert_eq!(requests.iter().filter(|r| r.method == "POST").count(), 1);
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("generateIds"))
            .count(),
        1
    );
    worker.close().await.unwrap();
}

#[tokio::test]
async fn conflict_and_success_responses_without_matching_owned_bytes_do_not_acknowledge() {
    for status in [200, 409, 500] {
        let directory = tempfile::tempdir().unwrap();
        let (worker, _) = queued(&directory.path().join("history.sqlite")).await;
        let server = Server::start(vec![
            identity(),
            reservation(),
            absent(),
            reply(TestResponse::new(status, vec![])),
            absent(),
        ])
        .await;
        let drive = server.connect(None).await.unwrap();
        assert!(matches!(
            drive.upload_next(&worker).await,
            Err(Error::Unconfirmed)
        ));
        assert_eq!(state(&worker).await.queued, 1);
        assert_eq!(
            server
                .finish()
                .await
                .iter()
                .filter(|r| r.method == "POST")
                .count(),
            1
        );
        worker.close().await.unwrap();
    }
}

#[tokio::test]
async fn cancellation_during_upload_retains_reservation_and_cannot_acknowledge_another_identity() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("history.sqlite");
    let (worker, upload) = queued(&path).await;
    let (entered, received) = tokio::sync::oneshot::channel();
    let (release, gate) = tokio::sync::oneshot::channel();
    let server = Server::start(vec![
        identity(),
        reservation(),
        absent(),
        Box::new(move |_| {
            entered.send(()).unwrap();
            TestResponse::new(200, vec![]).held(gate)
        }),
        identity(),
        value(metadata(&upload)),
        reply(TestResponse::new(200, upload.record.as_bytes().to_vec())),
    ])
    .await;
    let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
    let cloned = worker.clone();
    let task = tokio::spawn(async move { drive.upload_next(&cloned).await });
    tokio::time::timeout(Duration::from_secs(5), received)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(state(&worker).await.queued, 1);
    release.send(()).unwrap();
    worker.close().await.unwrap();
    let worker = Worker::open(path, binding()).await.unwrap();
    let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
    let mut other_binding = binding();
    other_binding.principal = "drive:other".into();
    let other = Worker::open(directory.path().join("other.sqlite"), other_binding)
        .await
        .unwrap();
    assert!(matches!(
        drive.upload_next(&other).await,
        Err(Error::Identity)
    ));
    assert_eq!(
        drive.upload_next(&worker).await.unwrap(),
        Some(upload.operation)
    );
    assert_eq!(state(&other).await.operations, 0);
    let requests = server.finish().await;
    assert_eq!(requests.iter().filter(|r| r.method == "POST").count(), 1);
    other.close().await.unwrap();
    worker.close().await.unwrap();
}

#[tokio::test]
async fn response_limits_apply_to_declared_chunked_and_duplicate_key_json() {
    let large = vec![b' '; JSON_LIMIT + 1];
    let mut chunked = format!("{:X}\r\n", large.len()).into_bytes();
    chunked.extend(&large);
    chunked.extend(b"\r\n0\r\n\r\n");
    for response in [
        TestResponse::new(200, large),
        TestResponse::new(200, chunked).header("Transfer-Encoding", "chunked"),
        TestResponse::new(
            200,
            br#"{"user":{"permissionId":"other","permissionId":"fixture-owner"}}"#.to_vec(),
        ),
    ] {
        let server = Server::start(vec![reply(response)]).await;
        assert!(matches!(
            server.connect(None).await,
            Err(Error::TooLarge | Error::Invalid)
        ));
        server.finish().await;
    }
}

#[tokio::test]
async fn failed_preflight_cannot_post_and_a_reserved_file_cannot_be_overwritten() {
    for status in [301, 401, 403, 429, 500] {
        let directory = tempfile::tempdir().unwrap();
        let (worker, _) = queued(&directory.path().join("history.sqlite")).await;
        let server = Server::start(vec![
            identity(),
            reservation(),
            reply(TestResponse::new(status, b"private details".to_vec())),
        ])
        .await;
        let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
        assert!(drive.upload_next(&worker).await.is_err());
        assert_eq!(state(&worker).await.queued, 1);
        assert!(server.finish().await.iter().all(|r| r.method == "GET"));
        worker.close().await.unwrap();
    }
    let directory = tempfile::tempdir().unwrap();
    let (worker, upload) = queued(&directory.path().join("history.sqlite")).await;
    let mut foreign = metadata(&upload);
    foreign["appProperties"]["shepSha256"] = json!("0".repeat(64));
    let server = Server::start(vec![identity(), reservation(), value(foreign)]).await;
    let drive = server.connect(None).await.unwrap();
    assert!(matches!(
        drive.upload_next(&worker).await,
        Err(Error::Changed)
    ));
    assert_eq!(state(&worker).await.queued, 1);
    assert!(server.finish().await.iter().all(|r| r.method == "GET"));
    worker.close().await.unwrap();
}

#[tokio::test]
async fn interrupted_create_retries_exact_reserved_id_and_media_then_verifies_a_conflict() {
    let directory = tempfile::tempdir().unwrap();
    let (worker, upload) = queued(&directory.path().join("history.sqlite")).await;
    let server = Server::start(vec![
        identity(),
        reservation(),
        absent(),
        reply(TestResponse::new(0, vec![])),
        absent(),
        absent(),
        reply(TestResponse::new(409, vec![])),
        value(metadata(&upload)),
        reply(TestResponse::new(200, upload.record.as_bytes().to_vec())),
    ])
    .await;
    let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
    assert!(matches!(
        drive.upload_next(&worker).await,
        Err(Error::Unconfirmed)
    ));
    assert_eq!(state(&worker).await.queued, 1);
    assert_eq!(
        drive.upload_next(&worker).await.unwrap(),
        Some(upload.operation)
    );
    assert_eq!(state(&worker).await.queued, 0);
    let requests = server.finish().await;
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("generateIds"))
            .count(),
        1
    );
    let posts = requests
        .iter()
        .filter(|r| r.method == "POST")
        .collect::<Vec<_>>();
    assert_eq!(posts.len(), 2);
    for post in posts {
        let text = String::from_utf8(post.body.clone()).unwrap();
        assert!(text.contains(&format!("\"id\":\"{FILE_ID}\"")));
        assert!(text.contains(&upload.record));
    }
    worker.close().await.unwrap();
}

#[tokio::test]
async fn correct_metadata_cannot_confirm_corrupt_or_truncated_media() {
    for variant in 0..3 {
        let directory = tempfile::tempdir().unwrap();
        let (worker, upload) = queued(&directory.path().join("history.sqlite")).await;
        let mut bytes = upload.record.as_bytes().to_vec();
        let response = match variant {
            0 => {
                bytes[0] = b'[';
                TestResponse::new(200, bytes)
            }
            1 => {
                bytes.pop();
                TestResponse::new(200, bytes)
            }
            _ => {
                bytes.push(b' ');
                let mut chunks = format!("{:X}\r\n", bytes.len()).into_bytes();
                chunks.extend(bytes);
                chunks.extend(b"\r\n0\r\n\r\n");
                TestResponse::new(200, chunks).header("Transfer-Encoding", "chunked")
            }
        };
        let server = Server::start(vec![
            identity(),
            reservation(),
            value(metadata(&upload)),
            reply(response),
        ])
        .await;
        let drive = server.connect(None).await.unwrap();
        assert!(matches!(
            drive.upload_next(&worker).await,
            Err(Error::Changed | Error::TooLarge)
        ));
        assert_eq!(state(&worker).await.queued, 1);
        assert!(server.finish().await.iter().all(|r| r.method == "GET"));
        worker.close().await.unwrap();
    }
}

#[tokio::test]
async fn malformed_reservations_and_bindings_never_write_or_send() {
    for body in [
        json!({"space":"drive","ids":[FILE_ID]}),
        json!({"space":"appDataFolder","ids":[]}),
        json!({"space":"appDataFolder","ids":[FILE_ID,"other"]}),
        json!({"space":"appDataFolder","ids":["../redirect"]}),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (worker, _) = queued(&directory.path().join("history.sqlite")).await;
        let server = Server::start(vec![identity(), value(body)]).await;
        let drive = server.connect(None).await.unwrap();
        assert!(matches!(
            drive.upload_next(&worker).await,
            Err(Error::Invalid)
        ));
        let Reply::Upload(Some(upload)) = worker.request(Command::NextUpload).await.unwrap() else {
            panic!()
        };
        assert!(upload.file_id.is_none());
        assert_eq!(state(&worker).await.queued, 1);
        assert_eq!(server.finish().await.len(), 2);
        worker.close().await.unwrap();
    }
    let directory = tempfile::tempdir().unwrap();
    let (worker, upload) = queued(&directory.path().join("history.sqlite")).await;
    let mut wrong = binding();
    wrong.namespace = "so.shep.other".into();
    let other = Worker::open(directory.path().join("other.sqlite"), wrong)
        .await
        .unwrap();
    let server = Server::start(vec![identity()]).await;
    let drive = server.connect(None).await.unwrap();
    assert!(matches!(
        drive.upload_next(&other).await,
        Err(Error::Namespace)
    ));
    assert!(matches!(
        drive.import(&other, &file(&upload)).await,
        Err(Error::Namespace)
    ));
    let mut file = file(&upload);
    file.profile = Uuid::from_u128(666);
    assert!(matches!(
        drive.import(&worker, &file).await,
        Err(Error::History(history::Error::Binding))
    ));
    assert_eq!(server.finish().await.len(), 1);
    other.close().await.unwrap();
    worker.close().await.unwrap();
}

#[tokio::test]
async fn common_wire_fixture_preserves_raw_bytes_and_does_not_claim_missing_ancestry_complete() {
    let raw = include_bytes!("../../../../profile-operation.json");
    let meta: Value =
        serde_json::from_str(include_str!("../../../../profile-drive-file.json")).unwrap();
    let op = Operation::decode(raw).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let worker = Worker::open(
        directory.path().join("history.sqlite"),
        history::Binding {
            namespace: op.namespace.clone(),
            principal: PRINCIPAL.into(),
            profile: op.profile,
            generation: op.generation,
        },
    )
    .await
    .unwrap();
    let server = Server::start(vec![
        identity(),
        value(json!({"incompleteSearch":false,"files":[meta.clone()]})),
        value(meta),
        reply(TestResponse::new(200, raw.to_vec())),
    ])
    .await;
    let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
    let page = drive.list_page(None).await.unwrap();
    assert!(page.next.is_none());
    let state = drive.import(&worker, &page.files[0]).await.unwrap();
    assert_eq!(state.operations, 1);
    assert_eq!(state.waiting, 1);
    assert_eq!(state.fields, 0);
    assert!(matches!(
        worker.request(Command::Edit { edit: edit() }).await,
        Err(history::Error::Incomplete)
    ));
    server.finish().await;
    worker.close().await.unwrap();
}

#[tokio::test]
async fn publication_records_own_file_before_ack_and_recovers_failed_catalog_receipt_without_posting_twice()
 {
    let directory = tempfile::tempdir().unwrap();
    let (worker, upload) = queued(&directory.path().join("local.sqlite")).await;
    let path = directory.path().join("catalog.sqlite");
    let scope = catalog::Scope {
        namespace: NAMESPACE.into(),
        principal: PRINCIPAL.into(),
    };
    let catalog = catalog::Discovery::open(path.clone(), scope.clone())
        .await
        .unwrap();
    let server = Server::start(vec![
        identity(),
        reservation(),
        reply(TestResponse::new(404, vec![])),
        value(json!({"id":FILE_ID})),
        value(metadata(&upload)),
        reply(TestResponse::new(200, upload.record.clone().into_bytes())),
        value(metadata(&upload)),
        reply(TestResponse::new(200, upload.record.clone().into_bytes())),
        value(json!({"startPageToken":"before-rescan"})),
        value(json!({"incompleteSearch":false,"files":[]})),
        value(json!({"changes":[],"newStartPageToken":"after-rescan"})),
    ])
    .await;
    let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_profile_receipt BEFORE INSERT ON profiles BEGIN SELECT RAISE(ABORT,'synthetic failed receipt'); END;").unwrap();
    assert!(matches!(
        drive.upload_next_tracked(&worker, &catalog).await,
        Err(Error::DiscoveryReceipt)
    ));
    assert_eq!(state(&worker).await.queued, 1);
    assert_eq!(catalog.state().await.unwrap().files, 1);
    db.execute_batch("DROP TRIGGER fail_profile_receipt")
        .unwrap();
    drop(db);
    catalog.close().await.unwrap();
    let catalog = catalog::Discovery::open(path, scope).await.unwrap();
    assert_eq!(
        drive.upload_next_tracked(&worker, &catalog).await.unwrap(),
        Some(upload.operation)
    );
    assert_eq!(state(&worker).await.queued, 0);
    let saved = catalog.state().await.unwrap();
    assert_eq!(saved.files, 1);
    catalog.refresh(saved.revision, true).await.unwrap();
    let mut missing = false;
    for _ in 0..8 {
        match catalog.advance(&drive).await {
            Err(catalog::Error::Missing) => {
                missing = true;
                break;
            }
            Ok(_) => {}
            Err(error) => panic!("{error}"),
        }
    }
    assert!(
        missing,
        "own acknowledged files cannot disappear silently on rescan"
    );
    let requests = server.finish().await;
    assert_eq!(requests.iter().filter(|r| r.method == "POST").count(), 1);
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("generateIds"))
            .count(),
        1
    );
    catalog.close().await.unwrap();
    worker.close().await.unwrap();
}

#[tokio::test]
async fn external_fixture_entry_rejects_unowned_endpoints_and_uses_only_a_fake_token() {
    for url in [
        "https://www.googleapis.com/",
        "http://example.test:8080/",
        "http://127.0.0.1/",
        "http://127.0.0.1:1234/path",
        "http://user@127.0.0.1:1234/",
        "http://127.0.0.1:1234/?token=forbidden",
        "http://127.0.0.1:1234/#fragment",
    ] {
        assert!(matches!(
            Drive::connect_fixture(Url::parse(url).unwrap(), NAMESPACE.into(), Some(PRINCIPAL))
                .await,
            Err(Error::Invalid)
        ));
    }
    let server = Server::start(vec![Box::new(|request| {
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("Bearer fixture-profile-token")
        );
        TestResponse::json(json!({"user":{"permissionId":"fixture-owner"}}))
    })])
    .await;
    let drive = Drive::connect_fixture(server.base.clone(), NAMESPACE.into(), Some(PRINCIPAL))
        .await
        .unwrap();
    assert_eq!(drive.principal(), PRINCIPAL);
    server.finish().await;
    let server = Server::start(vec![identity()]).await;
    assert!(matches!(
        Drive::connect_fixture(server.base.clone(), NAMESPACE.into(), Some("drive:another")).await,
        Err(Error::Identity)
    ));
    server.finish().await;
}
