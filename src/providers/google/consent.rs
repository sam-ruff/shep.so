use crate::model::{GoogleCalendarRequest, Preferences};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub(super) fn requested_scopes(prefs: &Preferences) -> anyhow::Result<String> {
    let services = prefs.requested_google_services();
    anyhow::ensure!(
        services.any(),
        "Choose Drive backup or Calendar access in Preferences before signing in."
    );
    let mut scopes = Vec::with_capacity(3);
    if services.drive {
        scopes.push("https://www.googleapis.com/auth/drive.appdata");
    }
    match services.calendar {
        GoogleCalendarRequest::Off => {}
        GoogleCalendarRequest::ReadOnly => {
            scopes.push("https://www.googleapis.com/auth/calendar.events.readonly");
            scopes.push("https://www.googleapis.com/auth/calendar.calendarlist.readonly");
        }
        GoogleCalendarRequest::ReadWrite => {
            scopes.push("https://www.googleapis.com/auth/calendar.events");
            scopes.push("https://www.googleapis.com/auth/calendar.calendarlist.readonly");
        }
    }
    Ok(scopes.join(" "))
}

/// RFC 7636 proof key: a fresh 256-bit verifier and its S256 challenge.
pub(super) struct Pkce {
    pub(super) verifier: Zeroizing<String>,
    pub(super) challenge: String,
}
impl Pkce {
    pub(super) fn new() -> Self {
        let verifier = Zeroizing::new(super::random());
        Self {
            challenge: challenge(&verifier),
            verifier,
        }
    }
}
pub(super) fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub(super) fn authorization_url(
    client_id: &str,
    prefs: &Preferences,
    redirect: &str,
    state: &str,
    challenge: &str,
    retry: bool,
) -> anyhow::Result<url::Url> {
    let scopes = requested_scopes(prefs)?;
    let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", client_id),
        ("redirect_uri", redirect),
        ("response_type", "code"),
        ("scope", scopes.as_str()),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
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
    Ok(url)
}
