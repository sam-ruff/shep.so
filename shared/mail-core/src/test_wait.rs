use std::time::Duration;

/// Bounds a wait for something a test expects to happen. It only turns a hang
/// into a failure, so it allows for a heavily starved CI runner. Never use it
/// for a "must not happen within" window or a performance budget.
pub(crate) const HANG: Duration = Duration::from_secs(120);
