//! Waits shared by unit and integration tests.
use std::time::{Duration, Instant};

/// Bounds a wait for something a test expects to happen. It only turns a hang
/// into a failure, so it allows for a heavily starved CI runner. Never use it
/// for a "must not happen within" window or a performance budget.
pub const HANG: Duration = Duration::from_secs(120);

/// Opens a database, waiting while a store the test dropped still holds it.
/// A dropped store's worker closes its connection in the background, and on a
/// slow disk that close can hold the file locked past the busy timeout.
pub fn reopen<T>(mut open: impl FnMut() -> anyhow::Result<T>) -> T {
    let deadline = Instant::now() + HANG;
    loop {
        match open() {
            Ok(opened) => return opened,
            Err(error) if format!("{error:#}").contains("database is locked") => {
                assert!(Instant::now() < deadline, "still locked: {error:#}");
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("reopening failed: {error:#}"),
        }
    }
}
