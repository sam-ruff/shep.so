//! `mailto:` links the desktop asks Shep to open.
use percent_encoding::percent_decode_str;
use std::ffi::OsString;

/// Longest link accepted from a launcher.
pub const MAX_LEN: usize = 8 * 1024;

/// The visible fields a link may prefill. Hidden recipients, other headers,
/// bodies and attachments are ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mailto {
    pub to: String,
    pub cc: String,
    pub subject: String,
}

impl Mailto {
    pub fn parse(link: &str) -> Option<Self> {
        if link.len() > MAX_LEN {
            return None;
        }
        let url = url::Url::parse(link).ok()?;
        if url.scheme() != "mailto" {
            return None;
        }
        let mut to = vec![decode(url.path())];
        let mut cc = Vec::new();
        let mut subject = None;
        for pair in url.query().unwrap_or_default().split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            match key.to_ascii_lowercase().as_str() {
                "to" => to.push(decode(value)),
                "cc" => cc.push(decode(value)),
                "subject" if subject.is_none() => subject = Some(decode(value)),
                _ => {}
            }
        }
        Some(Self {
            to: join(to),
            cc: join(cc),
            subject: subject.unwrap_or_default(),
        })
    }
}

/// The first valid `mailto:` link among launch arguments.
pub fn from_args(args: impl IntoIterator<Item = OsString>) -> Option<String> {
    args.into_iter()
        .filter_map(|arg| arg.into_string().ok())
        .find(|arg| Mailto::parse(arg).is_some())
}

/// Control characters would let a link smuggle extra header lines.
fn decode(value: &str) -> String {
    percent_decode_str(value)
        .decode_utf8_lossy()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

fn join(values: Vec<String>) -> String {
    values
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefills_recipients_and_subject_but_not_hidden_fields() {
        let link = "MAILTO:a%40example.test?to=b@example.test&cc=c@example.test\
                    &subject=Hello%20there+friend&bcc=hidden@example.test\
                    &body=text&attachment=/etc/passwd";
        assert_eq!(
            Mailto::parse(link),
            Some(Mailto {
                to: "a@example.test, b@example.test".into(),
                cc: "c@example.test".into(),
                subject: "Hello there+friend".into(),
            })
        );
    }

    #[test]
    fn strips_line_breaks_from_fields() {
        let parsed = Mailto::parse("mailto:a@example.test?subject=Hi%0D%0ABcc:%20x@example.test")
            .expect("valid link");
        assert_eq!(parsed.subject, "Hi  Bcc: x@example.test");
    }

    #[test]
    fn accepts_links_without_an_address() {
        let parsed = Mailto::parse("mailto:?subject=Report").expect("valid link");
        assert_eq!(parsed.to, "");
        assert_eq!(parsed.subject, "Report");
    }

    #[test]
    fn rejects_other_schemes_and_oversized_links() {
        assert_eq!(Mailto::parse("https://example.test"), None);
        assert_eq!(Mailto::parse("not a link"), None);
        let long = format!("mailto:{}@example.test", "a".repeat(MAX_LEN));
        assert_eq!(Mailto::parse(&long), None);
    }

    #[test]
    fn finds_the_link_among_launch_arguments() {
        let args = ["--demo", "https://example.test", "mailto:a@example.test"].map(OsString::from);
        assert_eq!(from_args(args), Some("mailto:a@example.test".into()));
        assert_eq!(from_args(["--demo"].map(OsString::from)), None);
    }
}
