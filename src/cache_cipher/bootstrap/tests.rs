use super::*;
use crate::{
    credentials::{Backend, Credentials, Scope},
    profiles::{CATALOG_FILE, Catalog},
    store::Store,
};
use rusqlite::{Connection, OpenFlags};
use secrecy::SecretString;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

const LEGACY: &str = "shep.sqlite";
const BODY: &str = "Fictional message kept through encryption";

/// Blocks the first key write until released, to observe the exclusive phase.
type Gate = Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>;

/// One in-memory credential store shared by every `Keys` the bootstrap builds.
#[derive(Clone, Default)]
struct Memory {
    entries: Arc<Mutex<HashMap<String, SecretString>>>,
    locked: Arc<Mutex<bool>>,
    gate: Arc<Mutex<Gate>>,
    requests: Arc<Mutex<usize>>,
}
impl Backend for Memory {
    fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>> {
        *self.requests.lock().unwrap() += 1;
        anyhow::ensure!(
            !*self.locked.lock().unwrap(),
            "The fixture keychain is locked"
        );
        Ok(self.entries.lock().unwrap().get(key).cloned())
    }
    fn write(&mut self, key: &str, value: SecretString) -> anyhow::Result<()> {
        if let Some((started, held)) = self.gate.lock().unwrap().take() {
            started.send(()).unwrap();
            let _ = held.blocking_recv();
        }
        self.entries.lock().unwrap().insert(key.into(), value);
        Ok(())
    }
    fn delete(&mut self, key: &str) -> anyhow::Result<()> {
        self.entries.lock().unwrap().remove(key);
        Ok(())
    }
}
impl Memory {
    fn source(&self) -> KeySource {
        let memory = self.clone();
        Arc::new(move |id| {
            Keys::with_credentials(Credentials::with_backend(
                Scope::CacheRoot(id),
                memory.clone(),
            ))
        })
    }
    fn only_key(&self) -> Key {
        let entries = self.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        let saved = entries.values().next().unwrap();
        Key::decode(secrecy::ExposeSecret::expose_secret(saved)).unwrap()
    }
}

fn bootstrap(memory: &Memory) -> Bootstrap {
    Bootstrap::start(memory.source()).unwrap()
}

/// A legacy root: WAL cache, catalog, an imported profile and a profile-sync
/// journal, all plaintext and closed.
async fn legacy_root(dir: &Path) -> uuid::Uuid {
    let catalog = Catalog::open(dir, LEGACY).unwrap();
    let (store, _) = catalog.clone().open_active(false).await.unwrap();
    store.put("kept", BODY).await.unwrap();
    let imported = uuid::Uuid::new_v4();
    catalog
        .reserve(imported, "Imported fixture".into())
        .await
        .unwrap();
    let path = catalog.path(crate::profiles::Id::Imported(imported));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let profile = Store::open(&path).unwrap();
    profile
        .put(
            crate::profiles::IMPORT_MARKER_KEY,
            crate::profiles::ImportMarker {
                version: 1,
                local_profile: imported,
                name: "Imported fixture".into(),
            },
        )
        .await
        .unwrap();
    catalog.finish(imported).await.unwrap();
    let sync = dir.join(SYNC_DIRECTORY);
    std::fs::create_dir_all(&sync).unwrap();
    Connection::open(sync.join("drive.sqlite"))
        .unwrap()
        .execute_batch("CREATE TABLE uploads(id TEXT); INSERT INTO uploads VALUES('fictional')")
        .unwrap();
    drop((store, profile, catalog));
    settle(&root_databases(dir, imported));
    imported
}

/// Worker threads close their connections after the last handle drops. Wait
/// until no connection remains open on the fixture databases.
fn settle(paths: &[PathBuf]) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    for path in paths {
        loop {
            // A lock error, like staying in WAL, means a connection is still open.
            let closed = Connection::open(path).is_ok_and(|c| {
                c.query_row("PRAGMA main.journal_mode=DELETE", [], |r| {
                    r.get::<_, String>(0)
                })
                .is_ok_and(|mode| mode.eq_ignore_ascii_case("delete"))
                    && c.execute_batch("PRAGMA main.journal_mode=WAL").is_ok()
            });
            if closed {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "{} stayed open",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn plaintext(path: &Path) -> bool {
    publication::starts_with_plaintext_header(path).unwrap()
}

fn root_databases(dir: &Path, imported: uuid::Uuid) -> Vec<PathBuf> {
    vec![
        dir.join(LEGACY),
        dir.join(CATALOG_FILE),
        dir.join(SYNC_DIRECTORY).join("drive.sqlite"),
        dir.join(PROFILES_DIRECTORY)
            .join(imported.to_string())
            .join("shep.sqlite"),
    ]
}

fn leftovers(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.contains(".partial")
                || name.contains("recovery")
                || name.contains("encryption-journal")
        })
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn plaintext_root_stays_plaintext_and_holds_ownership_without_a_key() {
    let dir = tempfile::tempdir().unwrap();
    let imported = legacy_root(dir.path()).await;
    let memory = Memory::default();
    let root = bootstrap(&memory)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Existing)
        .await
        .unwrap();
    assert!(root.key().is_none());
    assert_eq!(*memory.requests.lock().unwrap(), 0);
    assert!(!dir.path().join(MARKER).exists());
    for path in root_databases(dir.path(), imported) {
        assert!(plaintext(&path), "{}", path.display());
    }
    let catalog = Catalog::open_in(&root, LEGACY).unwrap();
    let (store, _) = catalog.clone().open_active(false).await.unwrap();
    assert_eq!(store.get::<String>("kept").await.unwrap(), BODY);
    assert!(store.connection_key().is_none());
    // Every owner shares the reader guard: no migration can start beneath them.
    drop(root);
    assert!(Guard::migration(dir.path()).is_err());
    assert!(Guard::reader(dir.path()).is_ok());
    drop(catalog);
    assert!(Guard::migration(dir.path()).is_err());
    drop(store);
    wait_for_release(dir.path());
}

fn wait_for_release(dir: &Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while Guard::migration(dir).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "ownership was not released"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[tokio::test]
async fn migration_publishes_every_database_before_opening_and_later_starts_reuse_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let imported = legacy_root(dir.path()).await;
    let memory = Memory::default();
    let root = bootstrap(&memory)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Migrate)
        .await
        .unwrap();
    let key = root.key().expect("a migrated root is keyed");
    assert_eq!(
        key.encode().as_str(),
        memory.only_key().encode().as_str(),
        "the admitted key is the stored key"
    );
    let marker = read_marker(dir.path()).unwrap().unwrap();
    assert!(
        memory
            .entries
            .lock()
            .unwrap()
            .keys()
            .all(|entry| entry.contains(&marker.to_string()))
    );
    for path in root_databases(dir.path(), imported) {
        assert!(!plaintext(&path), "{}", path.display());
        assert!(key.open(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).is_ok());
    }
    assert!(leftovers(dir.path()).is_empty());
    let catalog = Catalog::open_in(&root, LEGACY).unwrap();
    let (store, _) = catalog.clone().open_active(false).await.unwrap();
    assert!(store.connection_key().is_some());
    assert_eq!(store.get::<String>("kept").await.unwrap(), BODY);
    assert_eq!(catalog.page(0).await.unwrap().total, 2);
    let revision = catalog.page(0).await.unwrap().revision;
    catalog
        .activate(crate::profiles::Id::Imported(imported), revision)
        .await
        .unwrap();
    drop((store, catalog, root));
    wait_for_release(dir.path());

    // Production policy reuses the key; a plaintext straggler installed by an
    // older Shep is converted rather than opened beside the encrypted cache.
    let straggler = uuid::Uuid::new_v4();
    let path = dir
        .path()
        .join(PROFILES_DIRECTORY)
        .join(straggler.to_string())
        .join("shep.sqlite");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE kv(key TEXT PRIMARY KEY, value TEXT)")
        .unwrap();
    let root = bootstrap(&memory)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Existing)
        .await
        .unwrap();
    assert_eq!(root.key().unwrap().encode().as_str(), key.encode().as_str());
    assert!(!plaintext(&path));
    let catalog = Catalog::open_in(&root, LEGACY).unwrap();
    let (store, session) = catalog.open_active(false).await.unwrap();
    assert_eq!(session.current, crate::profiles::Id::Imported(imported));
    assert!(store.connection_key().is_some());
}

#[tokio::test]
async fn interrupted_publication_is_recovered_before_the_catalog_opens() {
    let dir = tempfile::tempdir().unwrap();
    let imported = legacy_root(dir.path()).await;
    let memory = Memory::default();
    let id = uuid::Uuid::new_v4();
    let key = futures::executor::block_on(memory.source()(id).create()).unwrap();
    write_marker(dir.path(), id).unwrap();
    let main = dir.path().join(LEGACY);
    {
        let guard = Guard::migration(dir.path()).unwrap();
        let (_send, receive) = oneshot::channel();
        let candidate = migration::stage_plaintext(&main, dir.path(), &key, receive).unwrap();
        let error = publication::publish_with(&guard, candidate, &main, &key, &|step| {
            step == publication::Step::Publish
        })
        .unwrap_err();
        assert!(error.to_string().contains("interrupted"), "{error}");
    }
    assert!(!main.exists());
    assert!(!leftovers(dir.path()).is_empty());
    let root = bootstrap(&memory)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Existing)
        .await
        .unwrap();
    assert!(leftovers(dir.path()).is_empty());
    for path in root_databases(dir.path(), imported) {
        assert!(!plaintext(&path), "{}", path.display());
    }
    let catalog = Catalog::open_in(&root, LEGACY).unwrap();
    let (store, _) = catalog.open_active(false).await.unwrap();
    assert_eq!(store.get::<String>("kept").await.unwrap(), BODY);
}

#[tokio::test]
async fn cancelling_the_open_keeps_the_plaintext_and_releases_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let imported = legacy_root(dir.path()).await;
    let memory = Memory::default();
    let (started, waiting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    *memory.gate.lock().unwrap() = Some((started, held));
    let bootstrap = bootstrap(&memory);
    let opening = tokio::spawn({
        let bootstrap = bootstrap.clone();
        let dir = dir.path().to_owned();
        async move { bootstrap.open(dir, LEGACY.into(), Policy::Migrate).await }
    });
    waiting.await.unwrap();
    assert!(
        Guard::reader(dir.path()).is_err(),
        "exclusive during key admission"
    );
    opening.abort();
    assert!(opening.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    wait_for_release(dir.path());
    for path in root_databases(dir.path(), imported) {
        assert!(plaintext(&path), "{}", path.display());
    }
    assert!(leftovers(dir.path()).is_empty());
    // The key and marker were admitted first; the next start resumes from them.
    assert!(read_marker(dir.path()).unwrap().is_some());
    let root = bootstrap
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Existing)
        .await
        .unwrap();
    assert!(root.key().is_some());
    for path in root_databases(dir.path(), imported) {
        assert!(!plaintext(&path), "{}", path.display());
    }
}

#[tokio::test]
async fn a_locked_key_store_fails_before_any_marker_or_copy_exists() {
    let dir = tempfile::tempdir().unwrap();
    let imported = legacy_root(dir.path()).await;
    let memory = Memory::default();
    *memory.locked.lock().unwrap() = true;
    let error = bootstrap(&memory)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Migrate)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("locked"), "{error}");
    assert!(!dir.path().join(MARKER).exists());
    assert!(memory.entries.lock().unwrap().is_empty());
    assert!(leftovers(dir.path()).is_empty());
    for path in root_databases(dir.path(), imported) {
        assert!(plaintext(&path), "{}", path.display());
    }
    wait_for_release(dir.path());
}

#[tokio::test]
async fn a_keyed_root_with_a_missing_or_wrong_key_is_kept_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let imported = legacy_root(dir.path()).await;
    let memory = Memory::default();
    let root = bootstrap(&memory)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Migrate)
        .await
        .unwrap();
    drop(root);
    wait_for_release(dir.path());
    let before: Vec<Vec<u8>> = root_databases(dir.path(), imported)
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    let id = read_marker(dir.path()).unwrap().unwrap();

    let missing = Memory::default();
    let error = bootstrap(&missing)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Existing)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("missing"), "{error}");
    assert!(
        missing.entries.lock().unwrap().is_empty(),
        "never regenerated"
    );

    let wrong = Memory::default();
    futures::executor::block_on(wrong.source()(id).create()).unwrap();
    let error = bootstrap(&wrong)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Existing)
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("unlocked"), "{error:#}");
    let after: Vec<Vec<u8>> = root_databases(dir.path(), imported)
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    assert_eq!(before, after);
    assert!(leftovers(dir.path()).is_empty());
    wait_for_release(dir.path());
    assert!(
        bootstrap(&memory)
            .open(dir.path().to_owned(), LEGACY.into(), Policy::Existing)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn a_second_shep_cannot_migrate_while_this_one_owns_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let imported = legacy_root(dir.path()).await;
    let memory = Memory::default();
    let root = bootstrap(&memory)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Existing)
        .await
        .unwrap();
    let error = bootstrap(&memory)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Migrate)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("another Shep process"),
        "{error}"
    );
    assert!(!dir.path().join(MARKER).exists());
    for path in root_databases(dir.path(), imported) {
        assert!(plaintext(&path), "{}", path.display());
    }
    // A second cooperating reader of a plaintext root is still admitted, and
    // migration stays excluded until both have released the root.
    let second = bootstrap(&memory)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Existing)
        .await
        .unwrap();
    assert!(second.key().is_none());
    drop(root);
    assert!(Guard::migration(dir.path()).is_err());
    drop(second);
    wait_for_release(dir.path());
}

#[tokio::test]
async fn a_concurrent_start_waits_for_another_shep_to_finish_opening() {
    let dir = tempfile::tempdir().unwrap();
    legacy_root(dir.path()).await;
    let memory = Memory::default();
    // Another Shep is inside its short exclusive opening phase.
    let opening = Guard::migration(dir.path()).unwrap();
    let joined = tokio::spawn({
        let bootstrap = bootstrap(&memory);
        let dir = dir.path().to_owned();
        async move { bootstrap.open(dir, LEGACY.into(), Policy::Existing).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!joined.is_finished());
    drop(opening);
    let root = joined.await.unwrap().unwrap();
    assert!(root.key().is_none());
    assert!(Guard::migration(dir.path()).is_err());
    drop(root);
    wait_for_release(dir.path());
}

/// Models a Shep older than the ownership guard: it opens the plaintext cache
/// in WAL mode and idles without taking any guard.
#[test]
fn legacy_shep_child() {
    let Some(path) = std::env::var_os("SHEP_FIXTURE_LEGACY_CACHE") else {
        return;
    };
    let connection = Connection::open(Path::new(&path)).unwrap();
    connection
        .query_row("SELECT count(*) FROM kv", [], |_| Ok(()))
        .unwrap();
    println!("held");
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    drop(connection);
}

#[tokio::test]
async fn an_idle_legacy_process_blocks_publication_until_it_exits() {
    use std::io::{BufRead, Write};
    let dir = tempfile::tempdir().unwrap();
    let imported = legacy_root(dir.path()).await;
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cache_cipher::bootstrap::tests::legacy_shep_child",
            "--nocapture",
        ])
        .env("SHEP_FIXTURE_LEGACY_CACHE", dir.path().join(LEGACY))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut lines = std::io::BufReader::new(stdout).lines();
    loop {
        let line = lines
            .next()
            .expect("the legacy fixture ended early")
            .unwrap();
        if line.trim() == "held" {
            break;
        }
    }
    let memory = Memory::default();
    let error = bootstrap(&memory)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Migrate)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("still open"), "{error}");
    let main = dir.path().join(LEGACY);
    assert!(plaintext(&main));
    assert!(leftovers(dir.path()).is_empty());
    wait_for_release(dir.path());
    // A legacy process cannot open the published cache as a plaintext file
    // either, so a later legacy start fails instead of resetting mail.
    writeln!(stdin, "done").unwrap();
    drop(stdin);
    assert!(child.wait().unwrap().success());
    let root = bootstrap(&memory)
        .open(dir.path().to_owned(), LEGACY.into(), Policy::Existing)
        .await
        .unwrap();
    for path in root_databases(dir.path(), imported) {
        assert!(!plaintext(&path), "{}", path.display());
    }
    let encrypted = std::fs::read(&main).unwrap();
    assert!(Store::open(&main).is_err());
    assert_eq!(std::fs::read(&main).unwrap(), encrypted);
    drop(root);
}
