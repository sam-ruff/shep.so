//! One native application owner per canonical workspace root.
mod identity;
mod ipc;
mod signal;

use anyhow::Context;
use fs2::FileExt;
use identity::{Identity, Relation, relation};
use ipc::{Reply, Transport};
use serde::{Deserialize, Serialize};
pub(crate) use signal::subscription;
pub use signal::{Request, Signal};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    time::Duration,
};
// Tokio's clock so the wait bounds can be tested under paused time.
use tokio::time::Instant;
use uuid::Uuid;

const ENDPOINT_PREFIX: &str = "shep-activate-v1-";
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(8);
/// How long a newer build keeps asking an older owner to quit for it.
const RESTART_REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
/// How long an owner that agreed to quit may take to release the lock.
const RESTART_EXIT_TIMEOUT: Duration = Duration::from_secs(20);

/// Shown when an older owner keeps running; no em dash, no emoji.
pub const STALE_OWNER_NOTICE: &str =
    "Shep is already running an older version. Quit it from the tray and open Shep again.";

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Record {
    endpoint: String,
    secret: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    identity: Option<Identity>,
}

impl Record {
    fn new(identity: Option<Identity>) -> Self {
        Self {
            endpoint: format!("{ENDPOINT_PREFIX}{}", Uuid::new_v4()),
            secret: Uuid::new_v4(),
            identity,
        }
    }

    fn valid(&self) -> bool {
        self.endpoint
            .strip_prefix(ENDPOINT_PREFIX)
            .and_then(|id| Uuid::parse_str(id).ok())
            .is_some()
    }
}

pub enum Launch {
    Primary(Owner),
    Activated,
    Independent,
    /// An older build owns the workspace and did not quit for this one.
    Stale,
}

pub struct Owner {
    server: Option<ipc::Server>,
    file: File,
    publication: File,
    signal: Signal,
}

impl Owner {
    pub fn signal(&self) -> Signal {
        self.signal.clone()
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        self.server.take();
        let _ = self.publication.set_len(0);
        let _ = FileExt::unlock(&self.file);
    }
}

/// A `mailto` link is handed to a running owner so it opens a draft.
pub fn start(mailto: Option<String>) -> anyhow::Result<Launch> {
    let demo = cfg!(feature = "test-support") && std::env::args_os().any(|arg| arg == "--demo");
    let root = crate::engine::workspace_location(demo)?.map(|(root, _)| root);
    #[cfg(feature = "test-support")]
    let root = root.or_else(|| {
        if !demo {
            return None;
        }
        let args: Vec<_> = std::env::args_os().collect();
        args.windows(2)
            .find(|pair| pair[0] == "--test-state")
            .and_then(|pair| Path::new(&pair[1]).parent().map(Path::to_owned))
    });
    let Some(root) = root else {
        return Ok(Launch::Independent);
    };
    let identity = launch_identity(demo);
    if identity.is_none() {
        tracing::warn!("The executable could not be identified; launches hand over as before");
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    match mailto {
        Some(link) => runtime.block_on(acquire(&root, &ipc::Compose(link), identity)),
        None => runtime.block_on(acquire(&root, &ipc::Local, identity)),
    }
}

/// Test fixtures spoof the executable through `SHEP_TEST_BINARY_IDENTITY`:
/// an inode number, or `none` for an owner built before identities.
fn launch_identity(demo: bool) -> Option<Identity> {
    let identity = Identity::current();
    #[cfg(feature = "test-support")]
    if demo && let Ok(spoof) = std::env::var("SHEP_TEST_BINARY_IDENTITY") {
        if spoof == "none" {
            return None;
        }
        return match spoof.parse() {
            Ok(inode) => identity.map(|identity| identity.spoofed(inode)),
            Err(_) => identity,
        };
    }
    #[cfg(not(feature = "test-support"))]
    let _ = demo;
    identity
}

async fn acquire(
    root: &Path,
    transport: &impl Transport,
    identity: Option<Identity>,
) -> anyhow::Result<Launch> {
    std::fs::create_dir_all(root).context("Could not create the Shep workspace directory")?;
    let root = root.canonicalize()?;
    let file = open_private(&root.join(".launch.lock"))?;
    let mut publication = open_private(&root.join(".launch.endpoint"))?;
    let started = Instant::now();
    let deadline = started + LAUNCH_TIMEOUT;
    let mut restarting: Option<Instant> = None;
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => {
                cleanup_endpoint(&mut publication)?;
                let signal = Signal::default();
                let record = Record::new(identity);
                let mut owner = Owner {
                    server: None,
                    file,
                    publication,
                    signal,
                };
                owner.server = Some(ipc::Server::start(record.clone(), owner.signal.clone())?);
                owner.publication.set_len(0)?;
                owner.publication.seek(SeekFrom::Start(0))?;
                serde_json::to_writer(&mut owner.publication, &record)?;
                owner.publication.flush()?;
                return Ok(Launch::Primary(owner));
            }
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {}
            Err(error) => return Err(error.into()),
        }
        if let Some(record) = read_record(&mut publication)? {
            match relation(identity.as_ref(), record.identity.as_ref()) {
                Relation::Same => {
                    if matches!(transport.request(record).await, Ok(Reply::Accepted)) {
                        return Ok(Launch::Activated);
                    }
                }
                // An owner from before identities cannot restart itself.
                Relation::Older => return Ok(Launch::Stale),
                Relation::Different if restarting.is_none() => {
                    match transport.restart(record).await {
                        Ok(Reply::Restarting | Reply::Closing) => restarting = Some(Instant::now()),
                        Ok(Reply::Accepted) | Err(_) => {
                            if started.elapsed() >= RESTART_REQUEST_TIMEOUT {
                                return Ok(Launch::Stale);
                            }
                        }
                    }
                }
                Relation::Different => {}
            }
        }
        match restarting {
            Some(acknowledged) => {
                if acknowledged.elapsed() >= RESTART_EXIT_TIMEOUT {
                    return Ok(Launch::Stale);
                }
            }
            None => anyhow::ensure!(
                Instant::now() < deadline,
                "The running Shep could not be activated. Try opening it again after it finishes closing."
            ),
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn open_private(path: &Path) -> anyhow::Result<File> {
    if let Ok(metadata) = path.symlink_metadata() {
        anyhow::ensure!(metadata.is_file(), "The launch lock must be a regular file");
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    anyhow::ensure!(
        file.metadata()?.is_file(),
        "The launch lock must be a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        let named = path.symlink_metadata()?;
        anyhow::ensure!(
            named.is_file()
                && metadata.ino() == named.ino()
                && metadata.dev() == named.dev()
                && metadata.nlink() == 1
                && metadata.mode() & 0o077 == 0,
            "The launch lock must be private and have one link"
        );
    }
    Ok(file)
}

fn cleanup_endpoint(file: &mut File) -> anyhow::Result<()> {
    // GenericNamespaced uses filesystem sockets on other Unix platforms.
    #[cfg(all(unix, not(target_os = "linux")))]
    if let Some(record) = read_record(file)? {
        use std::os::unix::fs::FileTypeExt;
        let path = Path::new("/tmp").join(record.endpoint);
        if path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_socket())
        {
            std::fs::remove_file(path)?;
        }
    }
    #[cfg(not(all(unix, not(target_os = "linux"))))]
    let _ = file;
    Ok(())
}

fn read_record(file: &mut File) -> anyhow::Result<Option<Record>> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(1025).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 {
        return Ok(None);
    }
    Ok(serde_json::from_slice::<Record>(&bytes)
        .ok()
        .filter(Record::valid))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::{Local, MockTransport};

    async fn owner(root: &Path) -> Owner {
        let mut transport = MockTransport::new();
        transport.expect_request().times(0);
        transport.expect_restart().times(0);
        match acquire(root, &transport, Identity::current())
            .await
            .expect("own fixture root")
        {
            Launch::Primary(owner) => owner,
            _ => panic!("fixture root should have a new owner"),
        }
    }

    fn other_binary() -> Option<Identity> {
        Identity::of(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("Cargo.toml")
                .as_path(),
        )
    }

    fn acknowledging(reply: Reply) -> MockTransport {
        let mut transport = MockTransport::new();
        transport.expect_request().times(0);
        transport
            .expect_restart()
            .times(1)
            .returning(move |_| Ok(reply));
        transport
    }

    #[tokio::test]
    async fn newer_build_asks_owner_to_quit_and_then_becomes_primary() {
        let root = tempfile::tempdir().expect("fixture directory");
        let mut primary = Some(owner(root.path()).await);
        let mut transport = MockTransport::new();
        transport.expect_request().times(0);
        transport.expect_restart().times(1).returning(move |_| {
            primary.take();
            Ok(Reply::Restarting)
        });
        let next = acquire(root.path(), &transport, other_binary()).await;
        assert!(matches!(next.expect("next owner"), Launch::Primary(_)));
    }

    #[tokio::test(start_paused = true)]
    async fn owner_that_never_answers_a_restart_leaves_a_notice() {
        let root = tempfile::tempdir().expect("fixture directory");
        let _primary = owner(root.path()).await;
        let mut transport = MockTransport::new();
        transport.expect_request().times(0);
        transport
            .expect_restart()
            .returning(|_| Err(anyhow::anyhow!("no acknowledgment")));
        let started = Instant::now();
        let launch = acquire(root.path(), &transport, other_binary()).await;
        assert!(matches!(launch.expect("bounded wait"), Launch::Stale));
        assert!(started.elapsed() >= RESTART_REQUEST_TIMEOUT);
        assert!(started.elapsed() < LAUNCH_TIMEOUT);
    }

    #[tokio::test(start_paused = true)]
    async fn acknowledged_owner_that_keeps_running_leaves_a_notice_after_its_bound() {
        let root = tempfile::tempdir().expect("fixture directory");
        let _primary = owner(root.path()).await;
        let transport = acknowledging(Reply::Restarting);
        let started = Instant::now();
        let launch = acquire(root.path(), &transport, other_binary()).await;
        assert!(matches!(launch.expect("bounded wait"), Launch::Stale));
        assert!(started.elapsed() >= RESTART_EXIT_TIMEOUT);
    }

    #[tokio::test]
    async fn closing_owner_answers_a_restart_and_releases_to_the_newer_build() {
        let root = tempfile::tempdir().expect("fixture directory");
        let primary = owner(root.path()).await;
        assert!(primary.signal.try_close());
        let mut primary = Some(primary);
        let mut transport = MockTransport::new();
        transport.expect_request().times(0);
        transport.expect_restart().times(1).returning(move |_| {
            primary.take();
            Ok(Reply::Closing)
        });
        let next = acquire(root.path(), &transport, other_binary()).await;
        assert!(matches!(next.expect("next owner"), Launch::Primary(_)));
    }

    #[tokio::test]
    async fn owner_published_before_identities_is_stale_without_a_request() {
        let root = tempfile::tempdir().expect("fixture directory");
        let mut primary = owner(root.path()).await;
        let mut record = read_record(&mut primary.publication)
            .expect("record")
            .expect("published");
        record.identity = None;
        primary.publication.set_len(0).expect("clear");
        primary
            .publication
            .seek(SeekFrom::Start(0))
            .expect("rewind");
        serde_json::to_writer(&primary.publication, &record).expect("older publication");
        let mut transport = MockTransport::new();
        transport.expect_request().times(0);
        transport.expect_restart().times(0);
        let launch = acquire(root.path(), &transport, Identity::current()).await;
        assert!(matches!(launch.expect("stale owner"), Launch::Stale));
        // A launcher that cannot identify itself keeps the plain handover.
        let mut transport = MockTransport::new();
        transport.expect_restart().times(0);
        transport
            .expect_request()
            .times(1)
            .returning(|_| Ok(Reply::Accepted));
        let launch = acquire(root.path(), &transport, None).await;
        assert!(matches!(launch.expect("handover"), Launch::Activated));
    }

    #[tokio::test]
    async fn local_socket_restart_reaches_ui_and_closing_owner_declines() {
        use iced::futures::StreamExt;
        let root = tempfile::tempdir().expect("fixture directory");
        let mut primary = owner(root.path()).await;
        let record = read_record(&mut primary.publication)
            .expect("record")
            .expect("published");
        assert!(record.identity.is_some(), "the owner publishes its binary");
        let signal = primary.signal();
        let subscription_signal = Some(signal.clone());
        let mut events = Box::pin(subscription(&subscription_signal));
        let request = Local.restart(record.clone());
        let handling = async {
            let request = tokio::time::timeout(Duration::from_secs(2), events.next())
                .await
                .expect("restart arrives")
                .expect("request");
            let Request::Restart(generation) = request else {
                panic!("a restart must not arrive as Open: {request:?}");
            };
            assert!(!signal.try_close());
            signal.acknowledge(generation);
        };
        let (reply, ()) = tokio::join!(request, handling);
        assert_eq!(reply.expect("acknowledged"), Reply::Restarting);
        assert!(signal.try_close());
        assert_eq!(
            Local.restart(record).await.expect("closing response"),
            Reply::Closing
        );
    }

    #[tokio::test]
    async fn primary_skips_transport_and_independent_roots_remain_independent() {
        let first = tempfile::tempdir().expect("fixture directory");
        let second = tempfile::tempdir().expect("fixture directory");
        let first_owner = owner(first.path()).await;
        let second_owner = owner(second.path()).await;
        assert!(first_owner.signal.try_close());
        assert!(second_owner.signal.try_close());
        drop(first_owner);
        drop(owner(first.path()).await);
        assert!(first.path().join(".launch.lock").is_file());
    }

    #[tokio::test]
    async fn secondary_retries_transport_error_and_closing_without_new_owner() {
        let root = tempfile::tempdir().expect("fixture directory");
        let _owner = owner(root.path()).await;
        let mut transport = MockTransport::new();
        let mut sequence = mockall::Sequence::new();
        transport
            .expect_request()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Err(anyhow::anyhow!("temporary connection failure")));
        transport
            .expect_request()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Ok(Reply::Closing));
        transport
            .expect_request()
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_| Ok(Reply::Accepted));
        assert!(matches!(
            acquire(root.path(), &transport, Identity::current())
                .await
                .expect("activate"),
            Launch::Activated
        ));
    }

    #[tokio::test]
    async fn closing_owner_releases_before_secondary_becomes_primary() {
        let root = tempfile::tempdir().expect("fixture directory");
        let primary = owner(root.path()).await;
        assert!(primary.signal.try_close());
        let mut primary = Some(primary);
        let mut transport = MockTransport::new();
        transport.expect_request().times(1).returning(move |_| {
            primary.take();
            Ok(Reply::Closing)
        });
        assert!(matches!(
            acquire(root.path(), &transport, Identity::current())
                .await
                .expect("next owner"),
            Launch::Primary(_)
        ));
    }

    #[tokio::test]
    async fn local_socket_waits_for_ui_acknowledgment_and_rejects_wrong_secret() {
        use iced::futures::StreamExt;
        let root = tempfile::tempdir().expect("fixture directory");
        let mut primary = owner(root.path()).await;
        let record = read_record(&mut primary.publication)
            .expect("record")
            .expect("published");
        let mut invalid = record.clone();
        invalid.secret = Uuid::new_v4();
        assert!(Local.request(invalid).await.is_err());
        let signal = primary.signal();
        let subscription_signal = Some(signal.clone());
        let mut events = Box::pin(subscription(&subscription_signal));
        let request = Local.request(record.clone());
        let handling = async {
            let request = tokio::time::timeout(Duration::from_secs(2), events.next())
                .await
                .expect("activation arrives")
                .expect("request");
            let Request::Open(generation) = request else {
                panic!("an Open must not arrive as a restart: {request:?}");
            };
            assert!(!signal.try_close());
            signal.acknowledge(generation);
        };
        let (reply, ()) = tokio::join!(request, handling);
        assert_eq!(reply.expect("acknowledged"), Reply::Accepted);
        assert!(signal.try_close());
        assert_eq!(
            Local.request(record).await.expect("closing response"),
            Reply::Closing
        );
    }

    #[tokio::test]
    async fn local_socket_hands_a_mailto_link_to_the_owner() {
        use iced::futures::StreamExt;
        let root = tempfile::tempdir().expect("fixture directory");
        let mut primary = owner(root.path()).await;
        let record = read_record(&mut primary.publication)
            .expect("record")
            .expect("published");
        let signal = primary.signal();
        let subscription_signal = Some(signal.clone());
        let mut events = Box::pin(subscription(&subscription_signal));
        let link = "mailto:friend@example.test?subject=Hi";
        let compose = ipc::Compose(link.into());
        let request = compose.request(record.clone());
        let handling = async {
            let request = tokio::time::timeout(Duration::from_secs(2), events.next())
                .await
                .expect("activation arrives")
                .expect("request");
            let Request::Open(generation) = request else {
                panic!("a compose must arrive as an Open: {request:?}");
            };
            assert_eq!(signal.take_compositions(), [link]);
            signal.acknowledge(generation);
        };
        let (reply, ()) = tokio::join!(request, handling);
        assert_eq!(reply.expect("acknowledged"), Reply::Accepted);

        assert!(
            ipc::Compose("https://example.test".into())
                .request(record)
                .await
                .is_err(),
            "the owner drops links that are not mailto"
        );
        assert!(signal.take_compositions().is_empty());
    }

    #[tokio::test]
    async fn stale_publication_is_replaced_under_the_same_lock_inode() {
        let root = tempfile::tempdir().expect("fixture directory");
        let mut primary = owner(root.path()).await;
        let previous = read_record(&mut primary.publication)
            .expect("record")
            .expect("published");
        #[cfg(unix)]
        let inode = {
            use std::os::unix::fs::MetadataExt;
            primary.file.metadata().expect("lock metadata").ino()
        };
        drop(primary);
        let path = root.path().join(".launch.endpoint");
        std::fs::write(&path, serde_json::to_vec(&previous).expect("record bytes"))
            .expect("simulate stale record");
        let mut next = owner(root.path()).await;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                inode,
                next.file.metadata().expect("same lock metadata").ino()
            );
        }
        let current = read_record(&mut next.publication)
            .expect("record")
            .expect("published");
        assert_ne!(previous.endpoint, current.endpoint);
        assert!(Local.request(previous).await.is_err());
    }

    #[test]
    fn incomplete_or_oversized_publication_is_not_an_endpoint() {
        let mut file = tempfile::tempfile().expect("publication fixture");
        file.write_all(b"{\"endpoint\":").expect("partial write");
        assert!(read_record(&mut file).expect("partial record").is_none());
        file.set_len(0).expect("clear fixture");
        file.seek(SeekFrom::Start(0)).expect("rewind");
        file.write_all(&[b' '; 1025]).expect("oversized write");
        assert!(read_record(&mut file).expect("bounded record").is_none());
    }

    #[tokio::test]
    async fn slow_connection_does_not_block_open_or_server_shutdown() {
        use iced::futures::StreamExt;
        use interprocess::local_socket::{
            GenericNamespaced,
            tokio::{Stream, prelude::*},
        };
        let root = tempfile::tempdir().expect("fixture directory");
        let mut primary = owner(root.path()).await;
        let record = read_record(&mut primary.publication)
            .expect("record")
            .expect("published");
        let name = record
            .endpoint
            .as_str()
            .to_ns_name::<GenericNamespaced>()
            .expect("socket name");
        let _slow = Stream::connect(name).await.expect("idle client");
        let signal = primary.signal();
        let binding = Some(signal.clone());
        let mut events = Box::pin(subscription(&binding));
        let handling = async {
            let request = events.next().await.expect("open generation");
            let (Request::Open(generation) | Request::Restart(generation)) = request;
            signal.acknowledge(generation);
        };
        let (reply, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(Local.request(record), handling)
        })
        .await
        .expect("idle client cannot block Open");
        assert_eq!(reply.expect("Open accepted"), Reply::Accepted);
        let before = Instant::now();
        drop(primary);
        assert!(before.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn activation_child_owner() {
        let Some(root) = std::env::var_os("SHEP_ACTIVATION_CHILD_ROOT") else {
            return;
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("child runtime");
        let _owner = runtime.block_on(owner(Path::new(&root)));
        std::fs::write(Path::new(&root).join("child-ready"), b"ready")
            .expect("child fixture marker");
        loop {
            std::thread::park();
        }
    }

    #[test]
    fn crashed_process_releases_lock_and_stale_endpoint_can_be_replaced() {
        let root = tempfile::tempdir().expect("fixture directory");
        let mut child = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "activation::tests::activation_child_owner",
                "--nocapture",
            ])
            .env("SHEP_ACTIVATION_CHILD_ROOT", root.path())
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("owned child process");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !root.path().join("child-ready").exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let ready = root.path().join("child-ready").exists();
        child.kill().expect("kill only owned fixture child");
        child.wait().expect("reap owned child");
        assert!(ready, "child published ownership before crash");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        drop(runtime.block_on(owner(root.path())));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn canonical_alias_activates_owner_and_symlink_lock_is_rejected() {
        use std::os::unix::fs::symlink;
        let parent = tempfile::tempdir().expect("fixture directory");
        let root = parent.path().join("actual");
        let _primary = owner(&root).await;
        let alias = parent.path().join("alias");
        symlink(&root, &alias).expect("root alias");
        let mut transport = MockTransport::new();
        transport
            .expect_request()
            .times(1)
            .returning(|_| Ok(Reply::Accepted));
        assert!(matches!(
            acquire(&alias, &transport, Identity::current())
                .await
                .expect("activate alias"),
            Launch::Activated
        ));
        let other = tempfile::tempdir().expect("fixture directory");
        symlink(root.join(".launch.lock"), other.path().join(".launch.lock")).expect("lock alias");
        assert!(
            acquire(other.path(), &transport, Identity::current())
                .await
                .is_err()
        );
    }
}
