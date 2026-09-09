//! Loopback FTP/FTPS peer with one bounded owner for files and observations.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    task::JoinSet,
};
trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}
type Stream = BufReader<Box<dyn Io>>;
#[derive(Default)]
pub(crate) struct Files {
    pub dirs: BTreeSet<String>,
    pub data: BTreeMap<String, Vec<u8>>,
    pub commands: Vec<String>,
    pub passwords: usize,
    pub clear_passwords: usize,
    pub partial_archive: bool,
    pub partial_commit: bool,
    pub lose_commit: bool,
    pub fail_next_manifest: bool,
    pub repeated_listing: bool,
    pub hold_list: Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>,
}
type Job = Box<dyn FnOnce(&mut Files) + Send>;
#[derive(Clone)]
pub(crate) struct FileOwner(mpsc::Sender<Job>);
impl FileOwner {
    fn new() -> Self {
        let (tx, mut rx) = mpsc::channel::<Job>(32);
        tokio::spawn(async move {
            let mut files = Files::default();
            files.dirs.insert("/archive".into());
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
    pub certificate: Arc<[u8]>,
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}
impl Fixture {
    pub async fn start(security: Security) -> Self {
        let rcgen::CertifiedKey { cert, signing_key } =
            rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()])
                .unwrap();
        let certificate: Arc<[u8]> = cert.pem().as_bytes().into();
        let identity =
            native_tls::Identity::from_pkcs8(&certificate, signing_key.serialize_pem().as_bytes())
                .unwrap();
        let tls =
            tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::new(identity).unwrap());
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let settings = Settings {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            username: "fixture-user".into(),
            directory: "/archive".into(),
            security,
        };
        let files = FileOwner::new();
        let peer = files.clone();
        let (stop, mut stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! { _=&mut stopped => break, connection = listener.accept() => { let (stream,_) = connection.unwrap(); let files=peer.clone(); let tls=tls.clone(); tasks.spawn(async move { let _=session(stream, tls, security, files).await; }); }, Some(_) = tasks.join_next(), if !tasks.is_empty() => {} }
            }
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        });
        Self {
            settings,
            files,
            certificate,
            stop,
            task,
        }
    }
    pub fn provider(&self) -> FtpBackup {
        let mut provider = FtpBackup::new(&self.settings, "fixture-password".into()).unwrap();
        provider.ca = Some(self.certificate.clone());
        provider
    }
    pub async fn finish(self) {
        self.stop.send(()).unwrap();
        self.task.await.unwrap();
    }
}
async fn reply(stream: &mut Stream, text: &str) -> anyhow::Result<()> {
    stream.get_mut().write_all(text.as_bytes()).await?;
    stream.get_mut().flush().await?;
    Ok(())
}
fn absolute(path: &str, cwd: &str) -> String {
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("{cwd}/{path}")
    };
    format!("/{}", path.trim_matches('/'))
}
async fn session(
    tcp: TcpStream,
    tls: tokio_native_tls::TlsAcceptor,
    security: Security,
    files: FileOwner,
) -> anyhow::Result<()> {
    let mut encrypted = security == Security::ImplicitTls;
    let io: Box<dyn Io> = if encrypted {
        Box::new(tls.accept(tcp).await?)
    } else {
        Box::new(tcp)
    };
    let mut stream = BufReader::new(io);
    reply(&mut stream, "220 Fixture FTP ready\r\n").await?;
    let mut cwd = "/".to_owned();
    let mut passive = None;
    let mut private_data = false;
    loop {
        let mut line = String::new();
        if stream.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let (command, argument) = line
            .trim_end_matches(['\r', '\n'])
            .split_once(' ')
            .unwrap_or((line.trim(), ""));
        let command = command.to_ascii_uppercase();
        let observation = command.clone();
        files.run(move |f| f.commands.push(observation)).await;
        match command.as_str() {
            "AUTH" => {
                reply(&mut stream, "234 Begin TLS\r\n").await?;
                let raw = stream.into_inner();
                stream = BufReader::new(Box::new(tls.accept(raw).await?));
                encrypted = true;
            }
            "USER" => reply(&mut stream, "331 Password required\r\n").await?,
            "PASS" => {
                files
                    .run(move |f| {
                        f.passwords += 1;
                        if !encrypted {
                            f.clear_passwords += 1;
                        }
                    })
                    .await;
                reply(
                    &mut stream,
                    if argument == "fixture-password" {
                        "230 Logged in\r\n"
                    } else {
                        "530 Rejected\r\n"
                    },
                )
                .await?;
            }
            "PBSZ" => reply(&mut stream, "200 Buffer accepted\r\n").await?,
            "PROT" => {
                private_data = argument == "P";
                reply(&mut stream, "200 Protection accepted\r\n").await?;
            }
            "PWD" => reply(&mut stream, &format!("257 \"{cwd}\"\r\n")).await?,
            "CWD" => {
                let path = absolute(argument, &cwd);
                let check = path.clone();
                if files
                    .run(move |f| check == "/" || f.dirs.contains(&check))
                    .await
                {
                    cwd = if path == "/fixture-alias" {
                        "/archive".into()
                    } else {
                        path
                    };
                    reply(&mut stream, "250 Directory changed\r\n").await?;
                } else {
                    reply(&mut stream, "550 No directory\r\n").await?;
                }
            }
            "TYPE" | "OPTS" | "NOOP" => reply(&mut stream, "200 Accepted\r\n").await?,
            "SYST" => reply(&mut stream, "215 UNIX Type L8\r\n").await?,
            "FEAT" => {
                reply(
                    &mut stream,
                    "211-Features\r\n MLST type*;size*;\r\n AUTH TLS\r\n211 End\r\n",
                )
                .await?
            }
            "EPSV" | "PASV" => {
                let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
                let port = listener.local_addr()?.port();
                passive = Some(listener);
                if command == "EPSV" {
                    reply(
                        &mut stream,
                        &format!("229 Entering Extended Passive Mode (|||{port}|)\r\n"),
                    )
                    .await?;
                } else {
                    reply(
                        &mut stream,
                        &format!(
                            "227 Entering Passive Mode (192,0,2,99,{},{})\r\n",
                            port / 256,
                            port % 256
                        ),
                    )
                    .await?;
                }
            }
            "SIZE" => {
                let path = absolute(argument, &cwd);
                let size = files.run(move |f| f.data.get(&path).map(Vec::len)).await;
                reply(
                    &mut stream,
                    &size.map_or_else(
                        || "550 No file\r\n".to_string(),
                        |size| format!("213 {size}\r\n"),
                    ),
                )
                .await?;
            }
            "MKD" => {
                let path = absolute(argument, &cwd);
                let inserted = files.run(move |f| f.dirs.insert(path)).await;
                reply(
                    &mut stream,
                    if inserted {
                        "257 Created\r\n"
                    } else {
                        "550 Exists\r\n"
                    },
                )
                .await?;
            }
            "DELE" => {
                let path = absolute(argument, &cwd);
                let removed = files.run(move |f| f.data.remove(&path).is_some()).await;
                reply(
                    &mut stream,
                    if removed {
                        "250 Deleted\r\n"
                    } else {
                        "550 Missing\r\n"
                    },
                )
                .await?;
            }
            "RMD" => {
                let path = absolute(argument, &cwd);
                let removed = files
                    .run(move |f| {
                        if f.data
                            .keys()
                            .any(|key| key.starts_with(&format!("{path}/")))
                        {
                            false
                        } else {
                            f.dirs.remove(&path)
                        }
                    })
                    .await;
                reply(
                    &mut stream,
                    if removed {
                        "250 Removed\r\n"
                    } else {
                        "550 Not empty\r\n"
                    },
                )
                .await?;
            }
            "MLSD" | "RETR" | "STOR" | "APPE" => {
                let path = absolute(argument, &cwd);
                if command == "RETR"
                    && path.ends_with(COMMIT)
                    && files
                        .run(|f| std::mem::take(&mut f.fail_next_manifest))
                        .await
                {
                    reply(&mut stream, "450 Temporary read failure\r\n").await?;
                    passive = None;
                    continue;
                }
                let data = if command == "MLSD" {
                    let listing = path.clone();
                    if let Some((started, release)) = files.run(|f| f.hold_list.take()).await {
                        let _ = started.send(());
                        let _ = release.await;
                    }
                    files
                        .run(move |f| {
                            let prefix = format!("{}/", listing.trim_end_matches('/'));
                            let mut lines = Vec::new();
                            for name in &f.dirs {
                                if let Some(child) = name.strip_prefix(&prefix)
                                    && !child.is_empty()
                                    && !child.contains('/')
                                {
                                    lines.push(format!("type=dir; {child}\r\n"));
                                }
                            }
                            for (name, data) in &f.data {
                                if let Some(child) = name.strip_prefix(&prefix)
                                    && !child.contains('/')
                                {
                                    lines.push(format!(
                                        "type=file;size={}; {child}\r\n",
                                        data.len()
                                    ));
                                }
                            }
                            if f.repeated_listing
                                && let Some(first) = lines.first().cloned()
                            {
                                lines.push(first);
                            }
                            Some(lines.concat().into_bytes())
                        })
                        .await
                } else if command == "RETR" {
                    let path = path.clone();
                    files.run(move |f| f.data.get(&path).cloned()).await
                } else {
                    Some(Vec::new())
                };
                let Some(data) = data else {
                    reply(&mut stream, "550 No file\r\n").await?;
                    passive = None;
                    continue;
                };
                reply(&mut stream, "150 Data connection\r\n").await?;
                let (tcp, _) = passive
                    .take()
                    .context("No passive listener")?
                    .accept()
                    .await?;
                let mut data_stream: Box<dyn Io> = if private_data {
                    Box::new(tls.accept(tcp).await?)
                } else {
                    Box::new(tcp)
                };
                if command == "STOR" || command == "APPE" {
                    let mut bytes = Vec::new();
                    data_stream.read_to_end(&mut bytes).await?;
                    let append = command == "APPE";
                    let archive = path.ends_with(ARCHIVE);
                    let commit = path.ends_with(COMMIT);
                    let (partial, lost) = files
                        .run(move |f| {
                            let partial = (archive && std::mem::take(&mut f.partial_archive))
                                || (commit && std::mem::take(&mut f.partial_commit));
                            if partial {
                                bytes.truncate(12);
                            }
                            if append {
                                f.data.entry(path).or_default().extend(bytes);
                            } else {
                                f.data.insert(path, bytes);
                            }
                            let lost = commit && std::mem::take(&mut f.lose_commit);
                            if lost {
                                f.fail_next_manifest = true;
                            }
                            (partial, lost)
                        })
                        .await;
                    data_stream.shutdown().await?;
                    if lost {
                        return Ok(());
                    }
                    if partial {
                        reply(&mut stream, "426 Transfer interrupted\r\n").await?;
                        continue;
                    }
                } else {
                    data_stream.write_all(&data).await?;
                    data_stream.shutdown().await?;
                }
                drop(data_stream);
                reply(&mut stream, "226 Transfer complete\r\n").await?;
            }
            "QUIT" => {
                reply(&mut stream, "221 Goodbye\r\n").await?;
                return Ok(());
            }
            _ => reply(&mut stream, "502 Unsupported\r\n").await?,
        }
    }
}
