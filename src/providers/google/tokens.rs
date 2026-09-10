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
    #[serde(default)]
    pub requested_scopes: Option<String>,
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
        let requested_scopes = match previous {
            Some(old) => old.requested_scopes.clone(),
            None => Some(consent::requested_scopes(prefs)?),
        };
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
                .or_else(|| {
                    if previous.is_none() {
                        requested_scopes.clone()
                    } else {
                        None
                    }
                }),
            requested_scopes,
        })
    }
    pub(super) fn access(&self) -> crate::model::GoogleAccess {
        let mut access = scopes::access(self.scope.as_deref());
        if let Some(requested) = &self.requested_scopes {
            let selected = scopes::access(Some(requested));
            // Google may return an earlier broader grant. The newly activated
            // connection still enables only the services selected for this login.
            access.known = true;
            access.drive &= selected.drive;
            access.calendar_read &= selected.calendar_read;
            access.calendar_write &= selected.calendar_write;
        }
        access
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

impl Backend {
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

    /// Resolve a usable access token for the selected grant, renewing and
    /// persisting it under the owner so a rotated refresh token is never lost.
    pub(super) async fn access_token(
        &self,
        state: &mut State,
        prefs: &Preferences,
        service: Option<Service>,
    ) -> anyhow::Result<SecretString> {
        self.load_tokens(state).await?;
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
            service.check(cached.value.access())?;
        }
        // Retry a failed save before using or renewing a rotated credential.
        if cached.pending_save {
            self.persist_refresh(state, index).await?;
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
                        .downcast_ref::<ExchangeError>()
                        .is_some_and(|kind| matches!(kind, ExchangeError::InvalidGrant))
                    {
                        cached.invalidated = true;
                    }
                    return Err(error);
                }
            };
            cached.value = Tokens::from_reply(prefs, reply, Some(&cached.value))?;
            cached.pending_save = true;
            self.persist_refresh(state, index).await?;
        }
        let cached = &state.grants[index];
        if let Some(service) = service {
            service.check(cached.value.access())?;
        }
        Ok(SecretString::from(cached.value.access_token.clone()))
    }

    async fn exchange_code(
        &self,
        state: &mut State,
        prefs: &Preferences,
        code: &str,
        redirect: &str,
        verifier: &str,
    ) -> anyhow::Result<crate::model::GoogleGrant> {
        self.load_tokens(state).await?;
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
        let access = candidate.access();
        anyhow::ensure!(
            access.drive || access.calendar_read,
            "Google did not grant usable Calendar or Drive access. Connect again and approve Calendar (including its list) or Drive backup."
        );
        state.pending_login = Some(candidate);
        self.stage_login(state, prefs).await
    }

    async fn finish_pending_login(
        &self,
        state: &mut State,
        prefs: &Preferences,
        requested: &str,
    ) -> anyhow::Result<Option<crate::model::GoogleGrant>> {
        self.load_tokens(state).await?;
        if state.pending_login.as_ref().is_some_and(|t| {
            t.client_id == prefs.google_client_id
                && t.requested_scopes.as_deref() == Some(requested)
        }) {
            return self.stage_login(state, prefs).await.map(Some);
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
                    && c.value.requested_scopes.as_deref() == Some(requested)
                    && !c.invalidated
            })
            .map(|c| crate::model::GoogleGrant {
                id: c.value.grant_id.clone(),
                client_id: c.value.client_id.clone(),
                access: c.value.access(),
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
            access: candidate.access(),
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

    async fn finish_activation(&self, state: &mut State, grant_id: &str) -> anyhow::Result<()> {
        self.load_tokens(state).await?;
        let selected = state
            .grants
            .iter()
            .find(|c| c.value.grant_id == grant_id)
            .context("The selected Google grant is missing. Reconnect Google.")?;
        self.write_grants(&[&selected.value], None).await?;
        state.grants.retain(|c| c.value.grant_id == grant_id);
        state.candidate_id = None;
        state.pending_login = None;
        Ok(())
    }
}

impl Google {
    pub(super) async fn exchange_code(
        &self,
        prefs: &Preferences,
        code: &str,
        redirect: &str,
        verifier: &str,
    ) -> anyhow::Result<crate::model::GoogleGrant> {
        consent::requested_scopes(prefs)?;
        let prefs = prefs.clone();
        let code = Zeroizing::new(code.to_owned());
        let redirect = redirect.to_owned();
        let verifier = Zeroizing::new(verifier.to_owned());
        self.tokens
            .run(move |state, backend| {
                Box::pin(async move {
                    backend
                        .exchange_code(state, &prefs, &code, &redirect, &verifier)
                        .await
                })
            })
            .await
    }

    pub(super) async fn finish_pending_login(
        &self,
        prefs: &Preferences,
    ) -> anyhow::Result<Option<crate::model::GoogleGrant>> {
        let requested = consent::requested_scopes(prefs)?;
        let prefs = prefs.clone();
        self.tokens
            .run(move |state, backend| {
                Box::pin(async move {
                    backend
                        .finish_pending_login(state, &prefs, &requested)
                        .await
                })
            })
            .await
    }

    /// Only after the SQLite pointer commits may the previous credential be pruned.
    /// Failure is harmless to activation; the bounded vault still selects by ID.
    pub(crate) async fn finish_activation(&self, prefs: &Preferences) -> anyhow::Result<()> {
        let grant_id = prefs.google_grant.id.clone();
        self.tokens
            .run(move |state, backend| {
                Box::pin(async move { backend.finish_activation(state, &grant_id).await })
            })
            .await
    }
}
