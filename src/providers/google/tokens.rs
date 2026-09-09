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
    pub(super) credentials: crate::credentials::Credentials,
}

#[async_trait]
impl CredentialStore for OsCredentialStore {
    async fn read(&self) -> anyhow::Result<Option<SecretString>> {
        self.credentials.read_optional("google-oauth").await
    }
    async fn write(&self, secret: SecretString) -> anyhow::Result<()> {
        self.credentials.write("google-oauth", secret).await
    }
    async fn delete(&self) -> anyhow::Result<()> {
        self.credentials.delete("google-oauth").await
    }
}

#[derive(Default)]
pub(super) struct State {
    pub grants: Vec<Cached>,
    pub loaded: bool,
    pub candidate_id: Option<String>,
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
    pub grant_id: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64,
    #[serde(default)]
    pub scope: Option<String>,
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
            grant_id: previous
                .map(|old| old.grant_id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            client_id: previous
                .map(|t| t.client_id.clone())
                .unwrap_or_else(|| prefs.google_client_id.clone()),
            client_secret: previous
                .filter(|t| t.client_id != prefs.google_client_id)
                .map(|t| t.client_secret.clone())
                .unwrap_or_else(|| prefs.google_client_secret.clone()),
            access_token: access,
            refresh_token: refresh,
            expires_at: chrono::Utc::now().timestamp() + lifetime as i64,
            scope: reply
                .scope
                .take()
                .or_else(|| previous.and_then(|old| old.scope.clone()))
                .or_else(|| previous.is_none().then(|| SCOPES.to_string())),
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
        if state.disconnected || state.loaded {
            return Ok(());
        }
        let Some(secret) = self.credentials.read().await.map_err(|_| {
            anyhow::anyhow!(
                "Could not read the saved Google connection. Unlock the OS keychain and try again."
            )
        })?
        else {
            return Ok(());
        };
        anyhow::ensure!(
            secret.expose_secret().len() <= 256 * 1024,
            "The saved Google connection is too large. Reconnect Google in Preferences."
        );
        // Accept the original single-grant keychain entry without rewriting it.
        // Deserializer diagnostics must never quote credential contents.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Saved {
            Vault {
                grants: Vec<Tokens>,
                #[serde(default)]
                candidate_id: Option<String>,
            },
            Legacy(Tokens),
        }
        let saved: Saved = serde_json::from_str(secret.expose_secret()).map_err(|_| {
            anyhow::anyhow!(
                "The saved Google connection is damaged. Reconnect Google in Preferences."
            )
        })?;
        let (values, candidate_id) = match saved {
            Saved::Vault {
                grants,
                candidate_id,
            } => (grants, candidate_id),
            Saved::Legacy(value) => (vec![value], None),
        };
        anyhow::ensure!(
            values.len() <= 2,
            "The saved Google connection contains too many grants."
        );
        let mut ids = std::collections::HashSet::new();
        for value in &values {
            value.validate_saved()?;
            anyhow::ensure!(
                ids.insert(&value.grant_id),
                "The saved Google connection has duplicate grants."
            );
        }
        state.candidate_id = candidate_id;
        state.grants = values
            .into_iter()
            .map(|value| Cached {
                value,
                pending_save: false,
                invalidated: false,
            })
            .collect();
        state.loaded = true;
        Ok(())
    }

    async fn write_grants(
        &self,
        values: &[&Tokens],
        candidate_id: Option<&str>,
    ) -> anyhow::Result<()> {
        #[derive(Serialize)]
        struct Vault<'a> {
            grants: &'a [&'a Tokens],
            candidate_id: Option<&'a str>,
        }
        self.credentials
            .write(SecretString::from(serde_json::to_string(&Vault {
                grants: values,
                candidate_id,
            })?))
            .await
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

    pub(super) async fn persist_refresh(
        &self,
        state: &mut State,
        index: usize,
    ) -> anyhow::Result<()> {
        self.write_grants(&state.grants.iter().map(|c| &c.value).collect::<Vec<_>>(), state.candidate_id.as_deref()).await
            .map_err(|_| anyhow::anyhow!("Google access was renewed, but its credentials could not be saved to the OS keychain. Keep Shep open, unlock the keychain and try Sync again."))?;
        state.grants[index].pending_save = false;
        Ok(())
    }

    pub(super) async fn exchange_code(
        &self,
        prefs: &Preferences,
        code: &str,
        redirect: &str,
        verifier: &str,
    ) -> anyhow::Result<crate::model::GoogleGrant> {
        let mut state = self.state.lock().await;
        self.load_tokens(&mut state).await?;
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
        let candidate = Tokens::from_reply(prefs, reply, None)?;
        let access = scopes::access(candidate.scope.as_deref());
        anyhow::ensure!(
            access.drive || access.calendar_read,
            "Google did not grant usable Calendar or Drive access. Connect again and approve Calendar (including its list) or Drive backup."
        );
        state.pending_login = Some(candidate);
        self.stage_login(&mut state, prefs).await
    }

    pub(super) async fn finish_pending_login(
        &self,
        prefs: &Preferences,
    ) -> anyhow::Result<Option<crate::model::GoogleGrant>> {
        let mut state = self.state.lock().await;
        self.load_tokens(&mut state).await?;
        if state
            .pending_login
            .as_ref()
            .is_some_and(|t| t.client_id == prefs.google_client_id)
        {
            return self.stage_login(&mut state, prefs).await.map(Some);
        }
        // A crash before the SQLite activation leaves the candidate available for
        // retry. The current preference pointer still selects the working grant.
        Ok(state
            .grants
            .iter()
            .find(|c| {
                state.candidate_id.as_deref() == Some(c.value.grant_id.as_str())
                    && c.value.grant_id != prefs.google_grant.id
                    && c.value.client_id == prefs.google_client_id
                    && !c.invalidated
            })
            .map(|c| crate::model::GoogleGrant {
                id: c.value.grant_id.clone(),
                client_id: c.value.client_id.clone(),
                access: scopes::access(c.value.scope.as_deref()),
            }))
    }

    async fn stage_login(
        &self,
        state: &mut State,
        prefs: &Preferences,
    ) -> anyhow::Result<crate::model::GoogleGrant> {
        let candidate = state
            .pending_login
            .as_ref()
            .context("No pending Google sign-in")?;
        let mut values: Vec<&Tokens> = state
            .grants
            .iter()
            .filter(|c| c.value.grant_id == prefs.google_grant.id)
            .map(|c| &c.value)
            .collect();
        values.push(candidate);
        self.write_grants(&values, Some(&candidate.grant_id)).await.map_err(|_| anyhow::anyhow!(
            "Google authorization was received, but it could not be saved to the OS keychain. Keep Shep open, unlock the keychain and choose Reconnect Google to finish."))?;
        let grant = crate::model::GoogleGrant {
            id: candidate.grant_id.clone(),
            client_id: candidate.client_id.clone(),
            access: scopes::access(candidate.scope.as_deref()),
        };
        state.candidate_id = Some(grant.id.clone());
        state
            .grants
            .retain(|c| c.value.grant_id == prefs.google_grant.id);
        state.grants.push(Cached {
            value: state.pending_login.take().unwrap(),
            pending_save: false,
            invalidated: false,
        });
        state.disconnected = false;
        state.loaded = true;
        Ok(grant)
    }

    /// Only after the SQLite pointer commits may the previous credential be pruned.
    /// Failure is harmless to activation; the bounded vault still selects by ID.
    pub(crate) async fn finish_activation(&self, prefs: &Preferences) -> anyhow::Result<()> {
        let mut state = self.state.lock().await;
        self.load_tokens(&mut state).await?;
        let selected = state
            .grants
            .iter()
            .find(|c| c.value.grant_id == prefs.google_grant.id)
            .context("The selected Google grant is missing. Reconnect Google.")?;
        self.write_grants(&[&selected.value], None).await?;
        state
            .grants
            .retain(|c| c.value.grant_id == prefs.google_grant.id);
        state.candidate_id = None;
        state.pending_login = None;
        Ok(())
    }
}
