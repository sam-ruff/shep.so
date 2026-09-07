//! Mail protocol and MIME contracts shared by the native desktop and beta service.
//! No UI, SQLite, filesystem cache, keychain or process-global credentials.
pub use shep_mail_content::{attachments, find};
pub mod compose;
pub mod mail_actions;
pub mod model;
pub mod outgoing;
pub mod providers;
pub mod replies;
