//! RFC 2342 NAMESPACE, exchanged on the raw stream because imap-proto has no
//! parser for the response and an undecodable line would end the session.
//!
//! The exchange writes one command and reads byte by byte up to its own tagged
//! completion, so nothing that belongs to a later response is consumed. Any
//! line it cannot place is an error, and the caller then stops using the
//! session rather than risk a desynchronised stream.
use anyhow::Context;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Distinct from async-imap's generated tags, which are never in flight here.
const TAG: &str = "SHEPNS";
/// Bounds the whole exchange, including literals and unsolicited lines.
const LIMIT: usize = 64 * 1024;
const MAX_NAMESPACES: usize = 64;
const MAX_PREFIX: usize = 1024;

/// One personal, other-user or shared namespace descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Namespace {
    /// The wire prefix, such as `INBOX.` or an empty string.
    pub prefix: String,
    pub delimiter: Option<char>,
}

/// Sends NAMESPACE and returns the personal namespaces from its response. Only a
/// matching tagged OK completes; NO, BAD, BYE, disconnection or a missing
/// response are errors.
pub(super) async fn exchange<T: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut T,
) -> anyhow::Result<Vec<Namespace>> {
    stream
        .write_all(format!("{TAG} NAMESPACE\r\n").as_bytes())
        .await?;
    stream.flush().await?;
    let mut budget = LIMIT;
    let mut personal = None;
    loop {
        let response = read_response(stream, &mut budget).await?;
        if let Some(rest) = strip_prefix_ignore_case(&response, b"* NAMESPACE ") {
            anyhow::ensure!(
                personal.is_none(),
                "The server repeated its namespace response."
            );
            personal = Some(parse(rest)?);
            continue;
        }
        if strip_prefix_ignore_case(&response, b"* BYE").is_some() {
            anyhow::bail!("The server disconnected while reporting its namespace.");
        }
        if response.starts_with(b"* ") {
            // Unsolicited status such as EXISTS belongs to no command.
            continue;
        }
        if let Some(status) = response.strip_prefix(format!("{TAG} ").as_bytes()) {
            anyhow::ensure!(
                strip_prefix_ignore_case(status, b"OK").is_some(),
                "The server refused to report its folder namespace."
            );
            return personal.context("The server confirmed NAMESPACE without reporting it.");
        }
        anyhow::bail!("The server sent an unexpected reply while reporting its namespace.");
    }
}

fn strip_prefix_ignore_case<'a>(value: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    (value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix))
        .then(|| &value[prefix.len()..])
}

/// One response without its final CRLF. Literals stay inline, so `{n}\r\n`
/// is followed by exactly `n` bytes, as on the wire.
async fn read_response<T: AsyncRead + Unpin>(
    stream: &mut T,
    budget: &mut usize,
) -> anyhow::Result<Vec<u8>> {
    let mut response = Vec::new();
    loop {
        let byte = match stream.read_u8().await {
            Ok(byte) => byte,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                anyhow::bail!("The server disconnected while reporting its namespace.")
            }
            Err(error) => return Err(error.into()),
        };
        *budget = budget
            .checked_sub(1)
            .context("The namespace response exceeded the supported size.")?;
        response.push(byte);
        if !response.ends_with(b"\r\n") {
            continue;
        }
        let Some(length) = literal_length(&response[..response.len() - 2])? else {
            response.truncate(response.len() - 2);
            return Ok(response);
        };
        anyhow::ensure!(
            length <= *budget,
            "The namespace response exceeded the supported size."
        );
        *budget -= length;
        let start = response.len();
        response.resize(start + length, 0);
        stream
            .read_exact(&mut response[start..])
            .await
            .context("The server disconnected while reporting its namespace.")?;
    }
}

/// The length announced by a trailing `{n}` or `{n+}`, if the line ends with one.
fn literal_length(line: &[u8]) -> anyhow::Result<Option<usize>> {
    let Some(open) = line
        .ends_with(b"}")
        .then(|| line.iter().rposition(|byte| *byte == b'{'))
        .flatten()
    else {
        return Ok(None);
    };
    let digits = &line[open + 1..line.len() - 1];
    let digits = digits.strip_suffix(b"+").unwrap_or(digits);
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Ok(None);
    }
    std::str::from_utf8(digits)?
        .parse()
        .map(Some)
        .context("Invalid literal length in the namespace response.")
}

/// Parses the three namespace lists that follow `* NAMESPACE ` and returns the
/// personal one. The other-user and shared lists are validated, not kept.
pub(super) fn parse(input: &[u8]) -> anyhow::Result<Vec<Namespace>> {
    let mut parser = Parser { input, position: 0 };
    let personal = parser.namespaces()?;
    parser.space()?;
    parser.namespaces()?;
    parser.space()?;
    parser.namespaces()?;
    while parser.peek() == Some(b' ') {
        parser.position += 1;
    }
    anyhow::ensure!(
        parser.peek().is_none(),
        "The server sent a malformed namespace response."
    );
    Ok(personal)
}

struct Parser<'a> {
    input: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn malformed() -> anyhow::Error {
        anyhow::anyhow!("The server sent a malformed namespace response.")
    }
    fn peek(&self) -> Option<u8> {
        self.input.get(self.position).copied()
    }
    fn expect(&mut self, byte: u8) -> anyhow::Result<()> {
        if self.peek() != Some(byte) {
            return Err(Self::malformed());
        }
        self.position += 1;
        Ok(())
    }
    fn space(&mut self) -> anyhow::Result<()> {
        self.expect(b' ')
    }
    fn nil(&mut self) -> bool {
        let end = self.position + 3;
        if self
            .input
            .get(self.position..end)
            .is_some_and(|value| value.eq_ignore_ascii_case(b"NIL"))
        {
            self.position = end;
            return true;
        }
        false
    }
    /// `nil / "(" 1*descriptor ")"`, tolerating spaces between descriptors.
    fn namespaces(&mut self) -> anyhow::Result<Vec<Namespace>> {
        if self.nil() {
            return Ok(Vec::new());
        }
        self.expect(b'(')?;
        let mut namespaces = Vec::new();
        loop {
            match self.peek() {
                Some(b'(') => {
                    anyhow::ensure!(
                        namespaces.len() < MAX_NAMESPACES,
                        "The server reported too many namespaces."
                    );
                    namespaces.push(self.descriptor()?);
                }
                Some(b' ') if !namespaces.is_empty() => self.position += 1,
                Some(b')') if !namespaces.is_empty() => {
                    self.position += 1;
                    return Ok(namespaces);
                }
                _ => return Err(Self::malformed()),
            }
        }
    }
    /// `"(" string SP (quoted-char / nil) *extension ")"`.
    fn descriptor(&mut self) -> anyhow::Result<Namespace> {
        self.expect(b'(')?;
        let prefix = String::from_utf8(self.string()?)
            .map_err(|_| anyhow::anyhow!("The server reported an invalid namespace prefix."))?;
        anyhow::ensure!(
            prefix.len() <= MAX_PREFIX && !prefix.chars().any(char::is_control),
            "The server reported an invalid namespace prefix."
        );
        self.space()?;
        let delimiter = if self.nil() {
            None
        } else {
            let value = String::from_utf8(self.quoted()?)
                .map_err(|_| anyhow::anyhow!("The server reported an invalid folder separator."))?;
            let mut characters = value.chars();
            let delimiter = characters.next();
            anyhow::ensure!(
                delimiter.is_some() && characters.next().is_none(),
                "The server reported an invalid folder separator."
            );
            delimiter
        };
        while self.peek() == Some(b' ') {
            self.extension()?;
        }
        self.expect(b')')?;
        Ok(Namespace { prefix, delimiter })
    }
    /// `SP string SP "(" string *(SP string) ")"`.
    fn extension(&mut self) -> anyhow::Result<()> {
        self.space()?;
        self.string()?;
        self.space()?;
        self.expect(b'(')?;
        self.string()?;
        while self.peek() == Some(b' ') {
            self.space()?;
            self.string()?;
        }
        self.expect(b')')
    }
    fn string(&mut self) -> anyhow::Result<Vec<u8>> {
        match self.peek() {
            Some(b'"') => self.quoted(),
            Some(b'{') => self.literal(),
            _ => Err(Self::malformed()),
        }
    }
    fn quoted(&mut self) -> anyhow::Result<Vec<u8>> {
        self.expect(b'"')?;
        let mut value = Vec::new();
        loop {
            match self.peek().ok_or_else(Self::malformed)? {
                b'"' => {
                    self.position += 1;
                    return Ok(value);
                }
                b'\\' => {
                    self.position += 1;
                    match self.peek() {
                        Some(escaped @ (b'"' | b'\\')) => value.push(escaped),
                        _ => return Err(Self::malformed()),
                    }
                }
                b'\r' | b'\n' | 0 => return Err(Self::malformed()),
                byte => value.push(byte),
            }
            self.position += 1;
        }
    }
    fn literal(&mut self) -> anyhow::Result<Vec<u8>> {
        self.expect(b'{')?;
        let start = self.position;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.position += 1;
        }
        let length: usize = std::str::from_utf8(&self.input[start..self.position])?
            .parse()
            .map_err(|_| Self::malformed())?;
        if self.peek() == Some(b'+') {
            self.position += 1;
        }
        self.expect(b'}')?;
        self.expect(b'\r')?;
        self.expect(b'\n')?;
        let end = self
            .position
            .checked_add(length)
            .filter(|end| *end <= self.input.len())
            .ok_or_else(Self::malformed)?;
        let value = self.input[self.position..end].to_vec();
        self.position = end;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    fn namespace(prefix: &str, delimiter: Option<char>) -> Namespace {
        Namespace {
            prefix: prefix.into(),
            delimiter,
        }
    }

    #[test]
    fn parses_the_rfc_2342_examples_and_returns_the_personal_namespaces() {
        for (input, expected) in [
            (r##"(("" "/")) NIL NIL"##, vec![namespace("", Some('/'))]),
            (r##"NIL NIL (("" "."))"##, vec![]),
            (
                r##"(("" "/")) NIL (("Public Folders/" "/"))"##,
                vec![namespace("", Some('/'))],
            ),
            (
                r##"(("" "/")) (("~" "/")) (("#shared/" "/")("#public/" "/")("#ftp/" "/")("#news." "."))"##,
                vec![namespace("", Some('/'))],
            ),
            (
                r##"(("INBOX." ".")) NIL NIL"##,
                vec![namespace("INBOX.", Some('.'))],
            ),
            (
                r##"(("" "/")("#mh/" "/" "X-PARAM" ("FLAG1" "FLAG2"))) NIL NIL"##,
                vec![namespace("", Some('/')), namespace("#mh/", Some('/'))],
            ),
            (r##"(("" NIL)) nil nil"##, vec![namespace("", None)]),
            (
                r##"(("INBOX." ".") ("Other." ".")) NIL NIL  "##,
                vec![
                    namespace("INBOX.", Some('.')),
                    namespace("Other.", Some('.')),
                ],
            ),
            (
                r##"(("Quote\"d\\" "\\")) NIL NIL"##,
                vec![namespace("Quote\"d\\", Some('\\'))],
            ),
            (
                "(({6}\r\nINBOX. \".\")) NIL NIL",
                vec![namespace("INBOX.", Some('.'))],
            ),
            (
                "((\"Dossiers/\" \"/\" {7+}\r\nX-PARAM ({3}\r\nabc))) NIL NIL",
                vec![namespace("Dossiers/", Some('/'))],
            ),
            (
                r##"(("日本語/" "/")) NIL NIL"##,
                vec![namespace("日本語/", Some('/'))],
            ),
        ] {
            assert_eq!(parse(input.as_bytes()).expect(input), expected, "{input}");
        }
    }

    #[test]
    fn rejects_malformed_or_unbounded_responses() {
        let long = format!(r##"(("{}" "/")) NIL NIL"##, "a".repeat(MAX_PREFIX + 1));
        let many = format!("({}) NIL NIL", r##"("" "/")"##.repeat(MAX_NAMESPACES + 1));
        for input in [
            "",
            r##"(("" "/")) NIL"##,
            r##"(("" "/"))"##,
            r##"() NIL NIL"##,
            r##"( ("" "/")) NIL NIL"##,
            r##"(("" "/") NIL NIL"##,
            r##"(("" "//")) NIL NIL"##,
            r##"(("" "")) NIL NIL"##,
            r##"((NIL "/")) NIL NIL"##,
            r##"(("unterminated "/")) NIL NIL"##,
            r##"(("" "/" "X-PARAM")) NIL NIL"##,
            r##"(("" "/" "X-PARAM" ())) NIL NIL"##,
            r##"(("bad\q" "/")) NIL NIL"##,
            "((\"line\rbreak\" \"/\")) NIL NIL",
            "((\"tab\tprefix\" \"/\")) NIL NIL",
            "(({9}\r\nshort \"/\")) NIL NIL",
            r##"(("" "/")) NIL NIL trailing"##,
            r##"(("" "/"))NIL NIL"##,
            long.as_str(),
            many.as_str(),
        ] {
            assert!(parse(input.as_bytes()).is_err(), "{input:?}");
        }
        assert!(parse(b"((\"\xff\" \"/\")) NIL NIL").is_err());
    }

    /// Runs `exchange` against a raw peer that answers the command with `reply`.
    async fn exchanged(reply: &'static str) -> (anyhow::Result<Vec<Namespace>>, String) {
        let (mut client, server) = tokio::io::duplex(LIMIT * 2);
        let peer = async move {
            let mut server = BufReader::new(server);
            let mut line = String::new();
            server.read_line(&mut line).await.expect("command");
            server
                .get_mut()
                .write_all(reply.as_bytes())
                .await
                .expect("reply");
            // Closing ends any read that expects more than the reply holds.
            drop(server);
            line
        };
        let (result, line) = tokio::join!(exchange(&mut client), peer);
        (result, line)
    }

    #[tokio::test]
    async fn exchange_sends_its_own_tag_and_skips_unsolicited_status() {
        let (result, line) = exchanged(
            "* 3 EXISTS\r\n* NAMESPACE ((\"INBOX.\" \".\")) NIL NIL\r\n* 1 RECENT\r\nSHEPNS OK done\r\n",
        )
        .await;
        assert_eq!(line, "SHEPNS NAMESPACE\r\n");
        assert_eq!(
            result.expect("namespace"),
            vec![namespace("INBOX.", Some('.'))]
        );
        let (result, _) =
            exchanged("* namespace (({6}\r\nINBOX. \".\")) NIL NIL\r\nSHEPNS ok done\r\n").await;
        assert_eq!(
            result.expect("literal prefix"),
            vec![namespace("INBOX.", Some('.'))]
        );
    }

    #[tokio::test]
    async fn exchange_stops_at_its_tagged_completion() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        server
            .write_all(b"* NAMESPACE ((\"\" \"/\")) NIL NIL\r\nSHEPNS OK done\r\n* 4 EXISTS\r\n")
            .await
            .expect("reply");
        assert_eq!(
            exchange(&mut client).await.expect("namespace"),
            vec![namespace("", Some('/'))]
        );
        let mut rest = [0; 12];
        client.read_exact(&mut rest).await.expect("later bytes");
        assert_eq!(&rest, b"* 4 EXISTS\r\n");
    }

    #[tokio::test]
    async fn tagged_refusals_partial_data_and_unexpected_lines_are_errors() {
        for (reply, message) in [
            (
                "SHEPNS NO unavailable\r\n",
                "refused to report its folder namespace",
            ),
            (
                "* NAMESPACE ((\"\" \"/\")) NIL NIL\r\nSHEPNS NO later refusal\r\n",
                "refused to report its folder namespace",
            ),
            (
                "SHEPNS BAD unknown command\r\n",
                "refused to report its folder namespace",
            ),
            ("SHEPNS OK done\r\n", "without reporting it"),
            (
                "* NAMESPACE ((\"\" \"/\")) NIL NIL\r\n",
                "disconnected while reporting",
            ),
            ("* NAMESPACE ((\"\" \"/", "disconnected while reporting"),
            (
                "* NAMESPACE (({40}\r\nshort",
                "disconnected while reporting",
            ),
            ("* BYE shutting down\r\n", "disconnected while reporting"),
            (
                "* NAMESPACE ((\"\" \"/\")) NIL NIL\r\n* NAMESPACE NIL NIL NIL\r\nSHEPNS OK\r\n",
                "repeated its namespace",
            ),
            ("A0001 OK other command\r\n", "unexpected reply"),
            ("+ go ahead\r\n", "unexpected reply"),
            (
                "* NAMESPACE ((\"\" \"/\")) NIL\r\nSHEPNS OK done\r\n",
                "malformed namespace",
            ),
            (
                "* NAMESPACE (({999999999}\r\n",
                "exceeded the supported size",
            ),
        ] {
            let (result, _) = exchanged(reply).await;
            let error = result.expect_err(reply);
            assert!(
                error.to_string().contains(message),
                "{error:#} for {reply:?}"
            );
        }
    }

    #[tokio::test]
    async fn an_oversized_reply_is_bounded() {
        let (mut client, mut server) = tokio::io::duplex(LIMIT * 2);
        let flood = [b'x'; LIMIT + 1];
        let writer = async move {
            let _ = server.write_all(&flood).await;
            server
        };
        let (result, _server) = tokio::join!(exchange(&mut client), writer);
        assert!(
            result
                .expect_err("oversized")
                .to_string()
                .contains("exceeded the supported size")
        );
    }
}
