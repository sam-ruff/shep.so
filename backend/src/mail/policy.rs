use serde::{Deserialize, Serialize};
use shep_mail_core::model::{Account, Protocol};
use std::net::SocketAddr;

#[derive(Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    Imap,
    Pop3,
    Smtp,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub service: Service,
    pub address: SocketAddr,
}
pub fn validate(endpoints: &[Endpoint]) -> Result<(), &'static str> {
    if endpoints.len() > 128 {
        return Err("Configure at most 128 explicit mail endpoints");
    }
    let mut unique = std::collections::HashSet::new();
    for endpoint in endpoints {
        if endpoint.host.is_empty()
            || endpoint.host.len() > 253
            || !endpoint
                .host
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'.')
            || endpoint.host.starts_with('.')
            || endpoint.host.ends_with('.')
            || endpoint.port == 0
            || endpoint.address.port() == 0
            || endpoint.address.ip().is_unspecified()
            || endpoint.address.ip().is_multicast()
        {
            return Err("Use exact mail hostnames, ports and configured IP destinations");
        }
        let key = (
            endpoint.host.to_ascii_lowercase(),
            endpoint.port,
            endpoint.service as u8,
        );
        if !unique.insert(key) {
            return Err("Mail endpoint entries must be unique");
        }
    }
    Ok(())
}
pub fn incoming(account: &Account) -> Service {
    match account.protocol {
        Protocol::Imap => Service::Imap,
        Protocol::Pop3 => Service::Pop3,
    }
}
pub fn resolve(
    endpoints: &[Endpoint],
    host: &str,
    port: u16,
    service: Service,
) -> Result<SocketAddr, &'static str> {
    endpoints
        .iter()
        .find(|e| e.host.eq_ignore_ascii_case(host) && e.port == port && e.service == service)
        .map(|e| e.address)
        .ok_or("This mail endpoint is not enabled for the beta. Ask the administrator to add it.")
}
