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
        };
        config.validate()?;
        Ok(config)
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
}
