//! One bounded image download. Every hop is resolved and checked before a
//! connection is made, the client connects only to those checked addresses,
//! redirects are followed here rather than by the HTTP client, and TLS keeps
//! certificate and hostname verification. No cookies, proxy, credentials or
//! referrer are ever sent.
use super::{Webp, convert_to_webp, public_ip};
use async_trait::async_trait;
use reqwest::{Url, header, redirect};
use std::{fmt, net::SocketAddr, time::Duration};
use url::Host;

pub const MAX_DOWNLOAD_BYTES: usize = 4 * 1024 * 1024;
const MAX_REQUESTS: usize = 4;

/// Fixed messages: remote server text never reaches the reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchError {
    Unsupported,
    PrivateAddress,
    Unavailable,
    Rejected,
    Redirects,
    TooLarge,
    Invalid,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "This image address is not supported.",
            Self::PrivateAddress => "Images from private network addresses are blocked.",
            Self::Unavailable => "The image server could not be reached. Retry later.",
            Self::Rejected => "The image server did not return this image.",
            Self::Redirects => "This image redirected too many times.",
            Self::TooLarge => "This image exceeds the 4 MiB download limit.",
            Self::Invalid => "This image is invalid or exceeds the image size limit.",
        })
    }
}

impl std::error::Error for FetchError {}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

/// Network boundary of the fetch service; the orchestration stays pure.
#[cfg_attr(any(test, feature = "remote-images-test"), mockall::automock)]
#[async_trait]
pub trait Transport: Send + Sync {
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, FetchError>;
    /// One GET that never follows a redirect, connects only to `addresses`
    /// and reads at most `limit` body bytes.
    async fn get(
        &self,
        url: &Url,
        addresses: &[SocketAddr],
        limit: usize,
    ) -> Result<Reply, FetchError>;
}

/// Download one discovered image, checking every hop's resolved addresses.
pub async fn download(transport: &dyn Transport, url: &str) -> Result<Vec<u8>, FetchError> {
    let mut url = Url::parse(url).map_err(|_| FetchError::Unsupported)?;
    for _ in 0..MAX_REQUESTS {
        let addresses = checked_addresses(transport, &url).await?;
        let reply = transport.get(&url, &addresses, MAX_DOWNLOAD_BYTES).await?;
        if matches!(reply.status, 301 | 302 | 303 | 307 | 308) {
            let target = reply.location.ok_or(FetchError::Rejected)?;
            let next = url.join(&target).map_err(|_| FetchError::Unsupported)?;
            if url.scheme() == "https" && next.scheme() != "https" {
                return Err(FetchError::Unsupported);
            }
            url = next;
            continue;
        }
        if !(200..300).contains(&reply.status) {
            return Err(FetchError::Rejected);
        }
        if reply.body.len() > MAX_DOWNLOAD_BYTES {
            return Err(FetchError::TooLarge);
        }
        return Ok(reply.body);
    }
    Err(FetchError::Redirects)
}

/// Download, then decode and re-encode on a blocking worker.
pub async fn fetch(transport: &dyn Transport, url: &str) -> Result<Webp, FetchError> {
    let bytes = download(transport, url).await?;
    tokio::task::spawn_blocking(move || convert_to_webp(&bytes))
        .await
        .map_err(|_| FetchError::Invalid)?
        .map_err(|_| FetchError::Invalid)
}

async fn checked_addresses(
    transport: &dyn Transport,
    url: &Url,
) -> Result<Vec<SocketAddr>, FetchError> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(FetchError::Unsupported);
    }
    let port = url.port_or_known_default().ok_or(FetchError::Unsupported)?;
    let addresses = match url.host().ok_or(FetchError::Unsupported)? {
        Host::Ipv4(ip) => vec![SocketAddr::new(ip.into(), port)],
        Host::Ipv6(ip) => vec![SocketAddr::new(ip.into(), port)],
        Host::Domain(name) => transport.resolve(name, port).await?,
    };
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !public_ip(address.ip()) || address.port() != port)
    {
        return Err(FetchError::PrivateAddress);
    }
    Ok(addresses)
}

/// Production transport: system resolver and rustls with the bundled web PKI roots.
#[derive(Default)]
pub struct ReqwestTransport {
    #[cfg(test)]
    roots: Vec<reqwest::Certificate>,
}

impl ReqwestTransport {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(super) fn trusting(root: reqwest::Certificate) -> Self {
        Self { roots: vec![root] }
    }
}

#[async_trait]
impl Transport for ReqwestTransport {
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, FetchError> {
        let found = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| FetchError::Unavailable)?
        .map_err(|_| FetchError::Unavailable)?;
        Ok(found.collect())
    }

    async fn get(
        &self,
        url: &Url,
        addresses: &[SocketAddr],
        limit: usize,
    ) -> Result<Reply, FetchError> {
        let host = url.host_str().ok_or(FetchError::Unsupported)?;
        let builder = reqwest::Client::builder()
            .no_proxy()
            .redirect(redirect::Policy::none())
            .referer(false)
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .resolve_to_addrs(host, addresses);
        #[cfg(test)]
        let builder = self.roots.iter().fold(builder, |builder, root| {
            builder.add_root_certificate(root.clone())
        });
        let client = builder.build().map_err(|_| FetchError::Unavailable)?;
        let mut response = client
            .get(url.clone())
            .header(header::ACCEPT, "image/webp,image/png,image/jpeg,image/gif")
            .send()
            .await
            .map_err(|_| FetchError::Unavailable)?;
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        if response.status().is_redirection() {
            return Ok(Reply {
                status,
                location,
                body: Vec::new(),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit as u64)
        {
            return Err(FetchError::TooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| FetchError::Unavailable)?
        {
            if body.len() + chunk.len() > limit {
                return Err(FetchError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Reply {
            status,
            location,
            body,
        })
    }
}
