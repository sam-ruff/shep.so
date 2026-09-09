//! Desktop adapters for shared profile history and reviewed local application.
//! OAuth grants, credential slots and the mail cache remain device-owned.
pub mod discovery;
#[cfg(feature = "test-support")]
pub(crate) mod fixture;
pub mod preferences;
