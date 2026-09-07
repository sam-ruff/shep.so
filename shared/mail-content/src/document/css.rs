//! Parse escaped URLs as CSS tokens; never look for network resources by regex.
use cssparser::{Parser, ParserInput, ToCss, Token};

pub(super) fn rewrite(source: &str, mut image: impl FnMut(&str) -> Option<String>) -> String {
    fn scan<'i>(
        parser: &mut Parser<'i, '_>,
        image: &mut impl FnMut(&str) -> Option<String>,
        depth: u8,
        image_set: bool,
    ) -> String {
        let mut output = String::new();
        while let Ok(token) = parser.next_including_whitespace_and_comments().cloned() {
            match token {
                Token::AtKeyword(ref name)
                    if [
                        "import",
                        "font-face",
                        "namespace",
                        "keyframes",
                        "-webkit-keyframes",
                    ]
                    .iter()
                    .any(|value| name.eq_ignore_ascii_case(value)) =>
                {
                    while let Ok(token) = parser.next() {
                        if matches!(token, Token::Semicolon | Token::CurlyBracketBlock) {
                            break;
                        }
                    }
                }
                Token::UnquotedUrl(value) => output.push_str(&resource(&value, image)),
                Token::QuotedString(value) if image_set => {
                    output.push_str(&resource(&value, image))
                }
                Token::Function(ref name) if name.eq_ignore_ascii_case("url") => {
                    let value: Result<String, cssparser::ParseError<'i, ()>> = parser
                        .parse_nested_block(|p| {
                            let value = p.expect_string()?.to_string();
                            p.expect_exhausted()?;
                            Ok(value)
                        });
                    output.push_str(
                        &value
                            .ok()
                            .map(|value| resource(&value, image))
                            .unwrap_or_else(|| "none".into()),
                    );
                }
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock => {
                    if depth >= 64 {
                        continue;
                    }
                    let set = matches!(&token, Token::Function(name) if name.eq_ignore_ascii_case("image-set") || name.eq_ignore_ascii_case("-webkit-image-set"));
                    let inner: Result<String, cssparser::ParseError<'i, ()>> =
                        parser.parse_nested_block(|p| Ok(scan(p, image, depth + 1, set)));
                    if let Ok(inner) = inner {
                        let _ = token.to_css(&mut output);
                        output.push_str(&inner);
                        output.push(match token {
                            Token::SquareBracketBlock => ']',
                            Token::CurlyBracketBlock => '}',
                            _ => ')',
                        });
                    }
                }
                Token::BadUrl(_) | Token::BadString(_) => {}
                _ => {
                    let _ = token.to_css(&mut output);
                }
            }
        }
        output
    }
    fn resource(value: &str, image: &mut impl FnMut(&str) -> Option<String>) -> String {
        image(value)
            .map(|key| format!("url(\"urn:shep-image:{key}\")"))
            .unwrap_or_else(|| "none".into())
    }
    // CSS string escapes can decode to an HTML raw-text end tag. Keep that tag
    // inert when the rewritten stylesheet is serialized into a <style> node.
    scan(
        &mut Parser::new(&mut ParserInput::new(source)),
        &mut image,
        0,
        false,
    )
    .replace("</", "<\\/")
}
