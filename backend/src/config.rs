use std::{collections::HashSet, net::SocketAddr, path::PathBuf};
use url::Url;
use zeroize::Zeroizing;

pub struct Config {
    pub origin: String,
    pub google_client_id: String,
    pub google_client_secret: Zeroizing<String>,
    pub allowed_emails: HashSet<String>,
    pub allowed_subjects: HashSet<String>,
    pub web_dir: PathBuf,
    pub bind: SocketAddr,
    pub mail_endpoints: Vec<crate::mail::policy::Endpoint>,
    pub caldav_endpoints: Vec<CalDavEndpoint>,
    /// Shared Shep application namespace for profile app data; absent means
    /// profile sync is not offered by this server.
    pub profile_namespace: Option<String>,
}
impl Config {
    pub fn from_env() -> Result<Self, &'static str> {
        let value =
            |name| std::env::var(name).map_err(|_| "Required server configuration is missing");
        let list = |name| {
            std::env::var(name)
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect::<HashSet<_>>()
        };
        let config = Self {
            origin: value("SHEP_PUBLIC_ORIGIN")?,
            google_client_id: value("SHEP_GOOGLE_CLIENT_ID")?,
            google_client_secret: Zeroizing::new(value("SHEP_GOOGLE_CLIENT_SECRET")?),
            allowed_emails: list("SHEP_BETA_EMAILS")
                .into_iter()
                .map(|s| s.to_lowercase())
                .collect(),
            allowed_subjects: list("SHEP_BETA_SUBJECTS"),
            web_dir: value("SHEP_WEB_DIR")?.into(),
            bind: std::env::var("SHEP_BIND")
                .unwrap_or_else(|_| "127.0.0.1:3080".into())
                .parse()
                .map_err(|_| "Invalid listen address")?,
            mail_endpoints: serde_json::from_str(
                &std::env::var("SHEP_MAIL_ENDPOINTS").unwrap_or_else(|_| "[]".into()),
            )
            .map_err(|_| "Invalid mail endpoint configuration")?,
            caldav_endpoints: serde_json::from_str(
                &std::env::var("SHEP_CALDAV_ENDPOINTS").unwrap_or_else(|_| "[]".into()),
            )
            .map_err(|_| "Invalid CalDAV endpoint configuration")?,
            profile_namespace: std::env::var("SHEP_PROFILE_NAMESPACE")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
        };
        config.validate()?;
        Ok(config)
    }
    fn namespace_valid(value: &str) -> bool {
        value.len() <= 128
            && value.contains('.')
            && value.split('.').all(|s| {
                !s.is_empty()
                    && s.len() <= 63
                    && !s.starts_with('-')
                    && !s.ends_with('-')
                    && s.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let url = Url::parse(&self.origin).map_err(|_| "Invalid public HTTPS origin")?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
            || self.origin.ends_with('/')
        {
            return Err("Use a public HTTPS origin without a trailing slash or path");
        }
        if !self.bind.ip().is_loopback() {
            return Err("Server must bind to loopback behind the HTTPS proxy");
        }
        if self.google_client_id.is_empty() || self.google_client_secret.is_empty() {
            return Err("Google login configuration is required");
        }
        crate::mail::policy::validate(&self.mail_endpoints)?;
        validate_caldav_endpoints(&self.caldav_endpoints)?;
        if self
            .profile_namespace
            .as_deref()
            .is_some_and(|n| !Self::namespace_valid(n))
        {
            return Err("Use a dotted lowercase profile namespace such as so.shep.profiles");
        }
        if self
            .allowed_emails
            .iter()
            .any(|s| !s.contains('@') || s.contains(['\r', '\n', ' ']))
        {
            return Err("Use exact beta email addresses");
        }
        Ok(())
    }
    pub fn permits(&self, email: &str, subject: &str) -> bool {
        self.allowed_emails.contains(&email.to_lowercase())
            && (self.allowed_subjects.is_empty() || self.allowed_subjects.contains(subject))
    }
    pub fn callback(&self) -> String {
        format!("{}/auth/callback", self.origin)
    }
    /// Separate redirect for provider consent, registered alongside the login one.
    pub fn google_callback(&self) -> String {
        format!("{}/auth/google/callback", self.origin)
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct CalDavEndpoint {
    pub id: String,
    pub name: String,
    pub url: String,
    pub address: SocketAddr,
}

pub fn validate_caldav_endpoints(endpoints: &[CalDavEndpoint]) -> Result<(), &'static str> {
    if endpoints.len() > 32 {
        return Err("Configure at most 32 CalDAV endpoints");
    }
    let mut ids = HashSet::new();
    for endpoint in endpoints {
        if endpoint.id.is_empty()
            || endpoint.id.len() > 128
            || !endpoint
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || endpoint.name.is_empty()
            || endpoint.name.len() > 256
            || !ids.insert(endpoint.id.as_str())
            || endpoint.address.ip().is_unspecified()
            || endpoint.address.ip().is_multicast()
        {
            return Err("Invalid CalDAV endpoint configuration");
        }
        let url = Url::parse(&endpoint.url).map_err(|_| "Invalid CalDAV endpoint URL")?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port_or_known_default() != Some(endpoint.address.port())
        {
            return Err("Use a pinned HTTPS CalDAV collection URL");
        }
    }
    Ok(())
}

#[cfg(test)]
mod caldav_tests {
    use super::*;

    fn endpoint() -> CalDavEndpoint {
        CalDavEndpoint {
            id: "home".into(),
            name: "Home".into(),
            url: "https://calendar.example.test:8443/home/".into(),
            address: "192.0.2.4:8443".parse().expect("address"),
        }
    }

    #[test]
    fn caldav_endpoints_require_exact_unique_https_pins() {
        assert!(validate_caldav_endpoints(&[endpoint()]).is_ok());
        let mut duplicate = endpoint();
        duplicate.url = "https://other.example.test:8443/home/".into();
        assert!(validate_caldav_endpoints(&[endpoint(), duplicate]).is_err());
        let mut insecure = endpoint();
        insecure.url = "http://calendar.example.test:8443/home/".into();
        assert!(validate_caldav_endpoints(&[insecure]).is_err());
        let mut wrong_port = endpoint();
        wrong_port.address = "192.0.2.4:443".parse().expect("address");
        assert!(validate_caldav_endpoints(&[wrong_port]).is_err());
        let mut credential_url = endpoint();
        credential_url.url = "https://sam:secret@calendar.example.test:8443/home/".into();
        assert!(validate_caldav_endpoints(&[credential_url]).is_err());
    }
}
