//! Shared desktop/server mail behavior.
pub use shep_mail_core::compose::*;

mod forwarding;
pub use forwarding::prepare_forward;
