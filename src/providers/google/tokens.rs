use super::*;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const RESPONSE_LIMIT: usize = 64 * 1024;

#[async_trait]
pub(super) trait CredentialStore: Send + Sync {
    async fn read(&self) -> anyhow::Result<Option<SecretString>>;
    async fn write(&self, secret: SecretString) -> anyhow::Result<()>;
    async fn delete(&self) -> anyhow::Result<()>;
}
#[derive(Default)]
pub(super) struct OsCredentialStore {
    writes: WriteQueue,
}

/// The blocking credential operation owns this FIFO guard. Cancelling the
/// caller cannot release it while the OS is still applying an older write.
#[derive(Default, Clone)]
pub(super) struct WriteQueue(Arc<Mutex<()>>);
impl WriteQueue {
    pub(super) async fn run(
        &self,
        operation: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
    ) -> anyhow::Result<()> {
        let guard = self.0.clone().lock_owned().await;
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            operation()
        })
        .await?
    }
}
#[async_trait]
impl CredentialStore for OsCredentialStore {
    async fn read(&self) -> anyhow::Result<Option<SecretString>> {
        tokio::task::spawn_blocking(|| {
            match keyring::Entry::new("so.shep.desktop", "google-oauth")?.get_password() {
                Ok(value) => Ok(Some(SecretString::from(value))),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(error.into()),
            }
        })
        .await?
    }
    async fn write(&self, secret: SecretString) -> anyhow::Result<()> {
        self.writes
            .run(move || {
                keyring::Entry::new("so.shep.desktop", "google-oauth")?
                    .set_password(secret.expose_secret())?;
                Ok(())
            })
            .await
    }
    async fn delete(&self) -> anyhow::Result<()> {
        self.writes
            .run(|| {
                match keyring::Entry::new("so.shep.desktop", "google-oauth")?.delete_credential() {
                    Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                    Err(error) => Err(error.into()),
                }
            })
            .await
    }
}

#[derive(Default)]
pub(super) struct State {
    pub active: Option<Cached>,
    pub pending_login: Option<Tokens>,
    pub disconnected: bool,
}
pub(super) struct Cached {
    pub value: Tokens,
    pub pending_save: bool,
    pub invalidated: bool,
}

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub(super) struct Tokens {
    #[serde(default)]
    pub client_id: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
pub(super) struct Reply {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
}

fn valid_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 16 * 1024 && value.bytes().all(|b| b.is_ascii_graphic())
}
impl Tokens {
    pub(super) fn from_reply(
        prefs: &Preferences,
        mut reply: Reply,
        previous: Option<&Self>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            reply
                .token_type
                .as_deref()
                .is_some_and(|kind| kind.eq_ignore_ascii_case("bearer")),
            "Google returned an unsupported token type. Reconnect Google in Preferences."
        );
        let access = reply
            .access_token
            .take()
            .filter(|s| valid_token(s))
            .context("Google returned an invalid access token. Try connecting again.")?;
        let lifetime = reply
            .expires_in
            .filter(|seconds| (1..=86400).contains(seconds))
            .context("Google returned an invalid token lifetime. Try connecting again.")?;
        let refresh = match reply.refresh_token.take() {
            Some(value) => {
                anyhow::ensure!(
                    valid_token(&value),
                    "Google returned an invalid refresh token. Reconnect Google in Preferences."
                );
                Some(value)
            }
            None => previous.and_then(|old| old.refresh_token.clone()),
        };
        // Never borrow the previous account's refresh token for a fresh login.
        anyhow::ensure!(
            refresh.is_some(),
            "Google did not grant offline access. Reconnect Google and approve access so Shep can stay connected."
        );
        Ok(Self {
            client_id: prefs.google_client_id.clone(),
            access_token: access,
            refresh_token: refresh,
            expires_at: chrono::Utc::now().timestamp() + lifetime as i64,
            scope: reply
                .scope
                .take()
                .or_else(|| previous.and_then(|old| old.scope.clone())),
        })
    }
    fn validate_saved(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            valid_token(&self.access_token)
                && self.refresh_token.as_deref().is_none_or(valid_token)
                && chrono::DateTime::from_timestamp(self.expires_at, 0).is_some(),
            "The saved Google connection is invalid. Reconnect Google in Preferences."
        );
        Ok(())
    }
}

#[derive(Debug)]
pub(super) enum ExchangeError {
    InvalidGrant,
    InvalidClient,
    Rejected(u16),
}
impl std::fmt::Display for ExchangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidGrant => f.write_str("Google access expired or was revoked. Reconnect Google in Preferences."),
            Self::InvalidClient => f.write_str("Google rejected the OAuth application. Check its desktop client ID and client secret in Preferences, then reconnect."),
            Self::Rejected(status) => write!(f, "Google could not authorize access (HTTP {status}). Try again; if it persists, reconnect Google in Preferences."),
        }
    }
}
impl std::error::Error for ExchangeError {}

impl Google {
    pub(super) async fn load_tokens(&self, state: &mut State) -> anyhow::Result<()> {
        if state.disconnected {
            return Ok(());
        }
        if state.active.is_none() {
            let Some(secret) = self.credentials.read().await.map_err(|_| anyhow::anyhow!(
                "Could not read the saved Google connection. Unlock the OS keychain and try again."
            ))?
            else {
                return Ok(());
            };
            anyhow::ensure!(
                secret.expose_secret().len() <= RESPONSE_LIMIT,
                "The saved Google connection is too large. Reconnect Google in Preferences."
            );
            // Deserializer errors can quote input strings, including credentials.
            let value: Tokens = serde_json::from_str(secret.expose_secret()).map_err(|_| {
                anyhow::anyhow!(
                    "The saved Google connection is damaged. Reconnect Google in Preferences."
                )
            })?;
            value.validate_saved()?;
            state.active = Some(Cached {
                value,
                pending_save: false,
                invalidated: false,
            });
        }
        Ok(())
    }

    pub(super) async fn exchange(&self, form: &[(&str, &str)]) -> anyhow::Result<Reply> {
        let mut response = self
            .http
            .post(self.token_endpoint.clone())
            .form(form)
            .send()
            .await
            .context("Could not reach Google sign-in. Check your connection and try again.")?;
        let status = response.status();
        anyhow::ensure!(
            response
                .content_length()
                .is_none_or(|n| n <= RESPONSE_LIMIT as u64),
            "Google sign-in response exceeds the size limit."
        );
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response
            .chunk()
            .await
            .context("Google sign-in response was interrupted. Try again.")?
        {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= RESPONSE_LIMIT,
                "Google sign-in response exceeds the size limit."
            );
            bytes.extend_from_slice(&chunk);
        }
        // HTTP redirects are never successful token exchanges, even with JSON.
        if !status.is_success() {
            let code = serde_json::from_slice::<Reply>(&bytes)
                .ok()
                .and_then(|mut reply| reply.error.take());
            return Err(match code.as_deref() {
                Some("invalid_grant") => ExchangeError::InvalidGrant,
                Some("invalid_client" | "unauthorized_client") => ExchangeError::InvalidClient,
                _ => ExchangeError::Rejected(status.as_u16()),
            }
            .into());
        }
        let reply: Reply = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!("Google returned an unreadable sign-in response. Try again.")
        })?;
        anyhow::ensure!(
            reply.error.is_none(),
            "Google returned an invalid sign-in response. Reconnect Google in Preferences."
        );
        Ok(reply)
    }

    pub(super) async fn persist_refresh(&self, cached: &mut Cached) -> anyhow::Result<()> {
        self.credentials.write(SecretString::from(serde_json::to_string(&cached.value)?)).await
            .map_err(|_| anyhow::anyhow!("Google access was renewed, but its credentials could not be saved to the OS keychain. Keep Shep open, unlock the keychain and try Sync again."))?;
        cached.pending_save = false;
        Ok(())
    }

    pub(super) async fn exchange_code(
        &self,
        prefs: &Preferences,
        code: &str,
        redirect: &str,
        verifier: &str,
    ) -> anyhow::Result<()> {
        // Refresh and sign-in must use one lock through exchange and persistence.
        let mut state = self.state.lock().await;
        let mut form = vec![
            ("client_id", prefs.google_client_id.as_str()),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect),
            ("grant_type", "authorization_code"),
        ];
        if !prefs.google_client_secret.is_empty() {
            form.push(("client_secret", prefs.google_client_secret.as_str()));
        }
        let reply = self.exchange(&form).await?;
        state.pending_login = Some(Tokens::from_reply(prefs, reply, None)?);
        self.commit_login(&mut state).await
    }

    pub(super) async fn finish_pending_login(&self, prefs: &Preferences) -> anyhow::Result<bool> {
        let mut state = self.state.lock().await;
        if state
            .pending_login
            .as_ref()
            .is_some_and(|tokens| tokens.client_id == prefs.google_client_id)
        {
            self.commit_login(&mut state).await?;
            return Ok(true);
        }
        Ok(false)
    }
    async fn commit_login(&self, state: &mut State) -> anyhow::Result<()> {
        let candidate = state
            .pending_login
            .as_ref()
            .context("No pending Google sign-in")?;
        self.credentials.write(SecretString::from(serde_json::to_string(candidate)?)).await
            .map_err(|_| anyhow::anyhow!("Google authorization was received, but it could not be saved to the OS keychain. Keep Shep open, unlock the keychain and choose Reconnect Google to finish."))?;
        state.active = state.pending_login.take().map(|value| Cached {
            value,
            pending_save: false,
            invalidated: false,
        });
        state.disconnected = false;
        Ok(())
    }
}
