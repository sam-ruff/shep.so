use super::{Image, RemoteImage};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
};
use url::Url;

#[derive(Default)]
pub(super) struct Resources {
    pub images: BTreeMap<String, Image>,
    converted: BTreeMap<String, Option<String>>,
    pub remote: BTreeMap<String, RemoteImage>,
    pub issues: BTreeSet<String>,
    decoded_bytes: usize,
}
impl Resources {
    pub fn image(
        &mut self,
        source: &str,
        alt: &str,
        base: Option<&Url>,
        part: &crate::reader::RawHtmlPart,
        body: &crate::reader::Body,
    ) -> Option<String> {
        let source = source.trim();
        let (scheme, payload) = source.split_once(':').unwrap_or(("", ""));
        if scheme.eq_ignore_ascii_case("cid") {
            let cid = payload;
            let cid = percent_encoding::percent_decode_str(cid)
                .decode_utf8()
                .ok()?;
            let key = part.inline.get(cid.trim_matches(['<', '>']))?.as_ref()?;
            return self.convert(key, body.resources.get(key)?);
        }
        if scheme.eq_ignore_ascii_case("data") {
            let data = payload;
            let (kind, data) = data.split_once(',')?;
            if !kind
                .split(';')
                .next()?
                .to_ascii_lowercase()
                .starts_with("image/")
            {
                return None;
            }
            let bytes = if kind
                .split(';')
                .any(|part| part.eq_ignore_ascii_case("base64"))
            {
                STANDARD.decode(data).ok()?
            } else {
                percent_encoding::percent_decode_str(data).collect()
            };
            if bytes.len() > crate::MAX_MESSAGE_BYTES {
                return None;
            }
            return self.convert(&format!("{:x}", Sha256::digest(&bytes)), &bytes);
        }
        if let Some(url) = web_url(source, base) {
            self.remote
                .entry(url.clone())
                .or_insert_with(|| RemoteImage {
                    url,
                    alt: alt.chars().take(160).collect(),
                });
        }
        None
    }
    fn convert(&mut self, key: &str, bytes: &[u8]) -> Option<String> {
        if let Some(value) = self.converted.get(key) {
            return value.clone();
        }
        let result = self.decode(bytes);
        if result.is_none() {
            self.issues
                .insert("An inline image is invalid or exceeds the current image limit.".into());
        }
        self.converted.insert(key.into(), result.clone());
        result
    }
    fn decode(&mut self, bytes: &[u8]) -> Option<String> {
        let mut reader = image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .ok()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(2048);
        limits.max_image_height = Some(2048);
        limits.max_alloc = Some(32 * 1024 * 1024);
        reader.limits(limits);
        let image = reader.decode().ok()?;
        let (width, height) = (image.width(), image.height());
        let mut output = Cursor::new(Vec::new());
        image.write_to(&mut output, image::ImageFormat::WebP).ok()?;
        let bytes = output.into_inner();
        let key = format!("{:x}", Sha256::digest(&bytes));
        if !self.images.contains_key(&key) {
            let decoded = width as usize * height as usize * 4;
            if self.decoded_bytes + decoded > 32 * 1024 * 1024 {
                return None;
            }
            self.decoded_bytes += decoded;
            self.images.insert(
                key.clone(),
                Image {
                    width,
                    height,
                    bytes: STANDARD.encode(bytes),
                },
            );
        }
        Some(key)
    }
}
pub(super) fn web_url(source: &str, base: Option<&Url>) -> Option<String> {
    let url = Url::parse(source)
        .or_else(|_| {
            base.ok_or(url::ParseError::RelativeUrlWithoutBase)?
                .join(source)
        })
        .ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.into())
}
pub(super) fn link(source: &str, base: Option<&Url>) -> Option<String> {
    if let Some(fragment) = source.strip_prefix('#') {
        return Some(format!("#{}", fragment));
    }
    if let Some(url) = web_url(source, base) {
        return Some(url);
    }
    let url = Url::parse(source).ok()?;
    (url.scheme() == "mailto").then(|| url.into())
}
