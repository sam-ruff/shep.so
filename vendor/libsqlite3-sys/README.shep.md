# SQLCipher build provenance

This is `libsqlite3-sys` 0.38.2 (MIT) from crates.io, with its bundled SQLCipher
amalgamation updated to SQLCipher 4.19.0, SQLite 3.53.4. The original ordinary
SQLite source and Rust bindings are retained. The application enables the
bundled SQLCipher and vendored OpenSSL features; it does not link a system
SQLite library.

Upstream: https://github.com/sqlcipher/sqlcipher/tree/c4b275a47932888216bade83aff2bbc73df0ff85
(tag `v4.19.0`). SQLCipher's license is in `sqlcipher/LICENSE`.

Generated with the upstream build tools:

```sh
./configure --with-tempstore=yes --enable-fts5
make -j4 sqlite3.c
```

Only `sqlcipher/sqlite3.c`, `sqlite3.h` and `sqlite3ext.h` were replaced. The
build uses SQLCipher-supported `SQLITE_TEMP_STORE=2` and the connection-local
`SQLCIPHER_SHEP_TEMP_POLICY` in `shep-temp-policy.patch`. New plaintext connections
default to FILE, preserving legacy disk-backed sorts. `Key::initialize` selects
MEMORY before schema access. A main-pager codec forces effective memory temporary
storage and rejects non-MEMORY temp_store changes, independently of replaceable
application authorizers. Keying only an attached scratch database leaves a
plaintext main's FILE policy intact. Existing SQLCipher codec lookup is reused;
no lifecycle, crypto algorithm or shared coordination is changed by this policy.
The test VFS observes actual temporary-file opens, with a plaintext positive
control and keyed-main/C-key/authorizer-replacement negative controls.

Encrypted application scratch tables must remain bounded or live in encrypted
databases. Existing GROUP BY/DISTINCT summaries, automatic indexes and other
large sort paths still require bounded scratch rewrites before keyed startup is
activated; this policy preserves normal startup behavior while work continues.
The application defines `SQLCIPHER_OMIT_AUTOMATIC_CLEANUP`. The small lifecycle
patch in `shep-lifecycle.patch` skips SQLCipher process-exit/finalizer cleanup
and initializes OpenSSL with `OPENSSL_INIT_NO_ATEXIT` before the first RNG call,
matching Rust OpenSSL's existing policy. Explicit `sqlite3_shutdown()` retains
SQLCipher cleanup and supports reinitialization after all connections close.
The separate `shep-export.patch` enables SQLite's existing `DBFLAG_VacuumInto`
rowid-preservation flag inside `sqlcipher_export`, restoring prior flags on return.
Without it, unindexed tables are renumbered even when exporting explicitly stored
rowids. The conversion regression preserves unindexed rowid 991, indexed mail
rowids 7/101 and external-content FTS. The flag only changes rowid assignment in
SQLite's existing transfer path; it does not change a cipher/page/KDF algorithm. The separate `vendor/curl` initializer
patch sets the same policy before libcurl's pre-main crypto use; keep the combined
`tests/crypto_lifecycle.rs` regression when updating either library.

Without this patch a deterministic child fixture crashes in `sqlite3Codec`
when an owned worker reads an evicted page after automatic global cleanup.
Keep the process-exit read, OpenSSL positive-control and explicit-shutdown
regressions when updating this source. Process-lifetime library allocations
are reclaimed by the OS at exit; ordinary connection keys still clean up as
connections close. This does not replace the application's required save drain.
See [OpenSSL initialization](https://docs.openssl.org/3.6/man3/OPENSSL_init_crypto/).

SQLCipher 4.19 retains the SQLite WAL fixes required by Shep and newer upstream
FTS fixes. Keep cache-worker restart/close, keyed WAL/corruption and transfer
regressions when updating this source. Do not downgrade to the older SQLCipher
amalgamation in the upstream Rust package.

Upstream generated source SHA-256:

- sqlite3.c: `8640c653acadf665cce6331646f60b5b74a4690746f2c4a2d8f688a0570a0c0c`
- sqlite3.h: `8a9d1bff44d75174ca6dea3ea9bac50a6104d86facb566647b8bb839375b7b3a`
- sqlite3ext.h: `ac9645e5c9ff0cf176efdd6e75cb5e98f46295d38e02db5c4d208826a39ab4be`

The current lockfile bundles OpenSSL 3.6.3 via `openssl-src` 300.6.1;
`OpenSSL-LICENSE.txt` accompanies the statically linked release binary. Update
that license alongside an OpenSSL source update.

Lifecycle-only `sqlite3.c` SHA-256: `4fb051a14915b03520e815a23ca3aebb10957af39a36a70da5c9fb9bba29bdd4`.

Lifecycle + temporary-policy `sqlite3.c` SHA-256: `36846a44259e232f5d75e671e2d9b73ff9ea603cb322b28b07250c08f786348a`.

Final lifecycle + temporary-policy + export `sqlite3.c` SHA-256: `77c71d5c5e8b1c0211881eb30892cde7da7b2f5353e656027bd8f1f38e1ea39e`.
