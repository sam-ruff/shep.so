use crate::model::Preferences;
use anyhow::Context;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::sync::Mutex;

mod callback;
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
            state: Default::default(),
            credentials: Arc::new(OsCredentialStore::default()),
            token_endpoint: url::Url::parse("https://oauth2.googleapis.com/token")
                .expect("official Google token endpoint"),
        }
    }
}
impl Google {
    pub async fn connected(&self, prefs: &Preferences) -> anyhow::Result<bool> {
        if prefs.google_client_id.trim().is_empty() || prefs.google_lifecycle.disconnected {
            return Ok(false);
        }
        let mut state = self.state.lock().await;
        self.load_tokens(&mut state).await?;
        Ok(state.pending_login.is_none()
            && state.active.as_ref().is_some_and(|cached| {
                cached.value.client_id == prefs.google_client_id
                    && !cached.invalidated
                    && !cached.pending_save
            }))
    }
    pub async fn login(&self, prefs: &Preferences) -> anyhow::Result<()> {
        anyhow::ensure!(
            !prefs.google_client_id.trim().is_empty(),
            "Add your Google Desktop OAuth client ID in Preferences first."
        );
        // Authorization codes are single-use. Finish a pending keychain save
        // without exchanging a received code again or opening another browser.
        if self.finish_pending_login(prefs).await? {
            return Ok(());
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
            ("prompt", "consent"),
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
    pub async fn token(&self, prefs: &Preferences) -> anyhow::Result<SecretString> {
        anyhow::ensure!(
            !prefs.google_lifecycle.disconnected,
            "Google is disconnected on this device. Reconnect in Preferences."
        );
        anyhow::ensure!(
            !prefs.google_client_id.trim().is_empty(),
            "Connect Google in Preferences before syncing or backing up to Drive."
        );
        let mut state = self.state.lock().await;
        anyhow::ensure!(
            state.pending_login.is_none(),
            "Finish Google sign-in in Preferences before syncing. Unlock the OS keychain, then choose Reconnect Google."
        );
        self.load_tokens(&mut state).await?;
        let cached = state
            .active
            .as_mut()
            .context("Connect Google in Preferences first.")?;
        anyhow::ensure!(
            cached.value.client_id == prefs.google_client_id && !cached.value.client_id.is_empty(),
            "Reconnect Google in Preferences to verify access for this OAuth application."
        );
        anyhow::ensure!(
            !cached.invalidated,
            "Google access expired or was revoked. Reconnect Google in Preferences."
        );
        // Retry a failed save before using or renewing a rotated credential.
        if cached.pending_save {
            self.persist_refresh(cached).await?;
        }
        if cached.value.expires_at <= chrono::Utc::now().timestamp() + 60 {
            let refresh = cached
                .value
                .refresh_token
                .as_deref()
                .context("Reconnect Google in Preferences to renew access.")?;
            let mut form = vec![
                ("client_id", prefs.google_client_id.as_str()),
                ("refresh_token", refresh),
                ("grant_type", "refresh_token"),
            ];
            if !prefs.google_client_secret.is_empty() {
                form.push(("client_secret", prefs.google_client_secret.as_str()));
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
            self.persist_refresh(cached).await?;
        }
        Ok(SecretString::from(cached.value.access_token.clone()))
    }
}

impl Google {
    pub async fn clear_credentials(&self) -> anyhow::Result<()> {
        let mut state = self.state.lock().await;
        state.active = None;
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
        let token = self.token(prefs).await?;
        super::calendar::google_sources(
            &self.http,
            url::Url::parse("https://www.googleapis.com/calendar/v3/users/me/calendarList")?,
            token.expose_secret(),
        )
        .await
    }
}
