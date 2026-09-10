//! An object-scoped real SSH/SFTP peer; the file map has one bounded owner.
use super::*;
use russh::{
    Channel, ChannelId,
    server::{ChannelOpenHandle, Msg, Session},
};
use russh_sftp::protocol::{Attrs, Data, File, Handle, Name, Status};
use std::collections::{BTreeMap, HashMap};

#[derive(Default)]
pub(crate) struct Files {
    pub data: BTreeMap<String, Vec<u8>>,
    pub writes: Vec<(String, u64, usize)>,
    pub deletes: Vec<String>,
    pub renames: usize,
    pub lose_rename: bool,
    pub lose_next_stat: bool,
    pub fail_write: Option<usize>,
    pub short_reads: bool,
    pub repeat_listing: bool,
    pub hold_list: Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>,
    pub hold_channel: Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>,
}
type Job = Box<dyn FnOnce(&mut Files) + Send>;
#[derive(Clone)]
pub(crate) struct FileOwner(mpsc::Sender<Job>);
impl FileOwner {
    fn new() -> Self {
        let (tx, mut rx) = mpsc::channel::<Job>(32);
        tokio::spawn(async move {
            let mut files = Files::default();
            while let Some(job) = rx.recv().await {
                job(&mut files);
            }
        });
        Self(tx)
    }
    pub async fn run<R: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Files) -> R + Send + 'static,
    ) -> R {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(Box::new(move |files| {
                let _ = tx.send(job(files));
            }))
            .await
            .unwrap();
        rx.await.unwrap()
    }
}
pub(crate) struct Fixture {
    pub settings: Settings,
    pub files: FileOwner,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    pub async fn start() -> Self {
        let key =
            russh::keys::PrivateKey::random(&mut rand_sftp::rng(), russh::keys::Algorithm::Ed25519)
                .unwrap();
        let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let settings = Settings {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            username: "fixture-user".into(),
            directory: "/archive".into(),
            fingerprint,
        };
        let files = FileOwner::new();
        let mut peer = Peer {
            files: files.clone(),
        };
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let running = peer.run_on_socket(
                Arc::new(server::Config {
                    keys: vec![key],
                    auth_rejection_time: Duration::ZERO,
                    auth_rejection_time_initial: Some(Duration::ZERO),
                    ..Default::default()
                }),
                &listener,
            );
            let handle = running.handle();
            tokio::pin!(running);
            tokio::select! { result = &mut running => result.unwrap(), _ = stopped => { handle.shutdown("Fixture finished".into()); tokio::time::timeout(Duration::from_secs(2), running).await.unwrap().unwrap(); } }
        });
        Self {
            settings,
            files,
            stop,
            task,
        }
    }
    pub fn provider(&self) -> SftpBackup {
        SftpBackup::new(&self.settings, "fixture-password".into()).unwrap()
    }
    pub async fn finish(self) {
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
    }
}
struct Peer {
    files: FileOwner,
}
impl server::Server for Peer {
    type Handler = Ssh;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        Ssh {
            files: self.files.clone(),
            channels: HashMap::new(),
        }
    }
}
struct Ssh {
    files: FileOwner,
    channels: HashMap<ChannelId, Channel<Msg>>,
}
impl server::Handler for Ssh {
    type Error = anyhow::Error;
    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        Ok(
            if user == "fixture-user" && password == "fixture-password" {
                server::Auth::Accept
            } else {
                server::Auth::Reject {
                    proceed_with_methods: None,
                    partial_success: false,
                }
            },
        )
    }
    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        if let Some((entered, released)) = self.files.run(|files| files.hold_channel.take()).await {
            let _ = entered.send(());
            let _ = released.await;
        }
        reply.accept().await;
        Ok(())
    }
    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        anyhow::ensure!(name == "sftp", "Only SFTP is available in this fixture.");
        let channel = self.channels.remove(&id).unwrap();
        session.channel_success(id)?;
        russh_sftp::server::run(
            channel.into_stream(),
            Sftp {
                files: self.files.clone(),
                handles: HashMap::new(),
                listed: false,
                next: 0,
            },
        )
        .await;
        Ok(())
    }
    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.close(channel)?;
        Ok(())
    }
}
struct Sftp {
    files: FileOwner,
    handles: HashMap<String, String>,
    listed: bool,
    next: u64,
}
fn attrs(size: usize) -> FileAttributes {
    FileAttributes {
        size: Some(size as u64),
        permissions: Some(0o100600),
        mtime: Some(1_789_000_000),
        ..FileAttributes::empty()
    }
}
fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: String::new(),
        language_tag: String::new(),
    }
}
fn filename(path: &str) -> Result<String, StatusCode> {
    path.strip_prefix("/archive/")
        .filter(|name| !name.is_empty() && !name.contains('/') && *name != "." && *name != "..")
        .map(str::to_owned)
        .ok_or(StatusCode::PermissionDenied)
}
impl Sftp {
    async fn metadata(&self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        if path == "/archive" {
            return Ok(Attrs {
                id,
                attrs: FileAttributes {
                    permissions: Some(0o040700),
                    ..FileAttributes::empty()
                },
            });
        }
        let name = filename(&path)?;
        self.files
            .run(move |files| {
                if files.lose_next_stat {
                    files.lose_next_stat = false;
                    return Err(StatusCode::Failure);
                }
                files
                    .data
                    .get(&name)
                    .map(|data| Attrs {
                        id,
                        attrs: attrs(data.len()),
                    })
                    .ok_or(StatusCode::NoSuchFile)
            })
            .await
    }
}
impl russh_sftp::server::Handler for Sftp {
    type Error = StatusCode;
    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }
    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, Self::Error> {
        if path != "/archive" {
            return Err(StatusCode::NoSuchFile);
        }
        Ok(Name {
            id,
            files: vec![File::new(path, FileAttributes::empty())],
        })
    }
    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        self.metadata(id, path).await
    }
    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, Self::Error> {
        self.metadata(
            id,
            self.handles
                .get(&handle)
                .ok_or(StatusCode::Failure)?
                .clone(),
        )
        .await
    }
    async fn open(
        &mut self,
        id: u32,
        path: String,
        flags: OpenFlags,
        _: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        let name = filename(&path)?;
        self.files
            .run(move |files| {
                if flags.contains(OpenFlags::CREATE | OpenFlags::EXCLUDE) {
                    if files.data.contains_key(&name) {
                        return Err(StatusCode::Failure);
                    }
                    files.data.insert(name, Vec::new());
                } else if !files.data.contains_key(&name) {
                    return Err(StatusCode::NoSuchFile);
                }
                if flags.contains(OpenFlags::TRUNCATE) {
                    return Err(StatusCode::PermissionDenied);
                }
                Ok(())
            })
            .await?;
        self.next += 1;
        let handle = format!("file-{}", self.next);
        self.handles.insert(handle.clone(), path);
        Ok(Handle { id, handle })
    }
    async fn close(&mut self, id: u32, handle: String) -> Result<Status, Self::Error> {
        self.handles.remove(&handle).ok_or(StatusCode::Failure)?;
        Ok(ok(id))
    }
    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, Self::Error> {
        let name = filename(self.handles.get(&handle).ok_or(StatusCode::Failure)?)?;
        self.files
            .run(move |files| {
                let data = files.data.get(&name).ok_or(StatusCode::NoSuchFile)?;
                let offset = usize::try_from(offset).map_err(|_| StatusCode::Failure)?;
                if offset >= data.len() {
                    return Err(StatusCode::Eof);
                }
                let count = if files.short_reads { len.min(3) } else { len } as usize;
                Ok(Data {
                    id,
                    data: data[offset..data.len().min(offset + count)].to_vec(),
                })
            })
            .await
    }
    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let name = filename(self.handles.get(&handle).ok_or(StatusCode::Failure)?)?;
        self.files
            .run(move |files| {
                files.writes.push((name.clone(), offset, data.len()));
                if files.fail_write == Some(files.writes.len()) {
                    files.fail_write = None;
                    return Err(StatusCode::Failure);
                }
                let existing = files.data.get_mut(&name).ok_or(StatusCode::NoSuchFile)?;
                let offset = usize::try_from(offset).map_err(|_| StatusCode::Failure)?;
                if offset != existing.len() {
                    return Err(StatusCode::PermissionDenied);
                }
                existing.extend(data);
                Ok(ok(id))
            })
            .await
    }
    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, Self::Error> {
        let old = filename(&oldpath)?;
        let new = filename(&newpath)?;
        self.files
            .run(move |files| {
                if files.data.contains_key(&new) {
                    return Err(StatusCode::Failure);
                }
                let bytes = files.data.remove(&old).ok_or(StatusCode::NoSuchFile)?;
                files.data.insert(new, bytes);
                files.renames += 1;
                if files.lose_rename {
                    files.lose_rename = false;
                    files.lose_next_stat = true;
                    return Err(StatusCode::Failure);
                }
                Ok(ok(id))
            })
            .await
    }
    async fn remove(&mut self, id: u32, path: String) -> Result<Status, Self::Error> {
        let name = filename(&path)?;
        self.files
            .run(move |files| {
                files.data.remove(&name).ok_or(StatusCode::NoSuchFile)?;
                files.deletes.push(name);
                Ok(ok(id))
            })
            .await
    }
    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, Self::Error> {
        if path != "/archive" {
            return Err(StatusCode::NoSuchFile);
        }
        self.handles.insert("directory".into(), path);
        Ok(Handle {
            id,
            handle: "directory".into(),
        })
    }
    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, Self::Error> {
        if handle != "directory" {
            return Err(StatusCode::Failure);
        }
        if let Some((observed, release)) = self.files.run(|files| files.hold_list.take()).await {
            let _ = observed.send(());
            release.await.map_err(|_| StatusCode::Failure)?;
        }
        let listed = self.listed;
        self.listed = true;
        self.files
            .run(move |files| {
                if listed && !files.repeat_listing {
                    return Err(StatusCode::Eof);
                }
                let mut entries = vec![
                    File::new(".", FileAttributes::empty()),
                    File::new("..", FileAttributes::empty()),
                ];
                entries.extend(
                    files
                        .data
                        .iter()
                        .map(|(name, data)| File::new(name.clone(), attrs(data.len()))),
                );
                Ok(Name { id, files: entries })
            })
            .await
    }
}
