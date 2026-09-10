//! Mail protocols shared with the hosted browser backend, plus the desktop-only
//! store-coupled IMAP move adapter.
#[cfg(test)]
mod folder_tests;
pub mod moves;
pub use shep_mail_core::providers::mail::*;
