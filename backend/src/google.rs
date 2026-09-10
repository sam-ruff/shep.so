use crate::config::Config;
use async_trait::async_trait;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Copy)]
pub struct VerificationFailed;
impl std::fmt::Display for VerificationFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Google verification failed")
    }
}
impl std::error::Error for VerificationFailed {}

#[derive(Clone)]
pub struct Identity {
    pub subject: String,
    pub email: String,
}
#[derive(Clone, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub email: String,
    pub email_verified: bool,
    pub nonce: String,
    pub aud: String,
    pub iss: String,
    pub exp: u64,
    pub azp: Option<String>,
}
#[derive(Deserialize)]
struct Key {
    kid: String,
    kty: String,
    n: String,
    e: String,
    alg: Option<String>,
    #[serde(rename = "use")]
    usage: Option<String>,
}
#[derive(Deserialize)]
struct KeySet {
    keys: Vec<Key>,
}
#[derive(Deserialize)]
struct Tokens {
    id_token: String,
}

#[async_trait]
pub trait LoginVerifier: Send + Sync {
    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<Identity, VerificationFailed>;
}
pub struct Google {
    config: Arc<Config>,
    client: reqwest::Client,
    keys: Mutex<(Instant, HashMap<String, DecodingKey>)>,
}
impl Google {
    pub fn new(config: Arc<Config>) -> Result<Self, reqwest::Error> {
        Ok(Self {
            config,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .https_only(true)
                .build()?,
            keys: Mutex::new((Instant::now() - Duration::from_secs(3601), HashMap::new())),
        })
    }
    async fn key(&self, kid: &str) -> Result<DecodingKey, VerificationFailed> {
        let mut cache = self.keys.lock().await;
        // Throttle unknown-kid refreshes as well as normal expiry. This cache
        // contains Google's public keys only, never any account/token data.
        if cache.0.elapsed() >= Duration::from_secs(3600)
            || (!cache.1.contains_key(kid) && cache.0.elapsed() >= Duration::from_secs(60))
        {
            let response = self
                .client
                .get("https://www.googleapis.com/oauth2/v3/certs")
                .send()
                .await
                .map_err(|_| VerificationFailed)?;
            let set: KeySet = bounded_json(response).await?;
            if set.keys.is_empty() || set.keys.len() > 20 {
                return Err(VerificationFailed);
            }
            let mut keys = HashMap::new();
            for key in set.keys {
                if key.kty != "RSA"
                    || key.alg.as_deref().is_some_and(|a| a != "RS256")
                    || key.usage.as_deref().is_some_and(|u| u != "sig")
                {
                    continue;
                }
                if key.kid.len() > 256 || keys.contains_key(&key.kid) {
                    return Err(VerificationFailed);
                }
                keys.insert(
                    key.kid,
                    DecodingKey::from_rsa_components(&key.n, &key.e)
                        .map_err(|_| VerificationFailed)?,
                );
            }
            *cache = (Instant::now(), keys);
        }
        cache.1.get(kid).cloned().ok_or(VerificationFailed)
    }
}
pub fn verify_token(
    token: &str,
    key: &DecodingKey,
    audience: &str,
    nonce: &str,
) -> Result<Identity, VerificationFailed> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[audience]);
    validation.set_issuer(&["https://accounts.google.com", "accounts.google.com"]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.validate_nbf = true;
    validation.leeway = 30;
    let c = decode::<Claims>(token, key, &validation)
        .map_err(|_| VerificationFailed)?
        .claims;
    if !c.email_verified
        || c.sub.is_empty()
        || c.sub.len() > 255
        || c.email.len() > 320
        || !c.email.contains('@')
        || c.email.contains(['\r', '\n', ' '])
        || !bool::from(c.nonce.as_bytes().ct_eq(nonce.as_bytes()))
        || c.azp.as_deref().is_some_and(|value| value != audience)
    {
        return Err(VerificationFailed);
    }
    Ok(Identity {
        subject: c.sub,
        email: c.email.to_lowercase(),
    })
}
#[async_trait]
impl LoginVerifier for Google {
    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<Identity, VerificationFailed> {
        let response = self
            .client
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("code", code),
                ("client_id", self.config.google_client_id.as_str()),
                ("client_secret", self.config.google_client_secret.as_str()),
                ("redirect_uri", self.config.callback().as_str()),
                ("grant_type", "authorization_code"),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .map_err(|_| VerificationFailed)?;
        let token: Tokens = bounded_json(response).await?;
        self.verify_id_token(&token.id_token, nonce).await
    }
}
impl Google {
    /// Verify a Google ID token against the cached signing keys, the configured
    /// client audience and the pending nonce.
    pub async fn verify_id_token(
        &self,
        id_token: &str,
        nonce: &str,
    ) -> Result<Identity, VerificationFailed> {
        if id_token.len() > 32 * 1024 {
            return Err(VerificationFailed);
        }
        let header = decode_header(id_token).map_err(|_| VerificationFailed)?;
        if header.alg != Algorithm::RS256 {
            return Err(VerificationFailed);
        }
        let key = self
            .key(header.kid.as_deref().ok_or(VerificationFailed)?)
            .await?;
        verify_token(id_token, &key, &self.config.google_client_id, nonce)
    }
}
async fn bounded_json<T: serde::de::DeserializeOwned>(
    mut response: reqwest::Response,
) -> Result<T, VerificationFailed> {
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > 65536) {
        return Err(VerificationFailed);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| VerificationFailed)? {
        if bytes.len() + chunk.len() > 65536 {
            return Err(VerificationFailed);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| VerificationFailed)
}
