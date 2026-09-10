//! Shared desktop/server mail behavior plus the desktop-only durable move journal and runner.
pub mod journal;
pub mod runner;
use crate::model::Mail;
pub use shep_mail_core::mail_actions::*;
