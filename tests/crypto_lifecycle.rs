//! The real libcurl constructor must select the same process-lifetime crypto
//! policy as SQLCipher before any SQLite connection is opened.
#![cfg(unix)]

#[test]
fn curl_and_cache_keep_crypto_alive_until_process_exit() {
    const CHILD: &str = "SHEP_CURL_CACHE_EXIT_FIXTURE";
    const MARKER: &str = "SHEP_CURL_CACHE_EXIT_MARKER";
    if std::env::var_os(CHILD).is_some() {
        // This also retains curl's pre-main constructor in the linked binary.
        curl::init();
        assert!(
            curl::Version::get()
                .ssl_version()
                .unwrap()
                .starts_with("OpenSSL/")
        );
        extern "C" fn marker() {
            if let Some(path) = std::env::var_os("SHEP_CURL_CACHE_EXIT_MARKER") {
                let _ = std::fs::write(path, b"OpenSSL global cleanup ran");
            }
        }
        unsafe extern "C" {
            fn OPENSSL_atexit(handler: extern "C" fn()) -> std::ffi::c_int;
        }
        // SAFETY: this callback has process lifetime and never unwinds. It
        // observes whether curl's first initialization enabled global cleanup.
        assert_eq!(unsafe { OPENSSL_atexit(marker) }, 1);
        let directory = tempfile::tempdir().unwrap();
        let key = std::sync::Arc::new(shep::cache_cipher::Key::generate().unwrap());
        let store =
            shep::store::Store::open_encrypted(directory.path().join("cache.sqlite"), key).unwrap();
        drop(store);
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("cleanup");
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "curl_and_cache_keep_crypto_alive_until_process_exit",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env(MARKER, &marker)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}: {}\n{}",
        result.status,
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        !marker.exists(),
        "libcurl initialized OpenSSL with automatic cleanup before SQLCipher could select the process-lifetime policy"
    );
}
