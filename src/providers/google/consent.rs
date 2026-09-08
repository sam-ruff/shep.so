use crate::model::{GoogleCalendarRequest, Preferences};

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

pub(super) fn authorization_url(
    prefs: &Preferences,
    redirect: &str,
    state: &str,
    challenge: &str,
    retry: bool,
) -> anyhow::Result<url::Url> {
    let scopes = requested_scopes(prefs)?;
    let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", prefs.google_client_id.as_str()),
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
