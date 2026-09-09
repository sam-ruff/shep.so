//! Synthetic loopback Google responses. Nondefault test-support only; never
//! takes an endpoint, token, account or database from the personal workspace.
use anyhow::Result;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use shep_profile_core::{Action, Change, Operation, SettingKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

pub const NAMESPACE: &str = "so.shep.fixture";
pub struct Fixture {
    pub root: tempfile::TempDir,
    pub url: url::Url,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    pub async fn start(
        count: usize,
        mut fail_once: bool,
        delay: std::time::Duration,
    ) -> Result<Self> {
        anyhow::ensure!(count <= 60, "fixture bound");
        let root = tempfile::tempdir()?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = url::Url::parse(&format!("http://{}/", listener.local_addr()?))?;
        let mut records = Vec::new();
        for n in 0..count {
            for phase in 0..3 {
                let id = 10_000 + n as u128 * 3 + phase;
                let operation = Operation {
                    format: shep_profile_core::FORMAT.into(),
                    major: 1,
                    minor: 0,
                    requires: vec![
                        "causal-v1".into(),
                        "settings-v1".into(),
                        "initialization-v1".into(),
                    ],
                    namespace: NAMESPACE.into(),
                    profile: Uuid::from_u128(100 + n as u128),
                    generation: Uuid::from_u128(200 + n as u128),
                    device: Uuid::from_u128(300),
                    operation: Uuid::from_u128(id),
                    parents: if phase == 0 {
                        vec![]
                    } else {
                        vec![Uuid::from_u128(id - 1)]
                    },
                    changes: (if phase == 0 {
                        vec![Action::ProfileSetup { complete: false }]
                    } else if phase == 1 {
                        vec![
                            Action::ProfileName {
                                name: if n == 0 {
                                    "Work".into()
                                } else {
                                    format!("Personal {n:02}")
                                },
                            },
                            Action::Setting {
                                key: SettingKey::Appearance,
                                value: json!("Dark"),
                            },
                        ]
                    } else {
                        vec![Action::ProfileSetup { complete: true }]
                    })
                    .into_iter()
                    .map(|action| Change {
                        action,
                        extra: Default::default(),
                    })
                    .collect(),
                    extra: Default::default(),
                };
                let bytes = operation.encode()?;
                let meta = json!({
                    "id": format!("fixture-{id}"), "name": format!("shep-profile-{}.json", operation.operation),
                    "trashed": false, "ownedByMe": true, "spaces": ["appDataFolder"],
                    "mimeType": "application/json", "size": bytes.len().to_string(),
                    "appProperties": {
                        "shepType": "profile", "shepFormat": "operation-v1",
                        "shepNamespace": format!("{:x}", Sha256::digest(NAMESPACE)),
                        "shepProfile": operation.profile, "shepGeneration": operation.generation,
                        "shepOperation": operation.operation, "shepSha256": format!("{:x}", Sha256::digest(&bytes)),
                    }
                });
                records.push((meta, bytes));
            }
        }
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let Ok(n) = socket.read(&mut chunk).await else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    if bytes.windows(4).any(|w| w == b"\r\n\r\n") || bytes.len() > 32768 {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&bytes);
                let mut line = request.lines().next().unwrap_or("").split_whitespace();
                let method = line.next().unwrap_or("");
                let target = line.next().unwrap_or("");
                let Ok(uri) = url::Url::parse(&format!("http://127.0.0.1{target}")) else {
                    continue;
                };
                let mut status = 200;
                let body: Vec<u8> = if method != "GET" {
                    status = 405;
                    b"fixture is read only".to_vec()
                } else if uri.path() == "/drive/v3/about" {
                    serde_json::to_vec(&json!({"user":{"permissionId":"fixture"}})).unwrap()
                } else if uri.path() == "/drive/v3/changes/startPageToken" {
                    serde_json::to_vec(&json!({"startPageToken":"fixture-start"})).unwrap()
                } else if uri.path() == "/drive/v3/changes" {
                    serde_json::to_vec(
                        &json!({"changes":[],"newStartPageToken":"fixture-caught-up"}),
                    )
                    .unwrap()
                } else if uri.path() == "/drive/v3/files" {
                    tokio::time::sleep(delay).await;
                    if fail_once {
                        fail_once = false;
                        status = 503;
                        b"synthetic provider failure".to_vec()
                    } else {
                        let offset: usize = uri
                            .query_pairs()
                            .find(|(k, _)| k == "pageToken")
                            .and_then(|(_, v)| v.parse().ok())
                            .unwrap_or(0);
                        let mut body = json!({"incompleteSearch":false,"files":records.iter().skip(offset).take(50).map(|(m,_)|m).collect::<Vec<_>>()});
                        if offset + 50 < records.len() {
                            body["nextPageToken"] = json!((offset + 50).to_string());
                        }
                        serde_json::to_vec(&body).unwrap()
                    }
                } else if let Some((meta, content)) = records.iter().find(|(meta, _)| {
                    uri.path().strip_prefix("/drive/v3/files/") == meta["id"].as_str()
                }) {
                    if uri.query_pairs().any(|(k, v)| k == "alt" && v == "media") {
                        content.clone()
                    } else {
                        serde_json::to_vec(meta).unwrap()
                    }
                } else {
                    status = 404;
                    serde_json::to_vec(&Value::Null).unwrap()
                };
                let head = format!(
                    "HTTP/1.1 {status} Fixture\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len()
                );
                if socket.write_all(head.as_bytes()).await.is_ok() {
                    let _ = socket.write_all(&body).await;
                }
            }
        });
        Ok(Self { root, url, task })
    }
    pub async fn connect(
        &self,
        namespace: String,
        principal: &str,
    ) -> Result<shep_profile_core::drive::Drive> {
        Ok(
            shep_profile_core::drive::Drive::connect_fixture(
                self.url.clone(),
                namespace,
                principal,
            )
            .await?,
        )
    }
}
