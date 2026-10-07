//! Remote image permission and conversion shared by the native clients.
//! Sender HTML never reaches the network: a client fetches only URLs discovered
//! in the cached message, after these rules allow that message, and hands the
//! confined renderer converted WebP bytes.
use crate::model::ImagePolicy;
use std::{io::Cursor, net::IpAddr};

#[cfg(feature = "remote-images")]
mod fetch;
#[cfg(feature = "remote-images")]
pub use fetch::*;

#[cfg(test)]
mod tests;

/// The synced default policy plus device-local explicit exceptions.
#[derive(Clone, Copy, Debug)]
pub struct Rules<'a> {
    pub policy: ImagePolicy,
    pub messages: &'a [String],
    pub senders: &'a [String],
    pub domains: &'a [String],
    pub contacts: &'a [String],
}

impl Rules<'_> {
    /// Sender identity in email is not verified; exceptions match the parsed
    /// From address exactly, never a display name or a parent domain.
    pub fn allows(&self, message: &str, sender: &str) -> bool {
        if self.policy == ImagePolicy::AllowAll || self.messages.iter().any(|id| id == message) {
            return true;
        }
        let Some(address) = sender_address(sender) else {
            return false;
        };
        self.senders.contains(&address)
            || sender_domain(sender).is_some_and(|domain| self.domains.contains(&domain))
            || (self.policy == ImagePolicy::Contacts
                && self
                    .contacts
                    .iter()
                    .any(|contact| contact.eq_ignore_ascii_case(&address)))
    }
}

pub fn sender_address(sender: &str) -> Option<String> {
    let parsed = mailparse::addrparse(sender).ok()?;
    let single = parsed.extract_single_info()?;
    Some(single.addr.to_lowercase())
}

pub fn sender_domain(sender: &str) -> Option<String> {
    let address = sender_address(sender)?;
    Some(address.rsplit_once('@')?.1.to_string())
}

/// Globally routable unicast addresses only.
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Webp {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Decode with dimension and allocation limits, then re-encode as WebP. Call
/// from a blocking worker; images larger than 1024 pixels are scaled down.
pub fn convert_to_webp(bytes: &[u8]) -> anyhow::Result<Webp> {
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
    Ok(Webp {
        bytes: output.into_inner(),
        width: decoded.width(),
        height: decoded.height(),
    })
}
