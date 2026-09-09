use super::*;
use crate::providers::drive_http::JSON_LIMIT;
use crate::providers::test_http::{self, Reply, Server};

const SESSION: &str = "/upload/drive/v3/files?uploadType=resumable&upload_id=fixture-session";
fn name() -> String {
    format!("shep-20260906T120000Z-{}.shepbackup", uuid::Uuid::nil())
}
fn prepared(data: &[u8]) -> PreparedUpload {
    PreparedUpload::new("reserved".into(), name(), data)
}
fn file(upload: &PreparedUpload) -> Value {
    json!({"id":upload.id,"name":upload.name,"createdTime":"2026-09-06T12:00:00Z","trashed":false,"spaces":["appDataFolder"],"mimeType":"application/octet-stream",
        "appProperties":{"shepBackup":"1","shepSha256":upload.sha256},"size":upload.size.to_string(),"sha256Checksum":upload.sha256})
}
fn api<'a>(http: &'a reqwest::Client, server: &Server) -> Api<'a> {
    Api {
        http,
        base: server.url.clone(),
        token: "fixture-token",
    }
}

#[tokio::test]
async fn drive_reserves_one_appdata_id_and_recovers_a_partially_received_chunk() {
    let data: Vec<u8> = (0..CHUNK + 17).map(|i| (i % 251) as u8).collect();
    let expected = prepared(&data);
    let mut server = Server::start(vec![
        Reply::new(
            200,
            json!({"ids":["reserved"],"space":"appDataFolder"}).to_string(),
        ),
        Reply::new(404, ""),
        Reply::new(200, "").header("Location", SESSION),
        Reply::disconnect(),
        Reply::new(308, "").header("Range", "bytes=0-524287"),
        Reply::new(201, file(&expected).to_string()),
    ])
    .await;
    let http = test_http::client();
    let api = api(&http, &server);
    let mut upload = api.reserve(&name(), &data).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("uploads.sqlite");
    let journal = journal::Journal::open(Some(&path)).unwrap();
    let target = BackupTarget::GoogleDrive {
        client_id: "fixture-client".into(),
        connection_id: "drive:fixture".into(),
    };
    journal
        .prepare(&target, upload.clone(), data.clone())
        .await
        .unwrap();
    api.upload(
        &mut upload,
        &data,
        &journal::Checkpoint {
            journal: journal.clone(),
            target: target.clone(),
        },
    )
    .await
    .unwrap();
    server.finish().await;
    let requests = server.requests();
    assert!(requests[0].target.contains("space=appDataFolder"));
    assert!(requests[2].body.contains("\"id\":\"reserved\""));
    assert_eq!(requests[3].bytes, data[..CHUNK]);
    assert!(requests[4].bytes.is_empty());
    assert_eq!(
        requests[4].headers["content-range"],
        format!("bytes */{}", data.len())
    );
    assert_eq!(
        requests[5].headers["content-range"],
        format!("bytes 524288-{}/{}", data.len() - 1, data.len())
    );
    assert_eq!(requests[5].bytes, data[524288..]);
    assert!(
        requests
            .iter()
            .all(|request| request.headers["authorization"] == "Bearer fixture-token")
    );
    drop(journal);
    let reopened = journal::Journal::open(Some(&path))
        .unwrap()
        .pending(&target)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reopened.upload.id, "reserved");
    assert!(reopened.upload.session.unwrap().contains("fixture-session"));
    assert_eq!(reopened.data, data);
}

#[tokio::test]
async fn drive_resumes_the_same_persisted_archive_after_an_app_restart() {
    let data = vec![0xa3; CHUNK + 53];
    let mut upload = prepared(&data);
    let mut server = Server::start(vec![
        Reply::new(404, ""),
        Reply::new(200, "").header("Location", SESSION),
        Reply::new(308, "").header("Range", &format!("bytes=0-{}", CHUNK - 1)),
        Reply::disconnect(),
        Reply::new(503, ""),
        Reply::new(503, ""),
        Reply::new(503, ""),
        Reply::new(404, ""),
        Reply::new(308, "").header("Range", &format!("bytes=0-{}", CHUNK - 1)),
        Reply::new(201, file(&upload).to_string()),
    ])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("uploads.sqlite");
    let target = BackupTarget::Local("fixture-target".into());
    {
        let journal = journal::Journal::open(Some(&path)).unwrap();
        journal
            .prepare(&target, upload.clone(), data.clone())
            .await
            .unwrap();
        let http = test_http::client();
        assert!(
            api(&http, &server)
                .upload(
                    &mut upload,
                    &data,
                    &journal::Checkpoint {
                        journal,
                        target: target.clone()
                    }
                )
                .await
                .is_err()
        );
    }
    let reopened = journal::Journal::open(Some(&path)).unwrap();
    let mut pending = reopened.pending(&target).await.unwrap().unwrap();
    let http = test_http::client();
    api(&http, &server)
        .upload(
            &mut pending.upload,
            &pending.data,
            &journal::Checkpoint {
                journal: reopened,
                target,
            },
        )
        .await
        .unwrap();
    server.finish().await;
    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    assert!(requests[8].bytes.is_empty());
    assert_eq!(requests[9].bytes, data[CHUNK..]);
}

#[tokio::test]
async fn drive_restarts_an_expired_session_with_the_same_id_and_verifies_conflicts() {
    let data = b"encrypted archive";
    let mut upload = prepared(data);
    let mut server = Server::start(vec![
        Reply::new(404, ""),
        Reply::new(404, ""),
        Reply::new(404, ""),
        Reply::new(409, ""),
        Reply::new(200, file(&upload).to_string()),
    ])
    .await;
    upload.session = Some(server.url.join(SESSION).unwrap().into());
    let http = test_http::client();
    api(&http, &server)
        .upload(&mut upload, data, &NoCheckpoint)
        .await
        .unwrap();
    server.finish().await;
    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    assert!(requests[3].body.contains("\"id\":\"reserved\""));
    assert!(
        requests
            .iter()
            .filter(|request| request.method == "PUT")
            .all(|request| request.bytes.is_empty())
    );
}

#[tokio::test]
async fn drive_recovers_a_lost_final_acknowledgment_and_malformed_response() {
    let data = b"encrypted archive";
    let mut upload = prepared(data);
    let mut server = Server::start(vec![
        Reply::new(404, ""),
        Reply::new(200, "").header("Location", SESSION),
        Reply::disconnect(),
        Reply::new(200, "truncated JSON"),
        Reply::new(200, file(&upload).to_string()),
    ])
    .await;
    let http = test_http::client();
    api(&http, &server)
        .upload(&mut upload, data, &NoCheckpoint)
        .await
        .unwrap();
    server.finish().await;
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|request| !request.bytes.is_empty() && request.method == "PUT")
            .count(),
        1
    );
}

struct FailedCheckpoint;
#[async_trait]
impl UploadCheckpoint for FailedCheckpoint {
    async fn save(&self, _: &PreparedUpload) -> anyhow::Result<()> {
        anyhow::bail!("disk full")
    }
}
#[tokio::test]
async fn drive_never_sends_bytes_before_the_session_is_durable_or_to_another_origin() {
    let data = b"encrypted archive";
    let mut server = Server::start(vec![
        Reply::new(404, ""),
        Reply::new(200, "").header("Location", SESSION),
    ])
    .await;
    let http = test_http::client();
    assert!(
        api(&http, &server)
            .upload(&mut prepared(data), data, &FailedCheckpoint)
            .await
            .unwrap_err()
            .to_string()
            .contains("disk full")
    );
    server.finish().await;
    assert!(
        server
            .requests()
            .iter()
            .all(|request| request.method != "PUT")
    );
    for location in [
        "https://example.com/upload/drive/v3/files?uploadType=resumable&upload_id=bad",
        "/drive/v3/files/unrelated",
        "http://user@127.0.0.1/upload/drive/v3/files?uploadType=resumable&upload_id=bad",
    ] {
        let mut server = Server::start(vec![
            Reply::new(404, ""),
            Reply::new(200, "").header("Location", location),
        ])
        .await;
        assert!(
            api(&http, &server)
                .upload(&mut prepared(data), data, &NoCheckpoint)
                .await
                .is_err()
        );
        server.finish().await;
        assert!(
            server
                .requests()
                .iter()
                .all(|request| request.method != "PUT")
        );
    }
}

#[tokio::test]
async fn drive_rejects_impossible_ranges_and_stops_when_no_bytes_are_acknowledged() {
    let data = b"encrypted archive";
    let http = test_http::client();
    for range in ["bytes=0-999", "bytes=1-4", "invalid"] {
        let mut server = Server::start(vec![
            Reply::new(404, ""),
            Reply::new(200, "").header("Location", SESSION),
            Reply::new(308, "").header("Range", range),
        ])
        .await;
        assert!(
            api(&http, &server)
                .upload(&mut prepared(data), data, &NoCheckpoint)
                .await
                .is_err()
        );
        server.finish().await;
    }
    let mut replies = vec![
        Reply::new(404, ""),
        Reply::new(200, "").header("Location", SESSION),
    ];
    replies.extend((0..4).map(|_| Reply::new(308, "")));
    let mut server = Server::start(replies).await;
    assert!(
        api(&http, &server)
            .upload(&mut prepared(data), data, &NoCheckpoint)
            .await
            .unwrap_err()
            .to_string()
            .contains("progress")
    );
    server.finish().await;
    let mut server = Server::start(vec![
        Reply::new(404, ""),
        Reply::new(200, "").header("Location", SESSION),
        Reply::new(308, "").header("Range", "bytes=0-7"),
        Reply::new(308, "").header("Range", "bytes=0-3"),
    ])
    .await;
    assert!(
        api(&http, &server)
            .upload(&mut prepared(data), data, &NoCheckpoint)
            .await
            .unwrap_err()
            .to_string()
            .contains("previously acknowledged")
    );
    server.finish().await;
}

#[tokio::test]
async fn drive_lists_all_pages_but_rejects_duplicate_tokens_ids_and_incomplete_searches() {
    let upload = prepared(b"archive");
    let mut other = upload.clone();
    other.id = "second".into();
    let http = test_http::client();
    let mut server = Server::start(vec![
        Reply::new(
            200,
            json!({"files":[file(&upload)],"nextPageToken":"next"}).to_string(),
        ),
        Reply::new(200, json!({"files":[file(&other)]}).to_string()),
    ])
    .await;
    assert_eq!(api(&http, &server).list().await.unwrap().len(), 2);
    server.finish().await;
    assert!(server.requests()[1].target.contains("pageToken=next"));
    for pages in [
        vec![
            json!({"files":[],"nextPageToken":"loop"}),
            json!({"files":[],"nextPageToken":"loop"}),
        ],
        vec![
            json!({"files":[file(&upload)],"nextPageToken":"next"}),
            json!({"files":[file(&upload)]}),
        ],
        vec![json!({"files":[file(&upload)],"incompleteSearch":true})],
        vec![json!({"files":{},"nextPageToken":3})],
    ] {
        let mut server = Server::start(
            pages
                .into_iter()
                .map(|value| Reply::new(200, value.to_string()))
                .collect(),
        )
        .await;
        assert!(api(&http, &server).list().await.is_err());
        server.finish().await;
    }
}

#[tokio::test]
async fn drive_checks_ownership_before_download_or_delete_and_only_accepts_success() {
    let upload = prepared(b"archive");
    let http = test_http::client();
    for field in ["spaces", "appProperties", "name", "mimeType", "trashed"] {
        let mut wrong = file(&upload);
        wrong[field] = Value::Null;
        let mut server = Server::start(vec![
            Reply::new(200, wrong.to_string()),
            Reply::new(200, wrong.to_string()),
        ])
        .await;
        assert!(api(&http, &server).delete(&upload.id).await.is_err());
        assert!(api(&http, &server).download(&upload.id).await.is_err());
        server.finish().await;
        assert!(
            server
                .requests()
                .iter()
                .all(|request| request.method == "GET" && !request.target.contains("alt=media"))
        );
    }
    for status in [204, 302, 410] {
        let mut server = Server::start(vec![
            Reply::new(200, file(&upload).to_string()),
            Reply::new(status, ""),
        ])
        .await;
        assert_eq!(
            api(&http, &server).delete(&upload.id).await.is_ok(),
            status != 302
        );
        server.finish().await;
    }
    let mut server = Server::start(vec![Reply::new(404, "")]).await;
    api(&http, &server).delete(&upload.id).await.unwrap();
    server.finish().await;
}

#[tokio::test]
async fn drive_bounds_json_and_downloads_and_checks_binary_checksums() {
    let data = vec![0, 255, 0, 128, 42];
    let upload = prepared(&data);
    let http = test_http::client();
    let mut server = Server::start(vec![
        Reply::new(200, file(&upload).to_string()),
        Reply::binary(200, data.clone()),
    ])
    .await;
    assert_eq!(
        api(&http, &server).download(&upload.id).await.unwrap(),
        data
    );
    server.finish().await;
    let mut server = Server::start(vec![
        Reply::new(200, file(&upload).to_string()),
        Reply::binary(200, vec![1; data.len()]),
    ])
    .await;
    assert!(api(&http, &server).download(&upload.id).await.is_err());
    server.finish().await;
    let mut oversized = file(&upload);
    oversized["size"] = json!((MAX_DECODED + 1).to_string());
    let mut server = Server::start(vec![Reply::new(200, oversized.to_string())]).await;
    assert!(api(&http, &server).download(&upload.id).await.is_err());
    server.finish().await;
    let mut server = Server::start(vec![
        Reply::new(200, "").header("Content-Length", &(JSON_LIMIT + 1).to_string()),
    ])
    .await;
    assert!(api(&http, &server).list().await.is_err());
    server.finish().await;
}

#[tokio::test]
async fn drive_recovers_only_identical_committed_files_and_verifies_the_account_identity() {
    let data = b"archive";
    let upload = prepared(data);
    let http = test_http::client();
    let mut wrong = file(&upload);
    wrong["sha256Checksum"] = json!("different");
    let mut server = Server::start(vec![Reply::new(200, wrong.to_string())]).await;
    assert!(
        api(&http, &server)
            .upload(&mut upload.clone(), data, &NoCheckpoint)
            .await
            .is_err()
    );
    server.finish().await;
    let mut no_digest = file(&upload);
    no_digest.as_object_mut().unwrap().remove("sha256Checksum");
    let mut server = Server::start(vec![
        Reply::new(200, no_digest.to_string()),
        Reply::binary(200, data.to_vec()),
        Reply::new(
            200,
            json!({"user":{"permissionId":"verified-user"}}).to_string(),
        ),
    ])
    .await;
    assert!(api(&http, &server).confirmed(&upload).await.unwrap());
    assert_eq!(
        api(&http, &server).identity().await.unwrap(),
        "drive:verified-user"
    );
    server.finish().await;
}
