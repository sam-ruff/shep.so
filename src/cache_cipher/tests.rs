use super::*;

fn writable() -> OpenFlags {
    OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
}

#[test]
fn key_roundtrip_rejects_invalid_material_and_debug_never_reveals_it() {
    let key = Key::generate().unwrap();
    let encoded = key.encode();
    assert_eq!(
        Key::decode(&encoded).unwrap().encode().as_str(),
        encoded.as_str()
    );
    assert_eq!(format!("{key:?}"), "Key([REDACTED])");
    for invalid in [
        "",
        "password",
        "shep-cache-key-v1:00",
        "shep-cache-key-v2:00",
    ] {
        assert!(Key::decode(invalid).is_err());
    }
    let invalid = format!("{KEY_PREFIX}{}", "x".repeat(64));
    assert!(Key::decode(&invalid).is_err());
}

#[test]
fn encrypted_database_and_wal_restart_without_plaintext_content() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.sqlite");
    let key = Key::generate().unwrap();
    let c = key.open(&path, writable()).unwrap();
    c.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE secrets(value TEXT);",
    )
    .unwrap();
    let value = "A distinctive fictional private message in the encrypted mail cache".repeat(500);
    c.execute("INSERT INTO secrets VALUES(?)", [&value])
        .unwrap();
    for path in [&path, &dir.path().join("cache.sqlite-wal")] {
        let bytes = std::fs::read(path).unwrap();
        assert!(
            !bytes
                .windows(24)
                .any(|chunk| chunk == &value.as_bytes()[..24])
        );
        assert!(!bytes.starts_with(b"SQLite format 3\0"));
    }
    drop(c);
    let reopened = key.open(&path, writable()).unwrap();
    assert_eq!(
        reopened
            .query_row("SELECT value FROM secrets", [], |r| r.get::<_, String>(0))
            .unwrap(),
        value
    );
    assert!(rusqlite::version_number() >= 3_053_004);
    let version: String = reopened
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .unwrap();
    assert!(version.starts_with("4.19.0"));
}

#[test]
fn wrong_missing_keys_and_corrupt_pages_fail_without_mutating_original() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.sqlite");
    let key = Key::generate().unwrap();
    let c = key.open(&path, writable()).unwrap();
    c.execute_batch(
        "CREATE TABLE secrets(value TEXT); INSERT INTO secrets VALUES('Fictional cached body');",
    )
    .unwrap();
    drop(c);
    let original = std::fs::read(&path).unwrap();
    assert!(Key::generate().unwrap().open(&path, writable()).is_err());
    let plain = Connection::open(&path).unwrap();
    assert!(
        plain
            .query_row("SELECT value FROM secrets", [], |r| r.get::<_, String>(0))
            .is_err()
    );
    drop(plain);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let mut corrupted = original;
    corrupted[100] ^= 0x40;
    std::fs::write(&path, &corrupted).unwrap();
    assert!(key.open(&path, writable()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), corrupted);
}

#[tokio::test]
async fn encrypted_store_retains_mail_search_and_settings_across_worker_reopen() {
    use crate::{model::*, store::Store};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.sqlite");
    let key = std::sync::Arc::new(Key::generate().unwrap());
    let store = Store::open_encrypted(&path, key.clone()).unwrap();
    let raw = b"From: Fictional Sender <sender@example.test>\r\nTo: reader@example.test\r\nSubject: Encrypted fixture\r\nContent-Type: text/plain\r\n\r\nUnique private search phrase.".to_vec();
    let mail = parse_mail("fixture", "1", "INBOX", raw.clone(), true, false).unwrap();
    let id = mail.summary.id.clone();
    store.upsert(vec![mail]).await.unwrap();
    store
        .update_preferences(|p| p.reader_font_size = 22)
        .await
        .unwrap();
    let page = store
        .query(MailQuery {
            folder: "INBOX".into(),
            search: "private".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(store.raw_message(id.clone()).await.unwrap(), raw);
    drop(store);
    // Opening a second worker is safe even while the first finishes dropping;
    // its accepted writes were already acknowledged before this point.
    let reopened = Store::open_encrypted(&path, key).unwrap();
    assert_eq!(
        reopened
            .workspace()
            .await
            .unwrap()
            .preferences
            .reader_font_size,
        22
    );
    assert_eq!(reopened.raw_message(id).await.unwrap(), raw);
    assert!(reopened.connection_key().is_some());
    assert!(Store::open(&path).is_err());
}

#[test]
fn cache_crypto_initialization_uses_the_tls_exit_policy() {
    const MODE: &str = "SHEP_CACHE_CRYPTO_EXIT_FIXTURE";
    const MARKER: &str = "SHEP_CACHE_CRYPTO_EXIT_MARKER";
    if let Some(mode) = std::env::var_os(MODE) {
        if mode == "explicit-control" {
            // SAFETY: this fresh child selects ordinary initialization before
            // registering the explicit-cleanup observer. init_crypto(0) alone
            // is a no-op in OpenSSL 3.6 and would not enable cleanup callbacks.
            assert_eq!(
                unsafe { OPENSSL_init_crypto(0x0000_0002, std::ptr::null()) },
                1
            );
        } else {
            let directory = tempfile::tempdir().unwrap();
            let key = std::sync::Arc::new(Key::generate().unwrap());
            let store =
                crate::store::Store::open_encrypted(directory.path().join("cache.sqlite"), key)
                    .unwrap();
            drop(store);
        }
        extern "C" fn marker() {
            if let Some(path) = std::env::var_os("SHEP_CACHE_CRYPTO_EXIT_MARKER") {
                let _ = std::fs::write(path, b"OpenSSL global cleanup ran");
            }
        }
        unsafe extern "C" {
            fn OPENSSL_atexit(handler: extern "C" fn()) -> std::ffi::c_int;
            fn OPENSSL_cleanup();
            fn OPENSSL_init_crypto(
                options: u64,
                settings: *const std::ffi::c_void,
            ) -> std::ffi::c_int;
        }
        // SAFETY: the no-capture callback has process lifetime and does not
        // unwind. OpenSSL copies its address for process cleanup observation.
        assert_eq!(unsafe { OPENSSL_atexit(marker) }, 1);
        if mode == "explicit-control" {
            // SAFETY: this fresh child has no cache connection or active TLS
            // user. Explicit cleanup is the callback observer's positive
            // control, even when curl's pre-main constructor disabled atexit.
            unsafe { OPENSSL_cleanup() };
        }
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    for mode in ["explicit-control", "owned-cache"] {
        let path = directory.path().join(mode);
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cache_cipher::tests::cache_crypto_initialization_uses_the_tls_exit_policy",
                "--nocapture",
            ])
            .env(MODE, mode)
            .env(MARKER, &path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{mode}: {}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(path.exists(), mode == "explicit-control", "{mode}");
    }
}

#[test]
fn admitted_cache_reads_remain_valid_during_process_exit() {
    const MODE: &str = "SHEP_CACHE_EXIT_READ_FIXTURE";
    const MARKER: &str = "SHEP_CACHE_EXIT_READ_MARKER";
    static WORKER: std::sync::OnceLock<
        std::sync::mpsc::SyncSender<std::sync::mpsc::SyncSender<()>>,
    > = std::sync::OnceLock::new();
    if std::env::var_os(MODE).is_some() {
        extern "C" fn finish_worker() {
            let (acknowledge, completed) = std::sync::mpsc::sync_channel(1);
            if let Some(worker) = WORKER.get() {
                let _ = worker.send(acknowledge);
                let _ = completed.recv_timeout(std::time::Duration::from_secs(10));
            }
        }
        unsafe extern "C" {
            fn atexit(callback: extern "C" fn()) -> std::ffi::c_int;
        }
        // SAFETY: this process-lifetime callback never unwinds. Register before
        // the first SQLite open so automatic library cleanup would precede it.
        assert_eq!(unsafe { atexit(finish_worker) }, 0);
        let (commands, input) = std::sync::mpsc::sync_channel(1);
        WORKER.set(commands).unwrap();
        let (ready, prepared) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let directory = tempfile::tempdir().unwrap();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let key = std::sync::Arc::new(Key::generate().unwrap());
            let store =
                crate::store::Store::open_encrypted(directory.path().join("cache.sqlite"), key)
                    .unwrap();
            runtime
                .block_on(store.put("exit-fixture", "preserved".to_string()))
                .unwrap();
            runtime
                .block_on(store.run(|connection| {
                    connection.execute_batch(
                        "PRAGMA main.wal_checkpoint(TRUNCATE); PRAGMA shrink_memory;",
                    )?;
                    Ok(())
                }))
                .unwrap();
            ready.send(()).unwrap();
            let acknowledge = input.recv().unwrap();
            let value = runtime
                .block_on(store.get::<String>("exit-fixture"))
                .unwrap();
            assert_eq!(value, "preserved");
            std::fs::write(std::env::var_os(MARKER).unwrap(), value).unwrap();
            acknowledge.send(()).unwrap();
        });
        prepared.recv().unwrap();
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("completed");
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cache_cipher::tests::admitted_cache_reads_remain_valid_during_process_exit",
            "--nocapture",
        ])
        .env(MODE, "1")
        .env(MARKER, &path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}: {}\n{}",
        result.status,
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), "preserved");
}

#[test]
fn explicit_sqlite_shutdown_still_releases_and_reinitializes_the_cipher() {
    const MODE: &str = "SHEP_CACHE_EXPLICIT_SHUTDOWN_FIXTURE";
    if std::env::var_os(MODE).is_some() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cipher.sqlite");
        let key = Key::generate().unwrap();
        let connection = key.open(&path, writable()).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE fixture(value TEXT); INSERT INTO fixture VALUES('preserved');",
            )
            .unwrap();
        drop(connection);
        // SAFETY: this fresh child has no other SQLite connection or worker.
        assert_eq!(
            unsafe { rusqlite::ffi::sqlite3_shutdown() },
            rusqlite::ffi::SQLITE_OK
        );
        let reopened = key.open(&path, writable()).unwrap();
        assert_eq!(
            reopened
                .query_row("SELECT value FROM fixture", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "preserved"
        );
        drop(reopened);
        // SAFETY: the only reopened connection was dropped above.
        assert_eq!(
            unsafe { rusqlite::ffi::sqlite3_shutdown() },
            rusqlite::ffi::SQLITE_OK
        );
        return;
    }
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cache_cipher::tests::explicit_sqlite_shutdown_still_releases_and_reinitializes_the_cipher", "--nocapture"])
        .env(MODE,"1").output().unwrap();
    assert!(
        result.status.success(),
        "{}: {}\n{}",
        result.status,
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
