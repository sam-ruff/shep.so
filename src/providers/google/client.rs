//! Shep's own installed-app ("Desktop app") Google OAuth client.
//!
//! Google treats installed-app client secrets as non-confidential, so release
//! builds embed both values at compile time:
//! `SHEP_GOOGLE_CLIENT_ID=... SHEP_GOOGLE_CLIENT_SECRET=... cargo build --release`.
//! Development and test builds also read the same variables at runtime; a set
//! but empty ID there simulates a build without a client.
use crate::model::Preferences;
use std::sync::OnceLock;

pub(crate) const NOT_CONFIGURED: &str = "Google sign-in is not configured in this build.";

/// An OAuth client that can issue or refresh a grant.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct OAuthClient {
    pub(crate) id: String,
    pub(crate) secret: String,
}

impl std::fmt::Debug for OAuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthClient")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl OAuthClient {
    /// Google desktop clients need both the ID and the secret.
    pub(crate) fn new(id: &str, secret: &str) -> Option<Self> {
        let (id, secret) = (id.trim(), secret.trim());
        (!id.is_empty() && !secret.is_empty()).then(|| Self {
            id: id.to_owned(),
            secret: secret.to_owned(),
        })
    }
}

/// A runtime override replaces the built-in client entirely, including with none.
pub(crate) fn select(
    built_in: Option<OAuthClient>,
    runtime: Option<(String, String)>,
) -> Option<OAuthClient> {
    match runtime {
        Some((id, secret)) => OAuthClient::new(&id, &secret),
        None => built_in,
    }
}

/// The client this process uses for new sign-ins.
pub(crate) fn sign_in() -> Option<&'static OAuthClient> {
    static CLIENT: OnceLock<Option<OAuthClient>> = OnceLock::new();
    CLIENT
        .get_or_init(|| select(built_in(), runtime_override()))
        .as_ref()
}

fn built_in() -> Option<OAuthClient> {
    OAuthClient::new(
        option_env!("SHEP_GOOGLE_CLIENT_ID").unwrap_or_default(),
        option_env!("SHEP_GOOGLE_CLIENT_SECRET").unwrap_or_default(),
    )
}

#[cfg(any(debug_assertions, feature = "test-support"))]
fn runtime_override() -> Option<(String, String)> {
    let id = std::env::var("SHEP_GOOGLE_CLIENT_ID").ok()?;
    Some((
        id,
        std::env::var("SHEP_GOOGLE_CLIENT_SECRET").unwrap_or_default(),
    ))
}

#[cfg(not(any(debug_assertions, feature = "test-support")))]
fn runtime_override() -> Option<(String, String)> {
    None
}

/// The secret for refreshing a grant: this build's when its client issued the
/// grant, then a legacy self-configured client's stored secret, then the
/// secret saved with the grant.
pub(crate) fn refresh_secret<'a>(
    sign_in: Option<&'a OAuthClient>,
    prefs: &'a Preferences,
    grant_client: &str,
    grant_secret: &'a str,
) -> &'a str {
    if let Some(client) = sign_in.filter(|client| client.id == grant_client) {
        return &client.secret;
    }
    if prefs.google_client_id == grant_client && !prefs.google_client_secret.is_empty() {
        return &prefs.google_client_secret;
    }
    grant_secret
}

/// The committed grant came from a client other than this build's sign-in client.
pub(crate) fn legacy_grant(prefs: &Preferences, sign_in: Option<&str>) -> bool {
    let active = prefs.active_google_client();
    sign_in.is_some_and(|id| !active.is_empty() && active != id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(id: &str, secret: &str) -> OAuthClient {
        OAuthClient::new(id, secret).unwrap()
    }

    #[test]
    fn a_usable_client_needs_both_halves_and_never_prints_its_secret() {
        assert!(OAuthClient::new("", "secret").is_none());
        assert!(OAuthClient::new("id", "  ").is_none());
        let built_in = client(" built-in.apps.googleusercontent.com ", "built-in-secret");
        assert_eq!(built_in.id, "built-in.apps.googleusercontent.com");
        assert!(!format!("{built_in:?}").contains("built-in-secret"));
    }

    #[test]
    fn built_in_override_and_unconfigured_builds_select_the_sign_in_client() {
        let built_in = client("built-in", "built-in-secret");
        assert_eq!(select(Some(built_in.clone()), None), Some(built_in.clone()));
        assert_eq!(select(None, None), None);
        let dev = ("dev-client".to_owned(), "dev-secret".to_owned());
        assert_eq!(
            select(Some(built_in.clone()), Some(dev.clone())),
            Some(client("dev-client", "dev-secret"))
        );
        assert_eq!(
            select(None, Some(dev)),
            Some(client("dev-client", "dev-secret"))
        );
        // An empty override simulates an unconfigured build over a built-in client.
        assert_eq!(
            select(Some(built_in.clone()), Some((String::new(), String::new()))),
            None
        );
        assert_eq!(
            select(Some(built_in), Some(("dev-client".into(), String::new()))),
            None
        );
    }

    #[test]
    fn refresh_uses_the_client_that_issued_the_grant() {
        let built_in = client("built-in", "current-secret");
        let legacy = Preferences {
            google_client_id: "own-client".into(),
            google_client_secret: "own-secret".into(),
            ..Default::default()
        };
        // This build's grants follow its current (possibly rotated) secret.
        assert_eq!(
            refresh_secret(Some(&built_in), &legacy, "built-in", "old-secret"),
            "current-secret"
        );
        // A self-configured client keeps refreshing with its stored secret.
        assert_eq!(
            refresh_secret(Some(&built_in), &legacy, "own-client", ""),
            "own-secret"
        );
        assert_eq!(
            refresh_secret(None, &legacy, "own-client", "vault-secret"),
            "own-secret"
        );
        // Without stored preferences, the secret saved with the grant is used.
        let cleared = Preferences::default();
        assert_eq!(
            refresh_secret(Some(&built_in), &cleared, "own-client", "vault-secret"),
            "vault-secret"
        );
        assert_eq!(
            refresh_secret(
                Some(&built_in),
                &cleared,
                "earlier-built-in",
                "earlier-secret"
            ),
            "earlier-secret"
        );
    }

    #[test]
    fn only_a_grant_from_another_client_is_legacy() {
        let mut prefs = Preferences::default();
        assert!(!legacy_grant(&prefs, Some("built-in")));
        prefs.google_client_id = "own-client".into();
        assert!(legacy_grant(&prefs, Some("built-in")));
        assert!(!legacy_grant(&prefs, None));
        prefs.google_grant.client_id = "built-in".into();
        assert!(!legacy_grant(&prefs, Some("built-in")));
    }
}
