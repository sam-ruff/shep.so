use crate::{Event, ProviderFailure, Source};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};

mod restoration;
use crate::restoration::{DeletePlan, DeleteReceipt, Inspection, RestoreRequest};

#[async_trait]
#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
pub trait CalendarProvider: Send + Sync {
    async fn sources(&self, token: &str) -> Result<Vec<Source>, ProviderFailure>;
    async fn events(
        &self,
        token: &str,
        source: &Source,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Event>, ProviderFailure>;
    async fn save(
        &self,
        token: &str,
        request_id: &str,
        event: &Event,
    ) -> Result<Event, ProviderFailure>;
    async fn read(&self, token: &str, event: &Event) -> Result<Option<Event>, ProviderFailure>;
    async fn delete(&self, token: &str, event: &Event) -> Result<(), ProviderFailure>;
    async fn prepare_delete(
        &self,
        _token: &str,
        _event: &Event,
    ) -> Result<DeletePlan, ProviderFailure> {
        Err(ProviderFailure::rejected(
            "This calendar does not support restorable deletion.",
        ))
    }
    async fn cancel_event(
        &self,
        _token: &str,
        _plan: &DeletePlan,
    ) -> Result<DeleteReceipt, ProviderFailure> {
        Err(ProviderFailure::rejected(
            "This calendar does not support restorable deletion.",
        ))
    }
    async fn restore_event(
        &self,
        _token: &str,
        _request: &RestoreRequest,
    ) -> Result<Event, ProviderFailure> {
        Err(ProviderFailure::rejected(
            "This calendar does not support event restoration.",
        ))
    }
    async fn inspect_deletion(
        &self,
        _token: &str,
        _plan: &DeletePlan,
    ) -> Result<Inspection, ProviderFailure> {
        Err(ProviderFailure::rejected(
            "This calendar does not support deletion inspection.",
        ))
    }
}

pub struct GoogleCalendarProvider {
    client: reqwest::Client,
    base: String,
}

impl GoogleCalendarProvider {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(45))
                .build()?,
            base: "https://www.googleapis.com/calendar/v3".into(),
        })
    }
}

fn google_event(value: &Value, source: &str) -> Result<Event, ProviderFailure> {
    let parse = |name: &str| {
        let value = value[name]["dateTime"]
            .as_str()
            .or_else(|| value[name]["date"].as_str())
            .ok_or_else(|| {
                ProviderFailure::uncertain(
                    "Google acknowledged the event but returned an incomplete identity.",
                )
            })?;
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|value| value.with_timezone(&chrono::Utc))
            .or_else(|_| {
                chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                    .map(|date| date.and_hms_opt(0, 0, 0).expect("midnight").and_utc())
            })
            .map_err(|_| {
                ProviderFailure::uncertain(
                    "Google acknowledged the event but returned an invalid date.",
                )
            })
    };
    let event = Event {
        id: value["id"]
            .as_str()
            .ok_or_else(|| {
                ProviderFailure::uncertain("Google acknowledged the event without its identity.")
            })?
            .into(),
        source_id: source.into(),
        title: value["summary"].as_str().unwrap_or("Untitled event").into(),
        start: parse("start")?,
        end: parse("end")?,
        location: value["location"].as_str().unwrap_or("").into(),
        description: value["description"].as_str().unwrap_or("").into(),
        all_day: value["start"]["date"].is_string(),
        etag: value["etag"].as_str().map(str::to_owned),
        remote_url: value["id"].as_str().map(str::to_owned),
    };
    if !event.is_bounded() {
        return Err(ProviderFailure::waiting(
            "Google returned oversized event metadata.",
        ));
    }
    Ok(event)
}

fn request_body(event: &Event) -> Value {
    let date = |value: chrono::DateTime<chrono::Utc>| {
        if event.all_day {
            json!({"date": value.format("%Y-%m-%d").to_string()})
        } else {
            json!({"dateTime": value.to_rfc3339()})
        }
    };
    json!({"summary":event.title,"location":event.location,"description":event.description,
        "start":date(event.start),"end":date(event.end)})
}

fn response_failure(status: reqwest::StatusCode, body: String) -> ProviderFailure {
    let _ = body;
    let message = format!("Google Calendar returned HTTP {status}.");
    if matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED
            | reqwest::StatusCode::FORBIDDEN
            | reqwest::StatusCode::TOO_MANY_REQUESTS
    ) {
        ProviderFailure::waiting(message)
    } else if status.is_client_error() && status != reqwest::StatusCode::REQUEST_TIMEOUT {
        ProviderFailure::rejected(message)
    } else {
        ProviderFailure::uncertain(message)
    }
}

async fn bounded_text(mut response: reqwest::Response) -> Result<String, ProviderFailure> {
    const LIMIT: usize = 16 * 1024 * 1024;
    if response
        .content_length()
        .is_some_and(|length| length > LIMIT as u64)
    {
        return Err(ProviderFailure::waiting(
            "Google returned a calendar response larger than 16 MiB.",
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ProviderFailure::waiting("Could not read the Google Calendar response."))?
    {
        if bytes.len() + chunk.len() > LIMIT {
            return Err(ProviderFailure::waiting(
                "Google returned a calendar response larger than 16 MiB.",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map_err(|_| ProviderFailure::waiting("Google returned an unreadable calendar response."))
}

#[async_trait]
impl CalendarProvider for GoogleCalendarProvider {
    async fn prepare_delete(
        &self,
        token: &str,
        event: &Event,
    ) -> Result<DeletePlan, ProviderFailure> {
        self.prepare_google_delete(token, event).await
    }
    async fn cancel_event(
        &self,
        token: &str,
        plan: &DeletePlan,
    ) -> Result<DeleteReceipt, ProviderFailure> {
        self.cancel_google_event(token, plan).await
    }
    async fn restore_event(
        &self,
        token: &str,
        request: &RestoreRequest,
    ) -> Result<Event, ProviderFailure> {
        self.restore_google_event(token, request).await
    }
    async fn inspect_deletion(
        &self,
        token: &str,
        plan: &DeletePlan,
    ) -> Result<Inspection, ProviderFailure> {
        self.inspect_google_deletion(token, plan).await
    }
    async fn sources(&self, token: &str) -> Result<Vec<Source>, ProviderFailure> {
        let mut sources = Vec::new();
        let mut page = String::new();
        for _ in 0..20 {
            let response = self
                .client
                .get(format!("{}/users/me/calendarList", self.base))
                .bearer_auth(token)
                .query(&[("maxResults", "250"), ("pageToken", page.as_str())])
                .send()
                .await
                .map_err(|_| {
                    ProviderFailure::waiting("Could not load Google calendars. Retry when online.")
                })?;
            let status = response.status();
            let text = bounded_text(response).await?;
            if !status.is_success() {
                return Err(response_failure(status, text));
            }
            let value: Value = serde_json::from_str(&text).map_err(|_| {
                ProviderFailure::waiting("Google returned an unreadable calendar list.")
            })?;
            for item in value["items"].as_array().into_iter().flatten() {
                if let Some(id) = item["id"].as_str() {
                    let name = item["summary"].as_str().unwrap_or(id);
                    if id.len() > 1024 || name.len() > 1024 {
                        return Err(ProviderFailure::waiting(
                            "Google returned an oversized calendar identity.",
                        ));
                    }
                    sources.push(Source {
                        id: id.into(),
                        name: name.into(),
                        read_only: !item["accessRole"]
                            .as_str()
                            .is_some_and(|role| matches!(role, "writer" | "owner")),
                    });
                }
                if sources.len() > 50 {
                    return Err(ProviderFailure::waiting(
                        "This account has more than 50 calendars. Choose a smaller calendar set in Google Calendar.",
                    ));
                }
            }
            match value["nextPageToken"]
                .as_str()
                .filter(|value| !value.is_empty())
            {
                Some(next) => page = next.into(),
                None => return Ok(sources),
            }
        }
        Err(ProviderFailure::waiting(
            "Google returned too many calendar pages.",
        ))
    }

    async fn events(
        &self,
        token: &str,
        source: &Source,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Event>, ProviderFailure> {
        let mut events = Vec::new();
        let mut page = String::new();
        for _ in 0..20 {
            let response = self
                .client
                .get(format!(
                    "{}/calendars/{}/events",
                    self.base,
                    urlencoding::encode(&source.id)
                ))
                .bearer_auth(token)
                .query(&[
                    ("timeMin", start.to_rfc3339()),
                    ("timeMax", end.to_rfc3339()),
                    ("singleEvents", "true".into()),
                    ("orderBy", "startTime".into()),
                    ("maxResults", "250".into()),
                    ("pageToken", page.clone()),
                ])
                .send()
                .await
                .map_err(|_| {
                    ProviderFailure::waiting("Could not load Google events. Retry when online.")
                })?;
            let status = response.status();
            let text = bounded_text(response).await?;
            if !status.is_success() {
                return Err(response_failure(status, text));
            }
            let value: Value = serde_json::from_str(&text)
                .map_err(|_| ProviderFailure::waiting("Google returned unreadable events."))?;
            for item in value["items"].as_array().into_iter().flatten() {
                if item["status"] != "cancelled" {
                    events.push(google_event(item, &source.id)?);
                    if events.len() > 5000 {
                        return Err(ProviderFailure::waiting(
                            "This sync window contains more than 5,000 events.",
                        ));
                    }
                }
            }
            match value["nextPageToken"]
                .as_str()
                .filter(|value| !value.is_empty())
            {
                Some(next) => page = next.into(),
                None => return Ok(events),
            }
        }
        Err(ProviderFailure::waiting(
            "Google returned too many event pages.",
        ))
    }

    async fn save(
        &self,
        token: &str,
        request_id: &str,
        event: &Event,
    ) -> Result<Event, ProviderFailure> {
        let calendar = urlencoding::encode(&event.source_id);
        let creating = event.is_create();
        let id = if creating {
            format!("shep{}", request_id.replace('-', ""))
        } else {
            event.id.clone()
        };
        let mut body = request_body(event);
        let request = if creating {
            body["id"] = json!(id);
            body["extendedProperties"] = json!({"private":{"shepCreateId":request_id}});
            self.client
                .post(format!("{}/calendars/{calendar}/events", self.base))
        } else {
            let etag = event
                .etag
                .as_deref()
                .ok_or_else(|| ProviderFailure::rejected("Sync this event before editing it."))?;
            self.client
                .patch(format!(
                    "{}/calendars/{calendar}/events/{}",
                    self.base,
                    urlencoding::encode(&id)
                ))
                .header("If-Match", etag)
        };
        let response = request
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|error| {
                if error.is_builder() {
                    ProviderFailure::rejected(error.to_string())
                } else {
                    ProviderFailure::uncertain(
                        "The event may have reached Google. Inspect it before retrying.",
                    )
                }
            })?;
        let status = response.status();
        let text = bounded_text(response).await.map_err(|_| {
            ProviderFailure::uncertain("Google acknowledged the event but its response was lost.")
        })?;
        if !status.is_success() {
            return Err(response_failure(status, text));
        }
        serde_json::from_str(&text)
            .map_err(|_| {
                ProviderFailure::uncertain(
                    "Google acknowledged the event but returned an unreadable response.",
                )
            })
            .and_then(|value| google_event(&value, &event.source_id))
            .map_err(|_| ProviderFailure::uncertain("Google acknowledged the event but its returned metadata could not be confirmed."))
            .and_then(|saved| {
                if saved.id != id || saved.etag.as_deref().is_none_or(str::is_empty) {
                    return Err(ProviderFailure::uncertain("Google acknowledged the event without a confirmed identity and version. Check the saved request before continuing."));
                }
                Ok(saved)
            })
    }

    async fn read(&self, token: &str, event: &Event) -> Result<Option<Event>, ProviderFailure> {
        let response = self
            .client
            .get(format!(
                "{}/calendars/{}/events/{}",
                self.base,
                urlencoding::encode(&event.source_id),
                urlencoding::encode(&event.id)
            ))
            .bearer_auth(token)
            .send()
            .await
            .map_err(|_| {
                ProviderFailure::waiting("Could not inspect Google Calendar. Retry when online.")
            })?;
        if matches!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
        ) {
            return Ok(None);
        }
        let status = response.status();
        let text = bounded_text(response).await?;
        if !status.is_success() {
            return Err(response_failure(status, text));
        }
        let value = serde_json::from_str(&text)
            .map_err(|_| ProviderFailure::waiting("Google returned an unreadable event."))?;
        let current = google_event(&value, &event.source_id)?;
        if current.id != event.id || current.etag.as_deref().is_none_or(str::is_empty) {
            return Err(ProviderFailure::waiting(
                "Google returned an unconfirmed event identity. Keep the saved change and inspect again.",
            ));
        }
        Ok(Some(current))
    }

    async fn delete(&self, token: &str, event: &Event) -> Result<(), ProviderFailure> {
        let etag = event
            .etag
            .as_deref()
            .ok_or_else(|| ProviderFailure::rejected("Sync this event before deleting it."))?;
        let response = self
            .client
            .delete(format!(
                "{}/calendars/{}/events/{}",
                self.base,
                urlencoding::encode(&event.source_id),
                urlencoding::encode(&event.id)
            ))
            .bearer_auth(token)
            .header("If-Match", etag)
            .send()
            .await
            .map_err(|error| {
                if error.is_builder() {
                    ProviderFailure::rejected(error.to_string())
                } else {
                    ProviderFailure::uncertain(
                        "The delete may have reached Google. Inspect it before retrying.",
                    )
                }
            })?;
        if matches!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
        ) {
            return Ok(());
        }
        let status = response.status();
        let text = bounded_text(response).await.unwrap_or_default();
        if status.is_success() {
            Ok(())
        } else {
            Err(response_failure(status, text))
        }
    }
}

#[cfg(test)]
mod tests;
