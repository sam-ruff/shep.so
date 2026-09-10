# Shep curl initialization patch

Vendored crate: curl 0.4.50, MIT, upstream commit
`0cfd9e3b8b1aa0b8fc2c8d552597555a30a21416` from
[alexcrichton/curl-rust](https://github.com/alexcrichton/curl-rust/tree/0cfd9e3b8b1aa0b8fc2c8d552597555a30a21416).
All upstream source is retained, including its fixtures; they are not invoked as network tests by Shep hooks. Only the initialization cfg in `build.rs` and `src/lib.rs` is changed.

When OpenSSL metadata is present, enable the existing `openssl_sys::init()` call
for every supported OpenSSL version. Upstream only enables it for versions below
1.1.0. The Rust initializer requests `OPENSSL_INIT_NO_ATEXIT` on OpenSSL 1.1.1b+;
Shep bundles OpenSSL 3.6.3. Initialization occurs inside curl's existing `Once`
and pre-main constructor, before `curl_global_init`, without competing constructor
ordering. Schannel and other builds without OpenSSL metadata remain unchanged.
Existing CA-path probing is unchanged; the new modern initializer does not mutate the process environment.

The exact static-curl 0.4.50 / curl-sys 0.4.90+curl-8.21.0 combination initialized
OpenSSL before SQLCipher's first-use policy. A subprocess observer reproduced
OpenSSL global cleanup at process exit; `tests/crypto_lifecycle.rs` requires the
combined build to retain crypto state for surviving owned workers. Ordinary
application shutdown still drains admitted writes. libcurl already deliberately
omits global cleanup because other threads may remain alive. Explicit OpenSSL
cleanup remains available only to an owner that has stopped all crypto users.

See [OpenSSL initialization](https://docs.openssl.org/3.6/man3/OPENSSL_init_crypto/).
Remove this patch only when upstream provides the same process-lifetime guarantee
and the combined crypto/FTPS tests pass. No FTP protocol or certificate-validation
code is changed. This source is distinct from curl-sys's bundled C libcurl source
and its license.
