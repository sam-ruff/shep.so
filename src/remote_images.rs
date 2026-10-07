use crate::model::{Mail, Preferences, RemoteImage};
use shep_mail_core::remote_images::{self as shared, ReqwestTransport, Rules};
pub use shep_mail_core::remote_images::{public_ip, sender_address, sender_domain};

pub fn allowed(preferences: &Preferences, mail: &Mail) -> bool {
    Rules {
        policy: preferences.image_policy,
        messages: &preferences.image_messages,
        senders: &preferences.image_senders,
        domains: &preferences.image_domains,
        contacts: &preferences.contacts,
    }
    .allows(&mail.id, &mail.sender)
}
pub fn extract(parsed: &mailparse::ParsedMail<'_>) -> Vec<RemoteImage> {
    crate::email_content::extract(parsed)
        .ok()
        .and_then(|content| content.html)
        .map(|h| h.remote_images)
        .unwrap_or_default()
}
pub fn extract_html(source: &str) -> Vec<RemoteImage> {
    extract_document(&scraper::Html::parse_document(source))
}
pub(crate) fn extract_document(html: &scraper::Html) -> Vec<RemoteImage> {
    use std::collections::HashSet;
    let base = html
        .select(&scraper::Selector::parse("base[href]").expect("static selector"))
        .next()
        .and_then(|e| url::Url::parse(e.value().attr("href")?).ok());
    let mut images = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |src: &str, alt: &str| {
        let parsed = url::Url::parse(src).or_else(|_| {
            base.as_ref()
                .ok_or(url::ParseError::RelativeUrlWithoutBase)?
                .join(src)
        });
        if let Ok(url) = parsed
            && matches!(url.scheme(), "https" | "http")
            && url.username().is_empty()
            && url.password().is_none()
            && seen.insert(url.to_string())
        {
            images.push(RemoteImage {
                url: url.to_string(),
                alt: alt.chars().take(160).collect(),
            });
        }
    };
    for element in html.select(&scraper::Selector::parse("img[src],body[background],table[background],td[background],th[background],[style],style").expect("static selector")) {
        let value = element.value();
        if value.name() == "img" && let Some(src) = value.attr("src") {
            add(src, value.attr("alt").unwrap_or("Email image"));
        }
        if matches!(value.name(), "body" | "table" | "td" | "th") && let Some(src) = value.attr("background") {
            add(src, "Email background");
        }
        if let Some(style) = value.attr("style") {
            css_images(style, &mut add);
        }
        if value.name() == "style" {
            css_images(&element.text().collect::<String>(), &mut add);
        }
    }
    images
}
fn css_images(source: &str, add: &mut impl FnMut(&str, &str)) {
    use cssparser::{Parser, ParserInput, Token};
    fn scan<'i>(parser: &mut Parser<'i, '_>, add: &mut impl FnMut(&str, &str), depth: u8) {
        while let Ok(token) = parser.next().cloned() {
            match token {
                Token::AtKeyword(name)
                    if name.eq_ignore_ascii_case("import")
                        || name.eq_ignore_ascii_case("font-face")
                        || name.eq_ignore_ascii_case("namespace") =>
                {
                    while let Ok(token) = parser.next() {
                        if matches!(token, Token::Semicolon | Token::CurlyBracketBlock) {
                            break;
                        }
                    }
                }
                Token::UnquotedUrl(url) => add(&url, "Email background"),
                Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                    let _: Result<(), cssparser::ParseError<'i, ()>> =
                        parser.parse_nested_block(|p| {
                            if let Ok(value) = p.expect_string() {
                                add(value, "Email background");
                            }
                            Ok(())
                        });
                }
                Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
                    if depth < 64 =>
                {
                    let _: Result<(), cssparser::ParseError<'i, ()>> =
                        parser.parse_nested_block(|p| {
                            scan(p, add, depth + 1);
                            Ok(())
                        });
                }
                _ => {}
            }
        }
    }
    scan(&mut Parser::new(&mut ParserInput::new(source)), add, 0);
}
/// Fetch only after a UI policy decision through the shared, address-checked
/// service; never forward account credentials or cookies.
pub async fn fetch(input: &str) -> anyhow::Result<Vec<u8>> {
    Ok(shared::fetch(&ReqwestTransport::new(), input).await?.bytes)
}
pub fn convert_to_webp(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    Ok(shared::convert_to_webp(bytes)?.bytes)
}

#[cfg(test)]
mod tests {
    #[test]
    fn webp_conversion_preserves_small_images_natural_size() {
        let image = image::DynamicImage::new_rgba8(32, 16);
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        let converted = super::convert_to_webp(&bytes.into_inner()).unwrap();
        let decoded = image::load_from_memory(&converted).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (32, 16));
    }
}
