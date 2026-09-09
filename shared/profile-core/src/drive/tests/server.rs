//! Object-scoped scripted HTTP fixture. No production endpoint override or tokens.
use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone)]
pub struct Request {
    pub method: String,
    pub url: Url,
    pub headers: std::collections::HashMap<String, String>,
    pub body: Vec<u8>,
}
pub struct Response {
    status: u16,
    bytes: Vec<u8>,
    headers: Vec<(String, String)>,
    gate: Option<tokio::sync::oneshot::Receiver<()>>,
}
impl Response {
    pub fn json(value: Value) -> Self {
        Self::new(200, value.to_string().into_bytes())
    }
    pub fn new(status: u16, bytes: Vec<u8>) -> Self {
        Self {
            status,
            bytes,
            headers: vec![],
            gate: None,
        }
    }
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }
    pub fn held(mut self, gate: tokio::sync::oneshot::Receiver<()>) -> Self {
        self.gate = Some(gate);
        self
    }
}
pub type Step = Box<dyn FnOnce(&Request) -> Response + Send>;
pub fn reply(response: Response) -> Step {
    Box::new(move |_| response)
}
pub fn value(value: Value) -> Step {
    reply(Response::json(value))
}
pub fn absent() -> Step {
    reply(Response::new(404, vec![]))
}
pub struct Server {
    pub base: Url,
    requests: Arc<Mutex<Vec<Request>>>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Server {
    pub async fn start(steps: Vec<Step>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let url = base.clone();
        let requests = Arc::new(Mutex::new(vec![]));
        let recorded = requests.clone();
        let task = tokio::spawn(async move {
            for step in steps {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = vec![];
                let end = loop {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(n, 0, "request header cut short");
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                    assert!(bytes.len() < 32768);
                };
                let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap().split_whitespace();
                let method = first.next().unwrap().to_string();
                let target = first.next().unwrap();
                let headers: std::collections::HashMap<_, _> = lines
                    .filter_map(|line| line.split_once(':'))
                    .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
                    .collect();
                let size = headers
                    .get("content-length")
                    .map_or(0, |n| n.parse::<usize>().unwrap());
                assert!(size < 2 * crate::MAX_RECORD_BYTES);
                while bytes.len() < end + size {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(n, 0, "request body cut short");
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let request = Request {
                    method,
                    url: url.join(target).unwrap(),
                    headers,
                    body: bytes[end..end + size].to_vec(),
                };
                let response = step(&request);
                recorded.lock().unwrap().push(request);
                if let Some(gate) = response.gate {
                    let _ = gate.await;
                }
                if response.status == 0 {
                    continue;
                }
                let mut head = format!(
                    "HTTP/1.1 {} Fixture\r\nConnection: close\r\n",
                    response.status
                );
                if !response.headers.iter().any(|(k, _)| {
                    matches!(
                        k.to_ascii_lowercase().as_str(),
                        "content-length" | "transfer-encoding"
                    )
                }) {
                    head.push_str(&format!("Content-Length: {}\r\n", response.bytes.len()));
                }
                for (k, v) in response.headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("\r\n");
                if socket.write_all(head.as_bytes()).await.is_ok() {
                    let _ = socket.write_all(&response.bytes).await;
                }
            }
        });
        Self {
            base,
            requests,
            task: Some(task),
        }
    }
    pub async fn connect(&self, expected: Option<&str>) -> Result<Drive> {
        Drive::verify(
            Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap(),
            self.base.clone(),
            SecretString::from("synthetic-access-token"),
            NAMESPACE.into(),
            expected,
        )
        .await
    }
    pub async fn finish(mut self) -> Vec<Request> {
        tokio::time::timeout(Duration::from_secs(10), self.task.take().unwrap())
            .await
            .expect("unconsumed scripted request")
            .expect("HTTP fixture panicked");
        self.requests.lock().unwrap().clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
