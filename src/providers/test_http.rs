//! Scripted loopback HTTP server for provider contract tests only.
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Debug)]
pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: std::collections::HashMap<String, String>,
    pub body: String,
    pub bytes: Vec<u8>,
}

pub struct Reply {
    status: u16,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}
impl Reply {
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            body: body.into().into_bytes(),
            headers: Vec::new(),
        }
    }
    pub fn binary(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            body,
            headers: Vec::new(),
        }
    }
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    pub fn disconnect() -> Self {
        Self::new(0, "")
    }
    pub fn chunked(status: u16, body: Vec<u8>) -> Self {
        let mut bytes = format!("{:X}\r\n", body.len()).into_bytes();
        bytes.extend(body);
        bytes.extend_from_slice(b"\r\n0\r\n\r\n");
        Self::binary(status, bytes).header("Transfer-Encoding", "chunked")
    }
}

pub struct Server {
    pub url: url::Url,
    requests: Arc<Mutex<Vec<Request>>>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Server {
    pub async fn start(replies: Vec<Reply>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = url::Url::parse(&format!(
            "http://{}/calendars/",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0, "Client closed before sending request headers");
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end + 4;
                    }
                    assert!(bytes.len() < 65536);
                };
                let head = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap().split_whitespace();
                let method = first.next().unwrap().into();
                let target = first.next().unwrap().into();
                let headers: std::collections::HashMap<_, _> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
                    .collect();
                let length = headers
                    .get("content-length")
                    .map(|s| s.parse::<usize>().unwrap())
                    .unwrap_or(0);
                assert!(length < 16 * 1024 * 1024);
                while bytes.len() < header_end + length {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert!(n > 0, "Client closed before sending request body");
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let raw = bytes[header_end..header_end + length].to_vec();
                let body = String::from_utf8_lossy(&raw).into_owned();
                recorded.lock().unwrap().push(Request {
                    method,
                    target,
                    headers,
                    body,
                    bytes: raw,
                });
                if reply.status == 0 {
                    continue;
                }
                let mut response =
                    format!("HTTP/1.1 {} Test\r\nConnection: close\r\n", reply.status);
                if !reply.headers.iter().any(|(k, _)| {
                    k.eq_ignore_ascii_case("content-length")
                        || k.eq_ignore_ascii_case("transfer-encoding")
                }) {
                    response.push_str(&format!("Content-Length: {}\r\n", reply.body.len()));
                }
                for (key, value) in reply.headers {
                    response.push_str(&format!("{key}: {value}\r\n"));
                }
                response.push_str("\r\n");
                let mut response = response.into_bytes();
                response.extend(reply.body);
                let _ = socket.write_all(&response).await;
            }
        });
        Self {
            url,
            requests,
            task: Some(task),
        }
    }
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
    pub async fn finish(&mut self) {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.task.as_mut().unwrap(),
        )
        .await
        .expect("Expected HTTP request was not made")
        .expect("HTTP server failed");
        self.task = None;
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap()
}
