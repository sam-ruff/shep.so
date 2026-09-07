pub mod backup;
pub mod bulk;
pub mod compose;
pub mod desktop_badge;
pub mod email_content;
pub mod engine;
pub mod folder_actions;
pub mod folders;
pub mod fuzzy;
pub mod html_render;
pub mod mail_actions;
pub mod message_find;
pub mod model;
pub mod notifications;
pub mod outgoing;
pub mod printing;
pub mod providers;
pub mod remote_images;
pub mod replies;
pub mod shortcuts;
pub mod store;
pub mod ui;

#[cfg(feature = "test-support")]
#[path = "../tests/support/fixtures.rs"]
pub mod test_support;

#[cfg(any(test, feature = "test-support"))]
#[path = "../tests/support/complex_html.rs"]
pub(crate) mod complex_html_fixture;
