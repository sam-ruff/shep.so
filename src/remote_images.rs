use crate::model::{ImagePolicy, Mail, Preferences, RemoteImage};
use anyhow::Context;
use std::{
    io::Cursor,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

pub fn sender_address(sender: &str) -> Option<String> {
    let parsed = mailparse::addrparse(sender).ok()?;
    let single = parsed.extract_single_info()?;
    Some(single.addr.to_lowercase())
}
pub fn sender_domain(sender: &str) -> Option<String> {
    let address = sender_address(sender)?;
    Some(address.rsplit_once('@')?.1.to_string())
}
pub fn allowed(preferences: &Preferences, mail: &Mail) -> bool {
    if preferences.image_policy == ImagePolicy::AllowAll
        || preferences.image_messages.contains(&mail.id)
    {
        return true;
    }
    let Some(address) = sender_address(&mail.sender) else {
        return false;
    };
    preferences.image_senders.contains(&address)
        || sender_domain(&mail.sender).is_some_and(|d| preferences.image_domains.contains(&d))
        || (preferences.image_policy == ImagePolicy::Contacts
            && preferences
                .contacts
                .iter()
                .any(|c| c.eq_ignore_ascii_case(&address)))
}
pub fn extract(parsed: &mailparse::ParsedMail<'_>) -> Vec<RemoteImage> {
    crate::email_content::extract(parsed)
        .html
        .as_ref()
        .map(|h| extract_html(&h.source))
        .unwrap_or_default()
}
pub fn extract_html(source: &str) -> Vec<RemoteImage> {
    let html = scraper::Html::parse_document(source);
    let selector = scraper::Selector::parse("img[src]").expect("static selector");
    let mut images = Vec::new();
    for element in html.select(&selector) {
        let value = element.value();
        if let Ok(url) = url::Url::parse(value.attr("src").unwrap_or_default())
            && matches!(url.scheme(), "https" | "http")
            && url.username().is_empty()
            && url.password().is_none()
            && !images.iter().any(|i: &RemoteImage| i.url == url.as_str())
        {
            images.push(RemoteImage {
                url: url.to_string(),
                alt: value
                    .attr("alt")
                    .unwrap_or("Email image")
                    .chars()
                    .take(160)
                    .collect(),
            });
        }
    }
    images
}
pub fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !ip.is_unspecified()
                && !ip.is_multicast()
                && ip.octets()[0] != 0
                && ip.octets()[0] < 224
                && !(ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
                && !(ip.octets()[0] == 198 && matches!(ip.octets()[1], 18 | 19))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                public_ip(IpAddr::V4(v4))
            } else {
                ip.segments()[0] & 0xe000 == 0x2000
                    && !(ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8)
            }
        }
    }
}
/// Fetch only after a UI policy decision; never forward account credentials or cookies.
pub async fn fetch(input: &str) -> anyhow::Result<Vec<u8>> {
    let mut url = url::Url::parse(input)?;
    for _ in 0..4 {
        anyhow::ensure!(
            matches!(url.scheme(), "https" | "http")
                && url.username().is_empty()
                && url.password().is_none(),
            "Unsupported image URL"
        );
        let host = url.host_str().context("Image hostname is missing")?;
        let port = url
            .port_or_known_default()
            .context("Image port is missing")?;
        let addresses: Vec<SocketAddr> = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host((host, port)),
        )
        .await??
        .collect();
        anyhow::ensure!(
            !addresses.is_empty() && addresses.iter().all(|a| public_ip(a.ip())),
            "Images from private network addresses are blocked"
        );
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .resolve_to_addrs(host, &addresses)
            .build()?;
        let mut response = client
            .get(url.clone())
            .header("Accept", "image/webp,image/png,image/jpeg,image/gif")
            .send()
            .await?;
        if response.status().is_redirection() {
            let target = response
                .headers()
                .get(reqwest::header::LOCATION)
                .context("Image redirect has no destination")?
                .to_str()?;
            url = url.join(target)?;
            continue;
        }
        response = response.error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= 4 * 1024 * 1024,
                "Image exceeds the 4 MiB download limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        return tokio::task::spawn_blocking(move || convert_to_webp(&bytes)).await?;
    }
    anyhow::bail!("Too many image redirects")
}
pub fn convert_to_webp(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode()?;
    let decoded = if decoded.width() > 1024 || decoded.height() > 1024 {
        decoded.thumbnail(1024, 1024)
    } else {
        decoded
    };
    let mut output = Cursor::new(Vec::new());
    decoded.write_to(&mut output, image::ImageFormat::WebP)?;
    Ok(output.into_inner())
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
