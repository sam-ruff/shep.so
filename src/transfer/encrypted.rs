//! Convert a keyed source to the explicit portable SQLite export. Its read-only
//! main connection pins one generation; only the attached private output writes.
use super::*;

pub(super) fn copy(
    source: &Path,
    key: &crate::cache_cipher::Key,
    destination: &Path,
    cancel: &mut watch::Receiver<bool>,
    progress: &mut impl FnMut(Progress, &mut watch::Receiver<bool>),
) -> anyhow::Result<Option<Progress>> {
    let mut uri = url::Url::from_file_path(source)
        .map_err(|_| anyhow::anyhow!("The export source needs an absolute path"))?;
    uri.query_pairs_mut().append_pair("mode", "ro");
    // The URI makes main read-only. The connection may separately attach the
    // already-created output writable; main is never opened for cache writes.
    let input = key.open(
        Path::new(uri.as_str()),
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    anyhow::ensure!(
        input.is_readonly(rusqlite::MAIN_DB)?,
        "The export source must remain read-only"
    );
    input.busy_timeout(Duration::from_millis(100))?;
    input.execute_batch("PRAGMA cache_size=-2048;")?;
    let mut target = url::Url::from_file_path(destination)
        .map_err(|_| anyhow::anyhow!("The export needs an absolute output path"))?;
    target.query_pairs_mut().append_pair("mode", "rw");
    input.execute("ATTACH DATABASE ? AS portable KEY ''", [target.as_str()])?;
    input.execute_batch("PRAGMA portable.journal_mode=DELETE; PRAGMA portable.synchronous=FULL; PRAGMA portable.cache_size=-2048;")?;
    let cancellation = cancel.clone();
    input.progress_handler(
        1000,
        Some(move || *cancellation.borrow() || cancellation.has_changed().is_err()),
    )?;
    let snapshot = input.unchecked_transaction()?;
    snapshot.query_row("SELECT count(*) FROM sqlite_schema", [], |_| Ok(()))?;
    let version: i64 = snapshot.query_row("PRAGMA main.user_version", [], |r| r.get(0))?;
    let application: i64 = snapshot.query_row("PRAGMA main.application_id", [], |r| r.get(0))?;
    let pages: u32 = snapshot.query_row("PRAGMA main.page_count", [], |r| r.get(0))?;
    let mut last = Progress {
        phase: Phase::Copying,
        copied_pages: 0,
        total_pages: pages,
    };
    progress(last, cancel);
    if cancelled(cancel) {
        return Ok(None);
    }
    // SQLCipher creates indexes before copying table rows, uses its bounded
    // b-tree transfer path, and installs triggers/views only after copying data.
    let result = snapshot.query_row("SELECT sqlcipher_export('portable','main')", [], |_| Ok(()));
    if cancelled(cancel) {
        return Ok(None);
    }
    result.context("Could not convert the encrypted cache to a portable database")?;
    snapshot.pragma_update(Some("portable"), "user_version", version)?;
    snapshot.pragma_update(Some("portable"), "application_id", application)?;
    snapshot.commit()?;
    input.close().map_err(|(_, error)| error)?;
    if cancelled(cancel) {
        return Ok(None);
    }
    // Verify the actual standalone output without any cache key. User exports
    // intentionally remain portable; implicit cache/import scratch stays keyed.
    let output = Connection::open_with_flags(destination, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let cancellation = cancel.clone();
    output.progress_handler(
        1000,
        Some(move || *cancellation.borrow() || cancellation.has_changed().is_err()),
    )?;
    let integrity = output.query_row("PRAGMA integrity_check(1)", [], |r| r.get::<_, String>(0));
    if cancelled(cancel) {
        return Ok(None);
    }
    anyhow::ensure!(
        integrity? == "ok",
        "The portable copy failed validation. The existing destination was kept."
    );
    output.close().map_err(|(_, error)| error)?;
    last.copied_pages = pages;
    Ok(Some(last))
}
