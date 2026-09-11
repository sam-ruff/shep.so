//! A stateful loopback Drive for credential files, exercised through the
//! production session, HTTP parsing and the full reconcile pass.
use super::*;
use crate::profile_sync::vault::{
    self as passwords, Field,
    tests::{History, INCOMING, SMTP, Servers, account, device},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

type Files = BTreeMap<String, (Value, Vec<u8>)>;
use std::collections::BTreeMap;

#[derive(Clone, Default)]
struct State {
    files: Arc<Mutex<Files>>,
    next: Arc<AtomicUsize>,
    lose_upload_reply: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
}

struct Loopback {
    url: url::Url,
    state: State,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Loopback {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn reply(status: u16, body: Vec<u8>) -> (u16, Vec<u8>) {
    (status, body)
}

fn route(
    state: &State,
    method: &str,
    target: &str,
    headers: &BTreeMap<String, String>,
    body: &[u8],
) -> (u16, Vec<u8>) {
    if headers.get("authorization").map(String::as_str) != Some("Bearer fixture-profile-token") {
        return reply(401, b"{}".to_vec());
    }
    let url = url::Url::parse(&format!("http://loopback{target}")).unwrap();
    let query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    state
        .requests
        .lock()
        .unwrap()
        .push(format!("{method} {}", url.path()));
    let mut files = state.files.lock().unwrap();
    match (method, url.path()) {
        ("GET", "/drive/v3/about") => {
            reply(200, br#"{"user":{"permissionId":"fixture-user"}}"#.to_vec())
        }
        ("GET", "/drive/v3/files/generateIds") => {
            let id = format!("credential-{}", state.next.fetch_add(1, Ordering::SeqCst));
            reply(
                200,
                json!({"ids":[id],"space":"appDataFolder"})
                    .to_string()
                    .into_bytes(),
            )
        }
        ("GET", "/drive/v3/files") => {
            let q = &query["q"];
            let wanted = |key: &str, value: &Value| {
                q.contains(&format!(
                    "key='{key}' and value='{}'",
                    value.as_str().unwrap_or_default()
                ))
            };
            let listed: Vec<&Value> = files
                .values()
                .map(|(metadata, _)| metadata)
                .filter(|m| {
                    let p = &m["appProperties"];
                    wanted("shepType", &p["shepType"])
                        && wanted("shepProfile", &p["shepProfile"])
                        && wanted("shepGeneration", &p["shepGeneration"])
                })
                .collect();
            reply(
                200,
                json!({"files":listed,"incompleteSearch":false})
                    .to_string()
                    .into_bytes(),
            )
        }
        ("POST", "/upload/drive/v3/files") => {
            let boundary = headers["content-type"]
                .split("boundary=")
                .nth(1)
                .unwrap()
                .to_owned();
            let text = body.to_vec();
            let marker = format!("--{boundary}");
            let parts: Vec<&[u8]> = split(&text, marker.as_bytes());
            let part = |i: usize| {
                let raw = parts[i];
                let start = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                raw[start..raw.len() - 2].to_vec()
            };
            let mut metadata: Value = serde_json::from_slice(&part(1)).unwrap();
            let media = part(2);
            let id = metadata["id"].as_str().unwrap().to_owned();
            if files.contains_key(&id) {
                return reply(409, b"{}".to_vec());
            }
            metadata["ownedByMe"] = json!(true);
            metadata["trashed"] = json!(false);
            metadata["spaces"] = json!(["appDataFolder"]);
            metadata["size"] = json!(media.len().to_string());
            metadata["sha256Checksum"] = json!(crate::profile_sync::digest(&media));
            metadata.as_object_mut().unwrap().remove("parents");
            files.insert(id, (metadata.clone(), media));
            if state.lose_upload_reply.swap(false, Ordering::SeqCst) {
                return reply(503, b"{}".to_vec());
            }
            reply(200, metadata.to_string().into_bytes())
        }
        ("DELETE", path) => match files.remove(path.trim_start_matches("/drive/v3/files/")) {
            Some(_) => reply(204, Vec::new()),
            None => reply(404, b"{}".to_vec()),
        },
        ("GET", path) => match files.get(path.trim_start_matches("/drive/v3/files/")) {
            Some((_, media)) if query.get("alt").map(String::as_str) == Some("media") => {
                reply(200, media.clone())
            }
            Some((metadata, _)) => reply(200, metadata.to_string().into_bytes()),
            None => reply(404, b"{}".to_vec()),
        },
        _ => reply(404, b"{}".to_vec()),
    }
}

fn split<'a>(bytes: &'a [u8], marker: &[u8]) -> Vec<&'a [u8]> {
    let mut parts = Vec::new();
    let mut rest = bytes;
    while let Some(at) = rest.windows(marker.len()).position(|w| w == marker) {
        parts.push(&rest[..at]);
        rest = &rest[at + marker.len()..];
    }
    parts.push(rest);
    parts
}

impl Loopback {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let state = State::default();
        let shared = state.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let state = shared.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let end = loop {
                        let mut chunk = [0; 8192];
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&chunk[..n]);
                        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                            break end + 4;
                        }
                    };
                    let head = String::from_utf8_lossy(&bytes[..end]).into_owned();
                    let mut lines = head.lines();
                    let mut first = lines.next().unwrap_or_default().split_whitespace();
                    let (method, target) = (
                        first.next().unwrap_or_default().to_owned(),
                        first.next().unwrap_or_default().to_owned(),
                    );
                    let headers: BTreeMap<String, String> = lines
                        .filter_map(|l| l.split_once(':'))
                        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
                        .collect();
                    let length: usize = headers
                        .get("content-length")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    while bytes.len() < end + length {
                        let mut chunk = [0; 8192];
                        let n = socket.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&chunk[..n]);
                    }
                    let (status, body) = route(
                        &state,
                        &method,
                        &target,
                        &headers,
                        &bytes[end..end + length],
                    );
                    let mut response = format!(
                        "HTTP/1.1 {status} Test\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    response.extend(body);
                    let _ = socket.write_all(&response).await;
                });
            }
        });
        Self { url, state, task }
    }
    fn session(&self) -> Session {
        Session {
            http: crate::providers::test_http::client(),
            base: self.url.clone(),
            token: SecretString::from("fixture-profile-token"),
            binding: super::super::tests::binding(),
        }
    }
}

fn scope() -> Scope {
    let binding = passwords::tests::binding();
    Scope {
        profile: binding.profile,
        generation: binding.generation,
    }
}

#[tokio::test]
async fn profile_vault_drive_creates_lists_downloads_and_deletes_owned_files() {
    let drive = Loopback::start().await;
    let session = drive.session();
    let bytes = br#"{"format":"so.shep.credential-vault"}"#.to_vec();
    let created = session
        .create(
            scope(),
            NewFile {
                kind: Kind::Vault { revision: 3 },
                key: Uuid::from_u128(7),
                bytes: bytes.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(created.kind, Kind::Vault { revision: 3 });
    let listed = session.list(scope()).await.unwrap();
    assert_eq!(listed, vec![created.clone()]);
    assert_eq!(
        Remote::download(&session, scope(), &created).await.unwrap(),
        bytes
    );
    // Another profile's files are not listed.
    let other = Scope {
        profile: Uuid::from_u128(0x99),
        ..scope()
    };
    assert!(session.list(other).await.unwrap().is_empty());
    session.delete(&created).await.unwrap();
    // Deleting an already removed file is not an error.
    session.delete(&created).await.unwrap();
    assert!(session.list(scope()).await.unwrap().is_empty());
    // A lost upload reply is confirmed from the reserved ID, not uploaded twice.
    drive.state.lose_upload_reply.store(true, Ordering::SeqCst);
    let key = session
        .create(
            scope(),
            NewFile {
                kind: Kind::Key { sequence: 2 },
                key: Uuid::from_u128(8),
                bytes: b"key".to_vec(),
            },
        )
        .await
        .unwrap();
    assert_eq!(key.kind, Kind::Key { sequence: 2 });
    assert_eq!(drive.state.files.lock().unwrap().len(), 1);
    let requests = drive.state.requests.lock().unwrap().clone();
    assert_eq!(requests.iter().filter(|r| r.starts_with("POST")).count(), 2);
}

#[tokio::test]
async fn profile_vault_drive_rejects_foreign_changed_or_newer_files() {
    let drive = Loopback::start().await;
    let session = drive.session();
    let created = session
        .create(
            scope(),
            NewFile {
                kind: Kind::Vault { revision: 1 },
                key: Uuid::from_u128(7),
                bytes: b"vault".to_vec(),
            },
        )
        .await
        .unwrap();
    // Media that no longer matches its checksum is never used.
    drive
        .state
        .files
        .lock()
        .unwrap()
        .get_mut(&created.id)
        .unwrap()
        .1 = b"VAULT".to_vec();
    assert!(Remote::download(&session, scope(), &created).await.is_err());
    for (property, value, expected) in [
        ("shepNamespace", json!("0".repeat(64)), "not owned"),
        ("shepFormat", json!("credential-vault-v2"), "newer format"),
        (
            "shepProfile",
            json!(Uuid::from_u128(0x77).to_string()),
            "another profile",
        ),
    ] {
        let mut file = drive.state.files.lock().unwrap()[&created.id].0.clone();
        file["appProperties"][property] = value;
        let error = session.parse_credential(&file, scope()).unwrap_err();
        assert!(
            format!("{error:#}").contains(expected),
            "{property}: {error:#}"
        );
    }
    let mut unowned = drive.state.files.lock().unwrap()[&created.id].0.clone();
    unowned["ownedByMe"] = json!(false);
    assert!(session.parse_credential(&unowned, scope()).is_err());
}

#[tokio::test]
async fn profile_vault_loopback_publish_and_import_keep_passwords_out_of_sqlite_logs_and_drive() {
    let buffer = Arc::new(Mutex::new(Vec::<u8>::new()));
    let writer = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || Capture(writer.clone()))
        .finish();
    let _logs = tracing::subscriber::set_default(subscriber);
    let dir = tempfile::tempdir().unwrap();
    let drive = Loopback::start().await;
    let session = drive.session();
    let studio = Uuid::from_u128(0x5001);
    let (first, second) = (dir.path().join("a"), dir.path().join("b"));
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let a = device(
        Some(&first),
        &[(
            account(&studio.to_string(), "imap.studio.test", true),
            studio,
        )],
        false,
        &[
            (&studio.to_string(), INCOMING),
            (&format!("{studio}:smtp"), SMTP),
        ],
    )
    .await;
    let b = device(
        Some(&second),
        &[(account("b-native", "imap.studio.test", true), studio)],
        true,
        &[],
    )
    .await;
    let published = passwords::tests::pass(
        &a,
        &session,
        &Servers::default(),
        &History::default(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(published.published, 2);
    let servers = Servers {
        accept: vec![INCOMING.into(), SMTP.into()],
        ..Default::default()
    };
    let imported = passwords::tests::pass(&b, &session, &servers, &History::default(), false)
        .await
        .unwrap();
    assert_eq!(imported.imported, 1);
    assert_eq!(b.secret("b-native").as_deref(), Some(INCOMING));
    assert_eq!(b.local().await.synced(studio, Field::Smtp).revision, 1);
    let report = format!("{published:?}{imported:?}");
    // Every byte on disk (main files and their live WAL), in the log and on
    // Drive is free of both passwords. The stores stay open so no sidecar is
    // checkpointed away between listing and reading.
    let mut stored = Vec::new();
    for entry in walk(dir.path()) {
        stored.push((entry.display().to_string(), std::fs::read(&entry).unwrap()));
    }
    drop((a, b));
    assert!(
        stored
            .iter()
            .any(|(name, _)| name.ends_with("cache.sqlite"))
    );
    for (name, bytes) in drive
        .state
        .files
        .lock()
        .unwrap()
        .values()
        .map(|(m, b)| (m["name"].to_string(), b.clone()))
    {
        stored.push((name, bytes));
    }
    // Tracing's shared callsite cache can drop some events while other tests
    // run in parallel, so the native scenarios also scan debug app logs.
    stored.push(("log".into(), buffer.lock().unwrap().clone()));
    stored.push(("reports".into(), report.into_bytes()));
    for secret in [INCOMING, SMTP] {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(secret);
        for (name, bytes) in &stored {
            for needle in [secret.as_bytes(), encoded.as_bytes()] {
                assert!(
                    !bytes.windows(needle.len()).any(|w| w == needle),
                    "{name} contains a synced password"
                );
            }
        }
    }
}

fn walk(path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

struct Capture(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
