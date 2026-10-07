use super::{Image, RemoteImage};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
};
use url::Url;

/// How discovered remote images appear in prepared output. Preparation never
/// fetches anything itself.
#[derive(Clone, Copy, Default)]
pub(crate) enum Remote<'a> {
    /// Discovered only; the element keeps no remote source.
    #[default]
    Omit,
    /// A stable slot the display runtime fills once permitted bytes arrive.
    Placeholder,
    /// Embed bytes the caller already holds for permitted images.
    Cached(&'a dyn Fn(&str) -> Option<Vec<u8>>),
}

pub(crate) struct Resources<'a> {
    pub images: BTreeMap<String, Image>,
    converted: BTreeMap<String, Option<String>>,
    pub remote: BTreeMap<String, RemoteImage>,
    pub issues: BTreeSet<String>,
    decoded_bytes: usize,
    mode: Remote<'a>,
    inline: bool,
}
impl<'a> Resources<'a> {
    /// `inline` converts CID/data images; discovery alone skips that work.
    pub fn new(mode: Remote<'a>, inline: bool) -> Self {
        Self {
            images: BTreeMap::new(),
            converted: BTreeMap::new(),
            remote: BTreeMap::new(),
            issues: BTreeSet::new(),
            decoded_bytes: 0,
            mode,
            inline,
        }
    }

    /// The resource URN that replaces `source`, if any.
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
        let embedded = scheme.eq_ignore_ascii_case("cid") || scheme.eq_ignore_ascii_case("data");
        if embedded && !self.inline {
            return None;
        }
        if scheme.eq_ignore_ascii_case("cid") {
            let cid = payload;
            let cid = percent_encoding::percent_decode_str(cid)
                .decode_utf8()
                .ok()?;
            let key = part.inline.get(cid.trim_matches(['<', '>']))?.as_ref()?;
            return self.inline_urn(key, body.resources.get(key)?);
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
            return self.inline_urn(&format!("{:x}", Sha256::digest(&bytes)), &bytes);
        }
        let url = web_url(source, base)?;
        let key = remote_key(&url);
        self.remote
            .entry(url.clone())
            .or_insert_with(|| RemoteImage {
                url: url.clone(),
                alt: alt.chars().take(160).collect(),
                key: key.clone(),
            });
        match self.mode {
            Remote::Omit => None,
            Remote::Placeholder => Some(format!("urn:shep-remote:{key}")),
            Remote::Cached(cached) => {
                let bytes = cached(&url)?;
                self.inline_urn(&format!("remote:{key}"), &bytes)
            }
        }
    }
    fn inline_urn(&mut self, key: &str, bytes: &[u8]) -> Option<String> {
        self.convert(key, bytes)
            .map(|key| format!("urn:shep-image:{key}"))
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
/// Identifies a remote image in prepared documents without exposing its URL.
pub(crate) fn remote_key(url: &str) -> String {
    format!("{:x}", Sha256::digest(url.as_bytes()))
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
