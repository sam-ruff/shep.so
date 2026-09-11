use crate::model::Preferences;
use anyhow::Context;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use std::{sync::Arc, time::Duration};
pub use tokio_util::sync::CancellationToken;

mod callback;
pub(crate) mod client;
mod consent;
mod owner;
mod scopes;
pub(crate) use scopes::Service;
#[cfg(test)]
mod tests;
mod tokens;
use owner::{Backend, Owner};
use tokens::{CredentialStore, OsCredentialStore};

#[cfg(test)]
const SCOPES: &str = "https://www.googleapis.com/auth/drive.appdata https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/calendar.calendarlist.readonly";

/// Opening the browser and consenting there must finish within this bound.
const SIGN_IN_WAIT: Duration = Duration::from_secs(180);

/// Why an interactive sign-in stopped before changing the saved connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignInError {
    NotConfigured,
    Denied,
    Refused,
    Cancelled,
    TimedOut,
    CodeExpired,
}
impl std::fmt::Display for SignInError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotConfigured => client::NOT_CONFIGURED,
            Self::Denied => "Google sign-in was cancelled or access was denied in the browser. Nothing changed; sign in again to grant access.",
            Self::Refused => "Google could not complete sign-in. Nothing changed; try again.",
            Self::Cancelled => "Google sign-in was cancelled. Nothing changed.",
            Self::TimedOut => "Google sign-in timed out waiting for the browser. Nothing changed; sign in again.",
            Self::CodeExpired => "Google's sign-in response expired before Shep could use it. Nothing changed; sign in again.",
        })
    }
}
impl std::error::Error for SignInError {}

/// Opens Google's consent page; tests stand in for the user's browser.
trait Browser: Send + Sync {
    fn open(&self, url: &str) -> anyhow::Result<()>;
}
struct SystemBrowser;
impl Browser for SystemBrowser {
    fn open(&self, url: &str) -> anyhow::Result<()> {
        webbrowser::open(url).context("Could not open your web browser for Google sign-in.")
    }
}

/// Everything an interactive sign-in needs besides the token vault.
#[derive(Clone)]
struct SignIn {
    client: Option<client::OAuthClient>,
    browser: Arc<dyn Browser>,
    wait: Duration,
}

#[derive(Clone)]
pub struct Google {
    pub http: reqwest::Client,
    // The vault state lives on its owning thread; callers only send jobs.
    tokens: Owner,
    pub(crate) api_base: url::Url,
    sign_in: SignIn,
}
impl Default for Google {
    fn default() -> Self {
        Self::with_credentials(crate::credentials::Credentials::default())
    }
}
impl Google {
    pub(crate) fn with_credentials(credentials: crate::credentials::Credentials) -> Self {
        Self::start(
            reqwest::Client::builder()
                .timeout(Duration::from_secs(45))
                .connect_timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("valid HTTP configuration"),
            Arc::new(OsCredentialStore { credentials }),
            url::Url::parse("https://oauth2.googleapis.com/token")
                .expect("official Google token endpoint"),
            url::Url::parse("https://www.googleapis.com/").expect("official Google API URL"),
            SignIn {
                client: client::sign_in().cloned(),
                browser: Arc::new(SystemBrowser),
                wait: SIGN_IN_WAIT,
            },
        )
    }

    /// Private, object-scoped endpoint injection for loopback protocol tests.
    fn start(
        http: reqwest::Client,
        credentials: Arc<dyn CredentialStore>,
        token_endpoint: url::Url,
        api_base: url::Url,
        sign_in: SignIn,
    ) -> Self {
        Self {
            tokens: Owner::start(Backend {
                http: http.clone(),
                credentials,
                token_endpoint,
                client: sign_in.client.clone(),
            }),
            http,
            api_base,
            sign_in,
        }
    }
}
impl Google {
    pub async fn connected(&self, prefs: &Preferences) -> anyhow::Result<bool> {
        if prefs.active_google_client().trim().is_empty() || prefs.google_lifecycle.disconnected {
            return Ok(false);
        }
        let prefs = prefs.clone();
        self.tokens
            .run(move |state, backend| {
                Box::pin(async move {
                    backend.load_tokens(state).await?;
                    Ok(state
                        .grants
                        .iter()
                        .find(|c| c.value.grant_id == prefs.google_grant.id)
                        .is_some_and(|cached| {
                            cached.value.client_id == prefs.active_google_client()
                                && !cached.invalidated
                                && !cached.pending_save
                        }))
                })
            })
            .await
    }
    #[cfg(test)]
    pub async fn login(&self, prefs: &Preferences) -> anyhow::Result<crate::model::GoogleGrant> {
        self.login_with_retry(prefs, true, &CancellationToken::new())
            .await
    }
    /// System-browser sign-in with a loopback redirect, PKCE and exact state.
    /// Cancelling only stops the browser wait; a received code always completes.
    pub(crate) async fn login_with_retry(
        &self,
        prefs: &Preferences,
        retry: bool,
        cancel: &CancellationToken,
    ) -> anyhow::Result<crate::model::GoogleGrant> {
        let client = self
            .sign_in
            .client
            .as_ref()
            .ok_or(SignInError::NotConfigured)?;
        consent::requested_scopes(prefs)?;
        // Authorization codes are single-use. Finish a pending keychain save
        // without exchanging a received code again or opening another browser.
        if retry && let Some(grant) = self.finish_pending_login(prefs).await? {
            return Ok(grant);
        }
        if cancel.is_cancelled() {
            return Err(SignInError::Cancelled.into());
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let redirect = format!(
            "http://127.0.0.1:{}/callback",
            listener.local_addr()?.port()
        );
        let pkce = consent::Pkce::new();
        let state = random();
        let url = consent::authorization_url(
            &client.id,
            prefs,
            &redirect,
            &state,
            &pkce.challenge,
            retry,
        )?;
        let browser = self.sign_in.browser.clone();
        let link = url.to_string();
        let browser_wait = tokio::time::timeout(self.sign_in.wait, async {
            tokio::task::spawn_blocking(move || browser.open(&link)).await??;
            callback::receive(listener, &state).await
        });
        // Dropping the wait closes the loopback listener.
        let code = tokio::select! {
            biased;
            received = browser_wait => received.map_err(|_| SignInError::TimedOut)??,
            () = cancel.cancelled() => return Err(SignInError::Cancelled.into()),
        };
        self.exchange_code(prefs, code.expose_secret(), &redirect, &pkce.verifier)
            .await
    }
    #[cfg(test)]
    pub async fn token(&self, prefs: &Preferences) -> anyhow::Result<SecretString> {
        self.access_token(prefs, None).await
    }
    pub(crate) async fn token_for(
        &self,
        prefs: &Preferences,
        service: Service,
    ) -> anyhow::Result<SecretString> {
        self.access_token(prefs, Some(service)).await
    }
    async fn access_token(
        &self,
        prefs: &Preferences,
        service: Option<Service>,
    ) -> anyhow::Result<SecretString> {
        anyhow::ensure!(
            !prefs.google_lifecycle.disconnected,
            "Google is disconnected on this device. Reconnect in Preferences."
        );
        anyhow::ensure!(
            !prefs.active_google_client().trim().is_empty(),
            "Connect Google in Preferences before syncing or backing up to Drive."
        );
        let prefs = prefs.clone();
        self.tokens
            .run(move |state, backend| {
                Box::pin(async move { backend.access_token(state, &prefs, service).await })
            })
            .await
    }
    /// The actual granted access of one cached grant, as the owner sees it.
    async fn cached_access(
        &self,
        grant_id: String,
        missing: &'static str,
    ) -> anyhow::Result<crate::model::GoogleAccess> {
        self.tokens
            .run(move |state, _| {
                Box::pin(async move {
                    state
                        .grants
                        .iter()
                        .find(|c| c.value.grant_id == grant_id)
                        .map(|cached| cached.value.access())
                        .context(missing)
                })
            })
            .await
    }
}

impl Google {
    pub(crate) async fn prepare_grant(
        &self,
        prefs: &Preferences,
        grant: crate::model::GoogleGrant,
    ) -> anyhow::Result<(
        crate::model::GoogleGrant,
        Option<String>,
        Vec<crate::model::CalendarSource>,
    )> {
        let mut authorized = prefs.clone();
        authorized.google_lifecycle.disconnected = false;
        authorized.google_grant = grant;
        // A resumed candidate can have expired. Re-evaluate the actual scopes
        // after refresh, before deciding which services to validate.
        self.access_token(&authorized, None).await?;
        authorized.google_grant.access = self
            .cached_access(
                authorized.google_grant.id.clone(),
                "The staged Google connection is missing. Start a new sign-in.",
            )
            .await?;
        let access = authorized.google_grant.access;
        anyhow::ensure!(
            access.drive || access.calendar_read,
            "Google did not grant Calendar or Drive access. Start a new sign-in and approve access."
        );
        let identity = if access.drive {
            Some(
                crate::backup::DriveBackup::new(self.clone(), authorized.clone())
                    .account_identity()
                    .await?,
            )
        } else {
            None
        };
        let sources = if access.calendar_read {
            self.calendars(&authorized).await?
        } else {
            Vec::new()
        };
        Ok((authorized.google_grant, identity, sources))
    }

    pub async fn clear_credentials(&self) -> anyhow::Result<()> {
        self.tokens
            .run(|state, backend| {
                Box::pin(async move {
                    state.grants.clear();
                    state.candidate_id = None;
                    state.pending_login = None;
                    state.disconnected = true;
                    backend.credentials.delete().await.map_err(|_| anyhow::anyhow!("Google is disconnected, but its saved credential could not be removed. Unlock the OS keychain and choose Retry Google cleanup in Preferences."))
                })
            })
            .await
    }
}
fn random() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

impl Google {
    pub async fn calendars(
        &self,
        prefs: &Preferences,
    ) -> anyhow::Result<Vec<crate::model::CalendarSource>> {
        let token = self.token_for(prefs, Service::CalendarRead).await?;
        let mut sources = super::calendar::google_sources(
            &self.http,
            self.api_base.join("calendar/v3/users/me/calendarList")?,
            token.expose_secret(),
        )
        .await?;
        let actual_access = self
            .cached_access(
                prefs.google_grant.id.clone(),
                "Google changed while listing calendars. Try syncing again.",
            )
            .await?;
        if !prefs.google_grant.access.calendar_write_allowed()
            || !actual_access.calendar_write_allowed()
        {
            for source in &mut sources {
                source.access = crate::model::CalendarAccess::READ_ONLY;
            }
        }
        Ok(sources)
    }
}
