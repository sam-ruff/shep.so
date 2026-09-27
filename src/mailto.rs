//! `mailto:` links the desktop asks Shep to open.
use percent_encoding::percent_decode_str;
use std::ffi::OsString;

/// Longest link accepted from a launcher.
pub const MAX_LEN: usize = 8 * 1024;

/// The draft fields a link may prefill, all shown before sending. Other
/// headers and attachments are ignored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mailto {
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
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
        let mut bcc = Vec::new();
        let mut subject = None;
        let mut body = None;
        for pair in url.query().unwrap_or_default().split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            match key.to_ascii_lowercase().as_str() {
                "to" => to.push(decode(value)),
                "cc" => cc.push(decode(value)),
                "bcc" => bcc.push(decode(value)),
                "subject" if subject.is_none() => subject = Some(decode(value)),
                "body" if body.is_none() => body = Some(decode_body(value)),
                _ => {}
            }
        }
        Some(Self {
            to: join(to),
            cc: join(cc),
            bcc: join(bcc),
            subject: subject.unwrap_or_default(),
            body: body.unwrap_or_default(),
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

/// Bodies keep their line breaks and tabs; other control characters go.
fn decode_body(value: &str) -> String {
    percent_decode_str(value)
        .decode_utf8_lossy()
        .replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
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
    fn prefills_draft_fields_but_not_other_headers() {
        let link = "MAILTO:a%40example.test?to=b@example.test&cc=c@example.test\
                    &subject=Hello%20there+friend&bcc=hidden@example.test\
                    &body=This%20is%20the%20body.&attachment=/etc/passwd&from=x@example.test";
        assert_eq!(
            Mailto::parse(link),
            Some(Mailto {
                to: "a@example.test, b@example.test".into(),
                cc: "c@example.test".into(),
                bcc: "hidden@example.test".into(),
                subject: "Hello there+friend".into(),
                body: "This is the body.".into(),
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
    fn body_keeps_line_breaks_and_drops_other_control_characters() {
        let parsed = Mailto::parse("mailto:a@example.test?body=First%0D%0ASecond%0A%09Third%00%1B")
            .expect("valid link");
        assert_eq!(parsed.body, "First\nSecond\n\tThird");
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
