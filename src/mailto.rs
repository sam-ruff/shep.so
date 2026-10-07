//! `mailto:` links the desktop asks Shep to open. Parsing is shared with the
//! mobile client in `shep_mail_content::mailto`.
pub use shep_mail_content::mailto::{MAX_LEN, Mailto};
use std::ffi::OsString;

/// The first valid `mailto:` link among launch arguments.
pub fn from_args(args: impl IntoIterator<Item = OsString>) -> Option<String> {
    args.into_iter()
        .filter_map(|arg| arg.into_string().ok())
        .find(|arg| Mailto::parse(arg).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_link_among_launch_arguments() {
        let args = ["--demo", "https://example.test", "mailto:a@example.test"].map(OsString::from);
        assert_eq!(from_args(args), Some("mailto:a@example.test".into()));
        assert_eq!(from_args(["--demo"].map(OsString::from)), None);
    }

    #[test]
    fn skips_links_with_invalid_encoding() {
        let args =
            ["mailto:a@example.test?subject=%FF", "mailto:b@example.test"].map(OsString::from);
        assert_eq!(from_args(args), Some("mailto:b@example.test".into()));
    }
}
