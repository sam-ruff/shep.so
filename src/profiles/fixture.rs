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
    #[cfg(test)]
    pub attempts: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
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
                    requires: vec!["accounts-v1".into(),
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
                        let account:crate::model::Account=serde_json::from_value(json!({"id":format!("fixture-account-{n}"),"name":format!("Shared account {n}"),"email":format!("shared-{n}@example.test"),"protocol":"Imap","host":"imap.example.test","port":993,"username":format!("shared-{n}"),"smtp_host":"smtp.example.test","smtp_port":465}))?;
                        let mut actions:Vec<_>=shep_mail_core::profiles::export_account(&account,Uuid::from_u128(10000+n as u128))?.into_iter().map(|c|c.action).collect();
                        actions.extend([
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
                        ]);
                        actions
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
        let attempts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let written = attempts.clone();
        let mut upload_failure = fail_once;
        let mut hidden_once: Option<String> = None;
        let mut reserved = std::collections::HashSet::new();
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                let mut body_start = None;
                let mut content_length = 0;
                loop {
                    let mut chunk = [0; 4096];
                    let Ok(n) = socket.read(&mut chunk).await else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    if body_start.is_none()
                        && let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n")
                    {
                        body_start = Some(end + 4);
                        content_length = String::from_utf8_lossy(&bytes[..end])
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().ok())
                                    .flatten()
                            })
                            .unwrap_or(0);
                    }
                    if bytes.len() > shep_profile_core::MAX_RECORD_BYTES + 32768
                        || content_length > shep_profile_core::MAX_RECORD_BYTES + 16384
                    {
                        break;
                    }
                    if body_start.is_some_and(|start| bytes.len() >= start + content_length) {
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
                let body: Vec<u8> = if method == "POST" && uri.path() == "/upload/drive/v3/files" {
                    tokio::time::sleep(delay).await;
                    match body_start
                        .and_then(|start| bytes.get(start..start + content_length))
                        .and_then(|body| upload_record(body).ok())
                    {
                        Some((meta, record))
                            if meta["id"].as_str().is_some_and(|id| reserved.contains(id)) =>
                        {
                            let id = meta["id"].as_str().unwrap().to_owned();
                            written.lock().unwrap().push(id.clone());
                            if records.iter().any(|(m, _)| m["id"] == id) {
                                status = 409;
                            } else {
                                records.push((meta, record));
                            }
                            if upload_failure {
                                upload_failure = false;
                                hidden_once = Some(id.clone());
                                status = 503;
                            }
                            serde_json::to_vec(&json!({"id":id})).unwrap()
                        }
                        _ => {
                            status = 400;
                            b"invalid synthetic upload".to_vec()
                        }
                    }
                } else if method != "GET" {
                    status = 405;
                    vec![]
                } else if uri.path() == "/drive/v3/files/generateIds" {
                    let id = format!("reserved-{}", reserved.len() + 1);
                    reserved.insert(id.clone());
                    serde_json::to_vec(&json!({"space":"appDataFolder","ids":[id]})).unwrap()
                } else if hidden_once
                    .as_ref()
                    .is_some_and(|id| uri.path() == format!("/drive/v3/files/{id}"))
                {
                    hidden_once = None;
                    status = 503;
                    b"synthetic lost upload receipt".to_vec()
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
        Ok(Self {
            root,
            url,
            task,
            #[cfg(test)]
            attempts,
        })
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

fn upload_record(body: &[u8]) -> Result<(Value, Vec<u8>)> {
    let text = std::str::from_utf8(body)?;
    let (boundary, _) = text
        .split_once("\r\n")
        .ok_or_else(|| anyhow::anyhow!("multipart boundary"))?;
    anyhow::ensure!(
        boundary.starts_with("--shep_") && boundary.len() < 100,
        "multipart boundary"
    );
    let parts: Vec<_> = text.split(boundary).collect();
    anyhow::ensure!(
        parts.len() == 4 && parts[0].is_empty() && parts[3] == "--\r\n",
        "multipart parts"
    );
    let json_part = |part: &str| -> Result<String> {
        let (_, value) = part
            .split_once("\r\n\r\n")
            .ok_or_else(|| anyhow::anyhow!("multipart headers"))?;
        Ok(value
            .strip_suffix("\r\n")
            .ok_or_else(|| anyhow::anyhow!("multipart ending"))?
            .to_owned())
    };
    let mut meta: Value = serde_json::from_str(&json_part(parts[1])?)?;
    let record = json_part(parts[2])?.into_bytes();
    let operation = Operation::decode(&record)?;
    anyhow::ensure!(
        operation.namespace == NAMESPACE
            && meta["parents"] == json!(["appDataFolder"])
            && meta["appProperties"]["shepSha256"] == format!("{:x}", Sha256::digest(&record))
            && meta["appProperties"]["shepOperation"] == operation.operation.to_string(),
        "uploaded metadata"
    );
    meta["ownedByMe"] = json!(true);
    meta["trashed"] = json!(false);
    meta["spaces"] = json!(["appDataFolder"]);
    meta["size"] = json!(record.len().to_string());
    Ok((meta, record))
}
