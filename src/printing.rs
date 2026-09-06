//! Off-thread MIME preparation and a short-lived, single-use loopback preview.
//! The browser owns printer/PDF selection; a launched dialog is not a receipt.
mod document;
#[cfg(test)]
mod tests;

use anyhow::Context;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub plain: bool,
    /// Only images already loaded under the selected message's current policy.
    pub images: HashMap<String, Arc<[u8]>>,
}

#[derive(Clone)]
pub struct Service(Arc<Semaphore>);
impl Default for Service {
    fn default() -> Self {
        Self(Arc::new(Semaphore::new(2)))
    }
}

#[derive(Debug)]
pub struct Preview {
    pub url: String,
    task: tokio::task::AbortHandle,
}
impl Drop for Preview {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Service {
    pub async fn prepare(
        &self,
        store: &crate::store::Store,
        id: &str,
        options: Options,
    ) -> anyhow::Result<Arc<Preview>> {
        let permit = self.0.clone().try_acquire_owned().context(
            "Two print previews are already preparing. Close a preview and try Print again.",
        )?;
        let raw = store.raw_message(id.to_owned()).await?;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let script_nonce = nonce.clone();
        let document =
            tokio::task::spawn_blocking(move || document::prepare(&raw, options, &script_nonce))
                .await??;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .context("Could not start the print preview. Try Print again.")?;
        let host = listener.local_addr()?.to_string();
        let path = format!("/print/{}", uuid::Uuid::new_v4().simple());
        let url = format!("http://{host}{path}");
        let task = tokio::spawn(async move {
            let _permit = permit;
            let _ = tokio::time::timeout(
                Duration::from_secs(300),
                serve(listener, host, path, document, nonce),
            )
            .await;
        });
        Ok(Arc::new(Preview {
            url,
            task: task.abort_handle(),
        }))
    }
}

async fn serve(listener: TcpListener, host: String, path: String, document: String, nonce: String) {
    let document = Arc::new(document);
    let policy = format!(
        "default-src 'none'; script-src 'nonce-{nonce}'; style-src 'unsafe-inline'; img-src data:; frame-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
    );
    let claimed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut clients = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            Some(result) = clients.join_next(), if !clients.is_empty() => {
                if matches!(result, Ok(Ok(true))) { break; }
            }
            accepted = listener.accept(), if clients.len() < 4 => {
                let Ok((socket, _)) = accepted else { break; };
                let (host, path, policy, document, claimed) = (host.clone(), path.clone(), policy.clone(), document.clone(), claimed.clone());
                clients.spawn(async move {
                    tokio::time::timeout(Duration::from_secs(5), response(socket, &host, &path, &policy, &document, &claimed))
                        .await.unwrap_or(Ok(false))
                });
            }
        }
    }
}

async fn response(
    mut socket: TcpStream,
    host: &str,
    path: &str,
    policy: &str,
    document: &str,
    claimed: &std::sync::atomic::AtomicBool,
) -> anyhow::Result<bool> {
    let mut request = Vec::new();
    let mut chunk = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
        let n = socket.read(&mut chunk).await?;
        if n == 0 {
            return Ok(false);
        }
        request.extend_from_slice(&chunk[..n]);
    }
    let request = String::from_utf8_lossy(&request);
    let mut lines = request.split("\r\n");
    let line = lines.next().unwrap_or_default();
    let hosts: Vec<_> = lines
        .filter_map(|l| l.split_once(':'))
        .filter(|(key, _)| key.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.trim())
        .collect();
    let valid = request.ends_with("\r\n\r\n") && hosts == [host];
    let get = line == format!("GET {path} HTTP/1.1");
    let head = line == format!("HEAD {path} HTTP/1.1");
    let accepted = valid && (get || head);
    let claimed_now = accepted
        && get
        && claimed
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .is_ok();
    let (status, body) = if accepted && (head || claimed_now) {
        ("200 OK", document)
    } else {
        (
            "404 Not Found",
            "This preview is unavailable. Return to Shep and choose Print again.",
        )
    };
    socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: {policy}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await?;
    if !head {
        socket.write_all(body.as_bytes()).await?;
    }
    socket.shutdown().await?;
    Ok(claimed_now)
}

pub fn open(preview: &Preview, demo: bool) -> anyhow::Result<()> {
    if demo {
        #[cfg(feature = "test-support")]
        {
            let launcher = std::env::var_os("SHEP_TEST_PRINT_BROWSER")
                .context("No isolated print browser is configured for this preview.")?;
            let status = std::process::Command::new(launcher)
                .arg(&preview.url)
                .status()?;
            anyhow::ensure!(
                status.success(),
                "The isolated print browser could not be opened."
            );
            return Ok(());
        }
        #[cfg(not(feature = "test-support"))]
        anyhow::bail!("Printing is unavailable in this preview.");
    }
    webbrowser::open(&preview.url).context(
        "Could not open your default browser. Check its configuration, then choose Print again.",
    )
}
