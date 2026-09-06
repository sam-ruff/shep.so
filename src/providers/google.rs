use crate::model::Preferences;
use anyhow::Context;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

mod callback;
mod scopes;
pub(crate) use scopes::Service;
#[cfg(test)]
mod tests;
mod tokens;
use tokens::{CredentialStore, OsCredentialStore, State};

const SCOPES: &str = "https://www.googleapis.com/auth/drive.appdata https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/calendar.calendarlist.readonly";

#[derive(Clone)]
pub struct Google {
    pub http: reqwest::Client,
    state: Arc<Mutex<State>>,
    credentials: Arc<dyn CredentialStore>,
    // Private, object-scoped endpoint injection for loopback protocol tests.
    token_endpoint: url::Url,
    pub(crate) api_base: url::Url,
}
impl Default for Google {
    fn default() -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(45))
                .connect_timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("valid HTTP configuration"),
            api_base: url::Url::parse("https://www.googleapis.com/")
                .expect("official Google API URL"),
            state: Default::default(),
            credentials: Arc::new(OsCredentialStore::default()),
            token_endpoint: url::Url::parse("https://oauth2.googleapis.com/token")
                .expect("official Google token endpoint"),
        }
    }
}
impl Google {
    pub async fn connected(&self, prefs: &Preferences) -> anyhow::Result<bool> {
        if prefs.active_google_client().trim().is_empty() || prefs.google_lifecycle.disconnected {
            return Ok(false);
        }
        let mut state = self.state.lock().await;
        self.load_tokens(&mut state).await?;
        Ok(state
            .grants
            .iter()
            .find(|c| c.value.grant_id == prefs.google_grant.id)
            .is_some_and(|cached| {
                cached.value.client_id == prefs.active_google_client()
                    && !cached.invalidated
                    && !cached.pending_save
            }))
    }
    #[cfg(test)]
    pub async fn login(&self, prefs: &Preferences) -> anyhow::Result<crate::model::GoogleGrant> {
        self.login_with_retry(prefs, true).await
    }
    pub(crate) async fn login_with_retry(
        &self,
        prefs: &Preferences,
        retry: bool,
    ) -> anyhow::Result<crate::model::GoogleGrant> {
        anyhow::ensure!(
            !prefs.google_client_id.trim().is_empty(),
            "Add your Google Desktop OAuth client ID in Preferences first."
        );
        // Authorization codes are single-use. Finish a pending keychain save
        // without exchanging a received code again or opening another browser.
        if retry && let Some(grant) = self.finish_pending_login(prefs).await? {
            return Ok(grant);
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let redirect = format!(
            "http://127.0.0.1:{}/callback",
            listener.local_addr()?.port()
        );
        let verifier = random();
        let state = random();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", prefs.google_client_id.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("response_type", "code"),
            ("scope", SCOPES),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.as_str()),
            ("access_type", "offline"),
            (
                "prompt",
                if retry {
                    "consent"
                } else {
                    "consent select_account"
                },
            ),
        ]);
        let link = url.to_string();
        tokio::task::spawn_blocking(move || webbrowser::open(&link)).await??;
        let code = tokio::time::timeout(
            Duration::from_secs(180),
            callback::receive(listener, &state),
        )
        .await
        .context("Google sign-in expired. Try again.")??;
        self.exchange_code(prefs, code.expose_secret(), &redirect, &verifier)
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
        let mut state = self.state.lock().await;
        self.load_tokens(&mut state).await?;
        let index = state
            .grants
            .iter()
            .position(|c| c.value.grant_id == prefs.google_grant.id)
            .context("Connect Google in Preferences first.")?;
        let cached = &state.grants[index];
        anyhow::ensure!(
            cached.value.client_id == prefs.active_google_client()
                && !cached.value.client_id.is_empty(),
            "Reconnect Google in Preferences to verify access for this OAuth application."
        );
        anyhow::ensure!(
            !cached.invalidated,
            "Google access expired or was revoked. Reconnect Google in Preferences."
        );
        if let Some(service) = service {
            service.check(scopes::access(cached.value.scope.as_deref()))?;
        }
        // Retry a failed save before using or renewing a rotated credential.
        if cached.pending_save {
            self.persist_refresh(&mut state, index).await?;
        }
        let cached = &mut state.grants[index];
        if cached.value.expires_at <= chrono::Utc::now().timestamp() + 60 {
            let refresh = cached
                .value
                .refresh_token
                .as_deref()
                .context("Reconnect Google in Preferences to renew access.")?;
            let mut form = vec![
                ("client_id", cached.value.client_id.as_str()),
                ("refresh_token", refresh),
                ("grant_type", "refresh_token"),
            ];
            let client_secret = if prefs.google_client_id == cached.value.client_id {
                &prefs.google_client_secret
            } else {
                &cached.value.client_secret
            };
            if !client_secret.is_empty() {
                form.push(("client_secret", client_secret.as_str()));
            }
            let reply = match self.exchange(&form).await {
                Ok(reply) => reply,
                Err(error) => {
                    if error
                        .downcast_ref::<tokens::ExchangeError>()
                        .is_some_and(|kind| matches!(kind, tokens::ExchangeError::InvalidGrant))
                    {
                        cached.invalidated = true;
                    }
                    return Err(error);
                }
            };
            cached.value = tokens::Tokens::from_reply(prefs, reply, Some(&cached.value))?;
            cached.pending_save = true;
            self.persist_refresh(&mut state, index).await?;
        }
        let cached = &state.grants[index];
        if let Some(service) = service {
            service.check(scopes::access(cached.value.scope.as_deref()))?;
        }
        Ok(SecretString::from(cached.value.access_token.clone()))
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
        {
            let state = self.state.lock().await;
            let cached = state
                .grants
                .iter()
                .find(|c| c.value.grant_id == authorized.google_grant.id)
                .context("The staged Google connection is missing. Start a new sign-in.")?;
            authorized.google_grant.access = scopes::access(cached.value.scope.as_deref());
        }
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
        let mut state = self.state.lock().await;
        state.grants.clear();
        state.candidate_id = None;
        state.pending_login = None;
        state.disconnected = true;
        self.credentials.delete().await.map_err(|_| anyhow::anyhow!("Google is disconnected, but its saved credential could not be removed. Unlock the OS keychain and choose Retry Google cleanup in Preferences."))
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
        let actual_access = {
            let state = self.state.lock().await;
            let cached = state
                .grants
                .iter()
                .find(|c| c.value.grant_id == prefs.google_grant.id)
                .context("Google changed while listing calendars. Try syncing again.")?;
            scopes::access(cached.value.scope.as_deref())
        };
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
