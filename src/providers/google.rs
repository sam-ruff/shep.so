use crate::model::Preferences;
use anyhow::Context;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

#[derive(Clone)]
pub struct Google {
    pub http: reqwest::Client,
    tokens: Arc<Mutex<Option<Tokens>>>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Tokens {
    #[serde(default)]
    client_id: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_at: i64,
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
            tokens: Default::default(),
        }
    }
}
impl Google {
    pub async fn connected(&self) -> bool {
        super::read_secret("google-oauth").await.is_ok()
    }
    pub async fn login(&self, prefs: &Preferences) -> anyhow::Result<()> {
        anyhow::ensure!(
            !prefs.google_client_id.trim().is_empty(),
            "Add your Google Desktop OAuth client ID in Preferences first."
        );
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
            ("client_id",prefs.google_client_id.as_str()),("redirect_uri",redirect.as_str()),("response_type","code"),
            ("scope","https://www.googleapis.com/auth/drive.appdata https://www.googleapis.com/auth/calendar.events https://www.googleapis.com/auth/calendar.calendarlist.readonly"),
            ("code_challenge",challenge.as_str()),("code_challenge_method","S256"),("state",state.as_str()),("access_type","offline"),("prompt","consent")]);
        let link = url.to_string();
        tokio::task::spawn_blocking(move || webbrowser::open(&link)).await??;
        let code=tokio::time::timeout(Duration::from_secs(180),async {
            loop {
                let(mut stream,_)=listener.accept().await?;
                let mut buf=vec![0u8;8192];
                let n=tokio::time::timeout(Duration::from_secs(5),stream.read(&mut buf)).await??;
                let request=String::from_utf8_lossy(&buf[..n]);
                let path=request.lines().next().and_then(|l|l.split_whitespace().nth(1)).unwrap_or("/");
                let url=url::Url::parse(&format!("http://127.0.0.1{path}"))?;
                let args:std::collections::HashMap<_,_>=url.query_pairs().into_owned().collect();
                if url.path()!="/callback" || args.get("state")!=Some(&state) {
                    stream.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nInvalid sign-in response.").await?;continue;
                }
                let code=args.get("code").cloned().context("Google sign-in was cancelled or denied")?;
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\nSign-in received. You can return to Shep.").await?;
                return Ok::<_,anyhow::Error>(code);
            }
        }).await.context("Google sign-in expired. Try again.")??;
        let mut form = vec![
            ("client_id", prefs.google_client_id.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
        ];
        if !prefs.google_client_secret.is_empty() {
            form.push(("client_secret", prefs.google_client_secret.as_str()));
        }
        let data: serde_json::Value = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&form)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let tokens = Tokens {
            client_id: prefs.google_client_id.clone(),
            access_token: data["access_token"]
                .as_str()
                .context("Google did not return an access token")?
                .into(),
            refresh_token: data["refresh_token"].as_str().map(str::to_owned),
            expires_at: chrono::Utc::now().timestamp()
                + data["expires_in"].as_i64().unwrap_or(3600),
        };
        super::write_secret(
            "google-oauth",
            SecretString::from(serde_json::to_string(&tokens)?),
        )
        .await?;
        *self.tokens.lock().await = Some(tokens);
        Ok(())
    }
    pub async fn token(&self, prefs: &Preferences) -> anyhow::Result<SecretString> {
        anyhow::ensure!(
            !prefs.google_client_id.trim().is_empty(),
            "Connect Google in Preferences before syncing or backing up to Drive."
        );
        let mut guard = self.tokens.lock().await;
        if guard.is_none() {
            let secret = super::read_secret("google-oauth")
                .await
                .context("Connect Google in Preferences first")?;
            *guard = Some(serde_json::from_str(secret.expose_secret())?);
        }
        let tokens = guard.as_mut().context("Google is not connected")?;
        anyhow::ensure!(
            tokens.client_id == prefs.google_client_id && !tokens.client_id.is_empty(),
            "Reconnect Google in Preferences to verify access for this OAuth application."
        );
        if tokens.expires_at < chrono::Utc::now().timestamp() + 60 {
            let refresh = tokens
                .refresh_token
                .as_deref()
                .context("Sign in to Google again to renew access")?;
            let mut form = vec![
                ("client_id", prefs.google_client_id.as_str()),
                ("refresh_token", refresh),
                ("grant_type", "refresh_token"),
            ];
            if !prefs.google_client_secret.is_empty() {
                form.push(("client_secret", prefs.google_client_secret.as_str()));
            }
            let data: serde_json::Value = self
                .http
                .post("https://oauth2.googleapis.com/token")
                .form(&form)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            tokens.access_token = data["access_token"]
                .as_str()
                .context("Google session expired; sign in again")?
                .into();
            tokens.expires_at =
                chrono::Utc::now().timestamp() + data["expires_in"].as_i64().unwrap_or(3600);
            super::write_secret(
                "google-oauth",
                SecretString::from(serde_json::to_string(tokens)?),
            )
            .await?;
        }
        Ok(SecretString::from(tokens.access_token.clone()))
    }
    pub async fn calendars(
        &self,
        prefs: &Preferences,
    ) -> anyhow::Result<Vec<crate::model::CalendarSource>> {
        let token = self.token(prefs).await?;
        let mut next = String::new();
        let mut sources = Vec::new();
        loop {
            let data: serde_json::Value = self
                .http
                .get("https://www.googleapis.com/calendar/v3/users/me/calendarList")
                .bearer_auth(token.expose_secret())
                .query(&[("maxResults", "250"), ("pageToken", next.as_str())])
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if let Some(items) = data["items"].as_array() {
                for item in items {
                    if let Some(id) = item["id"].as_str() {
                        sources.push(crate::model::CalendarSource {
                            id: format!("google:{id}"),
                            name: item["summary"].as_str().unwrap_or("Google Calendar").into(),
                            kind: crate::model::CalendarKind::Google,
                            url: id.into(),
                            username: String::new(),
                        });
                    }
                }
            }
            match data["nextPageToken"].as_str() {
                Some(n) => next = n.into(),
                None => break,
            }
        }
        Ok(sources)
    }
}
fn random() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cached_google_tokens_cannot_be_used_by_another_oauth_application() {
        let google = Google {
            http: crate::providers::test_http::client(),
            tokens: Arc::new(Mutex::new(Some(Tokens {
                client_id: "issuing-client".into(),
                access_token: "fixture-access".into(),
                refresh_token: None,
                expires_at: chrono::Utc::now().timestamp() + 3600,
            }))),
        };
        let mut prefs = Preferences {
            google_client_id: "other-client".into(),
            ..Default::default()
        };
        assert!(
            google
                .token(&prefs)
                .await
                .unwrap_err()
                .to_string()
                .contains("Reconnect Google")
        );
        prefs.google_client_id = "issuing-client".into();
        assert_eq!(
            google.token(&prefs).await.unwrap().expose_secret(),
            "fixture-access"
        );
        google
            .tokens
            .lock()
            .await
            .as_mut()
            .unwrap()
            .client_id
            .clear();
        assert!(
            google
                .token(&prefs)
                .await
                .unwrap_err()
                .to_string()
                .contains("Reconnect Google")
        );
        prefs.google_client_id.clear();
        assert!(
            google
                .token(&prefs)
                .await
                .unwrap_err()
                .to_string()
                .contains("Connect Google")
        );
    }
}
