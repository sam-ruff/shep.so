//! Bounds and delays encrypted server responses without replacing the server.
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Notify,
    task::{JoinHandle, JoinSet},
};

pub(super) struct ImapRelay {
    port: u16,
    control: Arc<Control>,
    task: JoinHandle<()>,
}

#[derive(Default)]
struct Control {
    held: AtomicBool,
    changed: Notify,
    delay_ms: AtomicU64,
    forwarded: AtomicU64,
    connections: AtomicUsize,
    held_responses: AtomicUsize,
}

impl ImapRelay {
    pub(super) async fn start(upstream_port: u16) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let control = Arc::new(Control::default());
        let accept_control = control.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    incoming = listener.accept() => {
                        let Ok((client, _)) = incoming else { return };
                        let control = accept_control.clone();
                        connections.spawn(async move {
                            let _ = forward(client, upstream_port, control).await;
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Ok(Self {
            port,
            control,
            task,
        })
    }

    pub(super) fn port(&self) -> u16 {
        self.port
    }

    pub(super) fn hold(&self) {
        self.control.held.store(true, Ordering::SeqCst);
    }

    pub(super) fn release(&self) {
        self.control.held.store(false, Ordering::SeqCst);
        self.control.changed.notify_waiters();
    }

    pub(super) fn set_delay(&self, delay: Duration) {
        self.control.delay_ms.store(
            delay.as_millis().try_into().unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
        self.control.changed.notify_waiters();
    }

    pub(super) fn forwarded_bytes(&self) -> u64 {
        self.control.forwarded.load(Ordering::SeqCst)
    }

    pub(super) fn connections(&self) -> usize {
        self.control.connections.load(Ordering::SeqCst)
    }

    pub(super) fn held_responses(&self) -> usize {
        self.control.held_responses.load(Ordering::SeqCst)
    }
}

impl Drop for ImapRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn forward(
    client: TcpStream,
    upstream_port: u16,
    control: Arc<Control>,
) -> std::io::Result<()> {
    let upstream = TcpStream::connect(("127.0.0.1", upstream_port)).await?;
    control.connections.fetch_add(1, Ordering::SeqCst);
    let (mut client_input, mut client_output) = client.into_split();
    let (mut server_input, mut server_output) = upstream.into_split();
    let requests = async {
        tokio::io::copy(&mut client_input, &mut server_output).await?;
        server_output.shutdown().await
    };
    let responses = async {
        let mut buffer = [0; 16 * 1024];
        loop {
            let size = server_input.read(&mut buffer).await?;
            if size == 0 {
                return client_output.shutdown().await;
            }
            loop {
                let changed = control.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if !control.held.load(Ordering::SeqCst) {
                    break;
                }
                control.held_responses.fetch_add(1, Ordering::SeqCst);
                let _held = HeldResponse(&control);
                changed.await;
            }
            let delay = Duration::from_millis(control.delay_ms.load(Ordering::SeqCst));
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            client_output.write_all(&buffer[..size]).await?;
            control.forwarded.fetch_add(size as u64, Ordering::SeqCst);
        }
    };
    tokio::try_join!(requests, responses)?;
    Ok(())
}

struct HeldResponse<'a>(&'a Control);

impl Drop for HeldResponse<'_> {
    fn drop(&mut self) {
        self.0.held_responses.fetch_sub(1, Ordering::SeqCst);
    }
}
