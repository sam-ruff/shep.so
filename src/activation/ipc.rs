use super::{Record, Signal};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions,
    tokio::{Stream, prelude::*},
};
use std::{io, thread, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::oneshot,
    task::JoinSet,
    time::timeout,
};

const MAGIC: &[u8; 8] = b"SHEPOP01";
/// An owner built before this request reads only the first magic and drops it.
const RESTART_MAGIC: &[u8; 8] = b"SHEPRS01";
/// An Open followed by a length-prefixed `mailto` link.
const COMPOSE_MAGIC: &[u8; 8] = b"SHEPMT01";
const HANDSHAKE: Duration = Duration::from_millis(250);
const ACKNOWLEDGMENT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reply {
    Accepted,
    Closing,
    /// The owner is quitting so the launching build can take over.
    Restarting,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub(super) trait Transport: Send + Sync {
    async fn request(&self, record: Record) -> anyhow::Result<Reply>;
    async fn restart(&self, record: Record) -> anyhow::Result<Reply>;
}

pub(super) struct Local;

#[async_trait::async_trait]
impl Transport for Local {
    async fn request(&self, record: Record) -> anyhow::Result<Reply> {
        exchange(record, MAGIC, None).await
    }

    async fn restart(&self, record: Record) -> anyhow::Result<Reply> {
        exchange(record, RESTART_MAGIC, None).await
    }
}

/// Opens the owner with a draft for the held `mailto` link.
pub(super) struct Compose(pub(super) String);

#[async_trait::async_trait]
impl Transport for Compose {
    async fn request(&self, record: Record) -> anyhow::Result<Reply> {
        exchange(record, COMPOSE_MAGIC, Some(self.0.as_bytes())).await
    }

    async fn restart(&self, record: Record) -> anyhow::Result<Reply> {
        Local.restart(record).await
    }
}

async fn exchange(
    record: Record,
    magic: &'static [u8; 8],
    payload: Option<&[u8]>,
) -> anyhow::Result<Reply> {
    timeout(ACKNOWLEDGMENT + HANDSHAKE, async move {
        let name = record.endpoint.as_str().to_ns_name::<GenericNamespaced>()?;
        let connection = Stream::connect(name).await?;
        let mut connection = &connection;
        connection.write_all(magic).await?;
        connection.write_all(record.secret.as_bytes()).await?;
        if let Some(payload) = payload {
            let length = u16::try_from(payload.len())?;
            anyhow::ensure!(
                usize::from(length) <= crate::mailto::MAX_LEN,
                "The mailto link is too long"
            );
            connection.write_u16(length).await?;
            connection.write_all(payload).await?;
        }
        match connection.read_u8().await? {
            1 => Ok(Reply::Accepted),
            2 => Ok(Reply::Closing),
            3 => Ok(Reply::Restarting),
            _ => anyhow::bail!("Invalid activation acknowledgment"),
        }
    })
    .await?
}

pub(super) struct Server {
    stop: Option<oneshot::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    pub(super) fn start(record: Record, signal: Signal) -> anyhow::Result<Self> {
        let (stop, stopped) = oneshot::channel();
        let (ready, receiving) = std::sync::mpsc::sync_channel(1);
        let thread = thread::Builder::new().name("shep-activation".into()).spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build();
            let result = runtime.and_then(|runtime| runtime.block_on(async {
                let name = record.endpoint.as_str().to_ns_name::<GenericNamespaced>()?;
                let listener = ListenerOptions::new().name(name).create_tokio()?;
                let _ = ready.send(Ok(()));
                let mut stopped = stopped;
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        _ = &mut stopped => break,
                        Some(_) = connections.join_next(), if !connections.is_empty() => {},
                        accepted = listener.accept() => {
                            let connection = accepted?;
                            if connections.len() < 8 {
                                connections.spawn(serve(connection, record.clone(), signal.clone()));
                            }
                        }
                    }
                }
                connections.abort_all();
                while connections.join_next().await.is_some() {}
                Ok::<_, io::Error>(())
            }));
            if let Err(error) = result {
                let _ = ready.send(Err(error));
            }
        })?;
        let mut server = Self {
            stop: Some(stop),
            thread: Some(thread),
        };
        if let Err(error) = receiving
            .recv()
            .map_err(anyhow::Error::from)
            .and_then(|result| result.map_err(Into::into))
        {
            server.stop();
            return Err(error);
        }
        Ok(server)
    }

    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

async fn serve(connection: Stream, record: Record, signal: Signal) {
    let mut connection = &connection;
    let mut request = [0_u8; 24];
    if !matches!(
        timeout(HANDSHAKE, connection.read_exact(&mut request)).await,
        Ok(Ok(_))
    ) || &request[8..] != record.secret.as_bytes()
    {
        return;
    }
    let (admitted, accepted) = match &request[..8] {
        magic if magic == MAGIC => (signal.request(), 1),
        magic if magic == RESTART_MAGIC => (signal.request_restart(), 3),
        magic if magic == COMPOSE_MAGIC => {
            let Ok(Some(link)) = timeout(HANDSHAKE, read_link(connection)).await else {
                return;
            };
            (signal.request_compose(link), 1)
        }
        _ => return,
    };
    let reply = match admitted {
        None => 2,
        Some(generation) => {
            if !matches!(
                timeout(ACKNOWLEDGMENT, signal.acknowledged(generation)).await,
                Ok(true)
            ) {
                return;
            }
            accepted
        }
    };
    let _ = timeout(HANDSHAKE, connection.write_u8(reply)).await;
}

async fn read_link(mut connection: &Stream) -> Option<String> {
    let length = usize::from(connection.read_u16().await.ok()?);
    if length > crate::mailto::MAX_LEN {
        return None;
    }
    let mut link = vec![0_u8; length];
    connection.read_exact(&mut link).await.ok()?;
    let link = String::from_utf8(link).ok()?;
    crate::mailto::Mailto::parse(&link).map(|_| link)
}
