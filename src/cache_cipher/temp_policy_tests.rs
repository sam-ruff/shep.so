use super::*;
use rusqlite::{
    ffi,
    hooks::{AuthContext, Authorization},
};
use std::{
    ffi::CString,
    sync::atomic::{AtomicUsize, Ordering},
};

fn temp_store(c: &Connection) -> i64 {
    c.query_row("PRAGMA temp_store", [], |r| r.get(0)).unwrap()
}

fn assert_keyed_policy(c: &Connection) {
    assert_eq!(temp_store(c), 2);
    for value in ["FILE", "DEFAULT", "0", "1", "unknown"] {
        let error = c.pragma_update(None, "temp_store", value).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("encrypted databases require memory"),
            "{error}"
        );
        assert_eq!(temp_store(c), 2);
    }
    for value in ["MEMORY", "memory", "2"] {
        c.pragma_update(None, "temp_store", value).unwrap();
    }
}

#[test]
fn keyed_temp_policy_survives_authorizer_replacement_and_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keyed.sqlite");
    let key = Key::generate().unwrap();
    let c = key.open(&path, OpenFlags::default()).unwrap();
    c.execute_batch("CREATE TABLE secrets(value TEXT); INSERT INTO secrets VALUES('fictional');")
        .unwrap();
    assert_keyed_policy(&c);
    c.authorizer(Some(|_: AuthContext<'_>| Authorization::Allow))
        .unwrap();
    assert_keyed_policy(&c);
    c.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
        .unwrap();
    assert_keyed_policy(&c);
    c.execute_batch("ATTACH DATABASE ':memory:' AS extra KEY ''; DETACH DATABASE extra;")
        .unwrap();
    assert_keyed_policy(&c);
    drop(c);
    let c = key.open(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_keyed_policy(&c);
    assert_eq!(
        c.query_row("SELECT value FROM secrets", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "fictional"
    );
}

#[tokio::test]
async fn plaintext_store_and_independent_opens_keep_file_temp_with_encrypted_scratch() {
    let dir = tempfile::tempdir().unwrap();
    let direct = Connection::open(dir.path().join("direct.sqlite")).unwrap();
    assert_eq!(temp_store(&direct), 1);
    let store = crate::store::Store::open(dir.path().join("mail.sqlite")).unwrap();
    store
        .run(|c| {
            assert_eq!(temp_store(c), 1);
            let path: String = c.query_row(
                "SELECT file FROM pragma_database_list WHERE name='scratch'",
                [],
                |r| r.get(0),
            )?;
            let plain = Connection::open(path)?;
            assert!(
                plain
                    .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                        .get::<_, i64>(0))
                    .is_err()
            );
            Ok(())
        })
        .await
        .unwrap();
    let c = super::open(
        None,
        &dir.path().join("independent.sqlite"),
        OpenFlags::default(),
    )
    .unwrap();
    assert_eq!(temp_store(&c), 1);
    let key = Key::generate().unwrap();
    let c = super::open(
        Some(&key),
        &dir.path().join("independent-keyed.sqlite"),
        OpenFlags::default(),
    )
    .unwrap();
    assert_keyed_policy(&c);
}

// A fixture VFS delegates to SQLite's real platform VFS and observes actual
// temporary-file opens. Each registration owns its counters; parallel tests
// cannot change the observation or the application's default VFS.
#[repr(C)]
struct ObservedVfs {
    vfs: ffi::sqlite3_vfs,
    original: *mut ffi::sqlite3_vfs,
    temporary_opens: AtomicUsize,
    name: CString,
}

impl ObservedVfs {
    fn new() -> Box<Self> {
        // SAFETY: SQLite owns its default VFS for the process lifetime. This
        // copy retains pAppData for inherited callbacks; only xOpen delegates.
        let original = unsafe { ffi::sqlite3_vfs_find(std::ptr::null()) };
        assert!(!original.is_null());
        let name = CString::new(format!("shep-temp-policy-{}", uuid::Uuid::new_v4())).unwrap();
        let mut observed = Box::new(Self {
            vfs: unsafe { std::ptr::read(original) },
            original,
            temporary_opens: AtomicUsize::new(0),
            name,
        });
        observed.vfs.zName = observed.name.as_ptr();
        observed.vfs.pNext = std::ptr::null_mut();
        observed.vfs.xOpen = Some(Self::open);
        // SAFETY: the boxed registration remains stable until its Drop, after
        // every connection using this test-local VFS has closed.
        assert_eq!(
            unsafe { ffi::sqlite3_vfs_register(&mut observed.vfs, 0) },
            ffi::SQLITE_OK
        );
        observed
    }

    unsafe extern "C" fn open(
        vfs: *mut ffi::sqlite3_vfs,
        name: *const std::ffi::c_char,
        file: *mut ffi::sqlite3_file,
        flags: i32,
        output: *mut i32,
    ) -> i32 {
        // SAFETY: repr(C) places vfs first; SQLite passes our live registered
        // pointer and the original VFS receives its own pointer/context.
        let observed = unsafe { &*vfs.cast::<Self>() };
        let original_open = unsafe { (*observed.original).xOpen.unwrap() };
        let rc = unsafe { original_open(observed.original, name, file, flags, output) };
        if rc == ffi::SQLITE_OK
            && flags
                & (ffi::SQLITE_OPEN_TEMP_DB
                    | ffi::SQLITE_OPEN_TEMP_JOURNAL
                    | ffi::SQLITE_OPEN_SUBJOURNAL
                    | ffi::SQLITE_OPEN_TRANSIENT_DB)
                != 0
        {
            observed.temporary_opens.fetch_add(1, Ordering::Relaxed);
        }
        rc
    }

    fn connect(&self, path: &Path) -> Connection {
        Connection::open_with_flags_and_vfs(path, OpenFlags::default(), self.name.to_str().unwrap())
            .unwrap()
    }
}

impl Drop for ObservedVfs {
    fn drop(&mut self) {
        // SAFETY: test scopes close all connections before this registration.
        assert_eq!(
            unsafe { ffi::sqlite3_vfs_unregister(&mut self.vfs) },
            ffi::SQLITE_OK
        );
    }
}

fn exercise_temporary_data(c: &Connection) {
    c.execute_batch(
        "PRAGMA temp.cache_size=1;
        CREATE TEMP TABLE private_temp(value BLOB);
        WITH RECURSIVE sequence(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM sequence WHERE i<128)
        INSERT INTO private_temp SELECT randomblob(32768) FROM sequence;",
    )
    .unwrap();
    let mut ordered = c
        .prepare("SELECT value FROM private_temp ORDER BY value")
        .unwrap();
    let mut rows = ordered.query([]).unwrap();
    let mut count = 0;
    while rows.next().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 128);
}

#[test]
fn actual_temp_file_spill_is_plain_only_including_c_key_and_attachment_paths() {
    let dir = tempfile::tempdir().unwrap();
    for mode in ["plain", "plain-with-keyed-scratch", "keyed", "c-key-only"] {
        let observed = ObservedVfs::new();
        let c = observed.connect(&dir.path().join(format!("{mode}.sqlite")));
        assert_eq!(temp_store(&c), 1);
        let key = Key::generate().unwrap();
        match mode {
            "keyed" => key.initialize(&c).unwrap(),
            "c-key-only" => key.apply(&c, c"main").unwrap(),
            "plain-with-keyed-scratch" => {
                c.execute(
                    "ATTACH DATABASE ? AS scratch KEY ''",
                    [dir.path().join("scratch.sqlite").to_str().unwrap()],
                )
                .unwrap();
                key.apply(&c, c"scratch").unwrap();
                c.execute_batch("CREATE TABLE scratch.private(value TEXT); INSERT INTO scratch.private VALUES('fictional');").unwrap();
                assert_eq!(temp_store(&c), 1);
            }
            _ => {}
        }
        c.authorizer(Some(|_: AuthContext<'_>| Authorization::Allow))
            .unwrap();
        c.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)
            .unwrap();
        exercise_temporary_data(&c);
        let opens = observed.temporary_opens.load(Ordering::Relaxed);
        if mode.starts_with("plain") {
            assert!(
                opens > 0,
                "positive control did not exercise disk temp: {mode}"
            );
            if mode == "plain-with-keyed-scratch" {
                c.execute_batch("DETACH DATABASE scratch").unwrap();
                assert_eq!(temp_store(&c), 1);
            }
        } else {
            assert_eq!(
                opens, 0,
                "keyed content opened a plaintext temporary file: {mode}"
            );
        }
        drop(c);
    }
}
