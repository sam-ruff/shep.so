//! `mailto:` links opened as unsent drafts by every client.

/// Longest link accepted from a launcher or a message.
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

/// Why a link cannot open a draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejected {
    TooLong,
    NotMailto,
    InvalidEncoding,
}

impl Rejected {
    /// Stable name used by shared fixtures.
    pub fn kind(self) -> &'static str {
        match self {
            Self::TooLong => "too-long",
            Self::NotMailto => "not-mailto",
            Self::InvalidEncoding => "invalid-encoding",
        }
    }
}

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLong => "This email link is too long to open safely.",
            Self::NotMailto => "This is not an email link.",
            Self::InvalidEncoding => {
                "This email link contains invalid encoded text, so Shep did not open it."
            }
        })
    }
}

impl std::error::Error for Rejected {}

impl Mailto {
    /// Every draft field a launcher link names. Repeated To/Cc/Bcc values are
    /// combined; the first Subject and body win.
    pub fn parse(link: &str) -> Result<Self, Rejected> {
        let url = mailto_url(link)?;
        let mut to = vec![decode_field(url.path())?];
        let mut cc = Vec::new();
        let mut bcc = Vec::new();
        let mut subject = None;
        let mut body = None;
        for pair in url.query().unwrap_or_default().split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            match key.to_ascii_lowercase().as_str() {
                "to" => to.push(decode_field(value)?),
                "cc" => cc.push(decode_field(value)?),
                "bcc" => bcc.push(decode_field(value)?),
                "subject" if subject.is_none() => subject = Some(decode_field(value)?),
                "body" if body.is_none() => body = Some(decode_body(value)?),
                _ => {}
            }
        }
        Ok(Self {
            to: join(to),
            cc: join(cc),
            bcc: join(bcc),
            subject: subject.unwrap_or_default(),
            body: body.unwrap_or_default(),
        })
    }

    /// Only the link's own address. A link inside received mail cannot add
    /// hidden recipients, a subject, a body or other headers.
    pub fn address(link: &str) -> Result<Self, Rejected> {
        let url = mailto_url(link)?;
        Ok(Self {
            to: decode_field(url.path())?,
            ..Self::default()
        })
    }
}

fn mailto_url(link: &str) -> Result<url::Url, Rejected> {
    if link.len() > MAX_LEN {
        return Err(Rejected::TooLong);
    }
    let url = url::Url::parse(link).map_err(|_| Rejected::NotMailto)?;
    if url.scheme() != "mailto" {
        return Err(Rejected::NotMailto);
    }
    Ok(url)
}

/// Percent escapes must be complete and decode to UTF-8; nothing is guessed.
fn percent_decode(value: &str) -> Result<String, Rejected> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while let Some(&byte) = bytes.get(index) {
        if byte != b'%' {
            decoded.push(byte);
            index += 1;
            continue;
        }
        let (Some(&high), Some(&low)) = (bytes.get(index + 1), bytes.get(index + 2)) else {
            return Err(Rejected::InvalidEncoding);
        };
        let (Some(high), Some(low)) = (hex(high), hex(low)) else {
            return Err(Rejected::InvalidEncoding);
        };
        decoded.push((high << 4) | low);
        index += 3;
    }
    String::from_utf8(decoded).map_err(|_| Rejected::InvalidEncoding)
}

fn hex(digit: u8) -> Option<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

/// Control characters would let a link smuggle extra header lines.
fn decode_field(value: &str) -> Result<String, Rejected> {
    Ok(percent_decode(value)?
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned())
}

/// Bodies keep their line breaks and tabs; other control characters go.
fn decode_body(value: &str) -> Result<String, Rejected> {
    Ok(percent_decode(value)?
        .replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect())
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
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Cases {
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        name: String,
        link: String,
        mode: String,
        expected: Option<Expected>,
        rejected: Option<String>,
    }

    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Expected {
        to: String,
        cc: String,
        bcc: String,
        subject: String,
        body: String,
    }

    #[test]
    fn shared_cases_agree() {
        let cases: Cases = serde_json::from_str(include_str!("../../mailto-cases.json"))
            .expect("shared mailto fixtures");
        assert!(!cases.cases.is_empty());
        for case in cases.cases {
            let parsed = match case.mode.as_str() {
                "activation" => Mailto::parse(&case.link),
                "message" => Mailto::address(&case.link),
                other => panic!("{}: unknown mode {other}", case.name),
            };
            match (case.expected, case.rejected) {
                (Some(expected), None) => assert_eq!(
                    parsed,
                    Ok(Mailto {
                        to: expected.to,
                        cc: expected.cc,
                        bcc: expected.bcc,
                        subject: expected.subject,
                        body: expected.body,
                    }),
                    "{}",
                    case.name
                ),
                (None, Some(kind)) => assert_eq!(
                    parsed.map_err(Rejected::kind),
                    Err(kind.as_str()),
                    "{}",
                    case.name
                ),
                _ => panic!("{}: needs one expected result", case.name),
            }
        }
    }

    #[test]
    fn length_bound_counts_the_whole_link() {
        let fits = format!("mailto:{}@example.test", "a".repeat(MAX_LEN - 20));
        assert_eq!(fits.len(), MAX_LEN);
        assert!(Mailto::parse(&fits).is_ok());
        let long = format!("mailto:{}@example.test", "a".repeat(MAX_LEN - 19));
        assert_eq!(Mailto::parse(&long), Err(Rejected::TooLong));
        assert_eq!(Mailto::address(&long), Err(Rejected::TooLong));
    }

    #[test]
    fn ignored_fields_are_not_decoded() {
        let parsed = Mailto::parse("mailto:a@example.test?attachment=%FF&subject=Hi&subject=%FF")
            .expect("unused fields are ignored");
        assert_eq!(parsed.subject, "Hi");
        let parsed = Mailto::address("mailto:a@example.test?subject=%FF").expect("address only");
        assert_eq!(parsed.to, "a@example.test");
    }

    #[test]
    fn rejections_have_fixed_public_text() {
        for rejected in [
            Rejected::TooLong,
            Rejected::NotMailto,
            Rejected::InvalidEncoding,
        ] {
            let text = rejected.to_string();
            assert!(text.ends_with('.'), "{text}");
            assert!(!text.contains("mailto:"), "{text}");
        }
    }
}
