use crate::{
    model::*,
    providers::{CalendarProvider, google::Google},
};
use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub struct GoogleCalendar {
    pub google: Google,
    pub preferences: Preferences,
}

// Object-scoped transport permits protocol tests against a local HTTP server.
// The production entry point always uses the official endpoint and OS credentials.
struct Api<'a> {
    http: &'a reqwest::Client,
    base: url::Url,
    token: &'a str,
}

impl Api<'_> {
    fn url(&self, source: &CalendarSource, suffix: &str) -> anyhow::Result<url::Url> {
        let mut url = self.base.clone();
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Invalid API URL"))?;
        path.pop_if_empty().push(&source.url).push("events");
        if !suffix.is_empty() {
            path.push(suffix);
        }
        drop(path);
        Ok(url)
    }

    async fn events(
        &self,
        source: &CalendarSource,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> anyhow::Result<Vec<CalendarEvent>> {
        let mut events = Vec::new();
        let mut next = String::new();
        let mut seen = std::collections::HashSet::new();
        loop {
            anyhow::ensure!(
                seen.len() < 100 && seen.insert(next.clone()),
                "Google Calendar repeated a page token or returned too many pages. Try syncing again."
            );
            let data = super::response_json(
                self.http
                    .get(self.url(source, "")?)
                    .bearer_auth(self.token)
                    .query(&[
                        ("timeMin", start.to_rfc3339()),
                        ("timeMax", end.to_rfc3339()),
                        ("singleEvents", "true".into()),
                        ("orderBy", "startTime".into()),
                        ("maxResults", "250".into()),
                        ("pageToken", next),
                    ])
                    .send()
                    .await?
                    .error_for_status()?,
            )
            .await?;
            if let Some(items) = data["items"].as_array() {
                for item in items {
                    if item["status"] != "cancelled" {
                        events.push(parse_event(item, source)?);
                    }
                }
            }
            anyhow::ensure!(
                events.len() <= 5000,
                "This calendar has more than 5,000 events in the sync window."
            );
            match data["nextPageToken"].as_str().filter(|s| !s.is_empty()) {
                Some(token) => next = token.into(),
                None => return Ok(events),
            }
        }
    }

    async fn save(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
    ) -> anyhow::Result<CalendarEvent> {
        super::ensure_event_access(source, event, false)?;
        anyhow::ensure!(
            source.id == event.source_id,
            "This event belongs to another calendar."
        );
        let editing = event.etag.is_some() || event.remote_url.is_some();
        let id = if editing {
            event.id.clone()
        } else {
            format!("shep{:x}", Sha256::digest(event.key().as_bytes()))
        };
        let url = self.url(source, &id)?;
        let mut body = event_body(event);
        let request = if editing {
            let etag = event
                .etag
                .as_deref()
                .context("Sync calendar before editing this event again.")?;
            self.http.patch(url.clone()).header("If-Match", etag)
        } else {
            // Stable per-form identity makes an ambiguous POST safely retryable.
            body["id"] = json!(id);
            body["extendedProperties"] = json!({"private": {"shepCreateId": event.key()}});
            self.http.post(self.url(source, "")?)
        };
        let response = request.bearer_auth(self.token).json(&body).send().await?;
        if response.status() == reqwest::StatusCode::CONFLICT
            || response.status() == reqwest::StatusCode::PRECONDITION_FAILED
        {
            let data = super::response_json(
                self.http
                    .get(url)
                    .bearer_auth(self.token)
                    .send()
                    .await?
                    .error_for_status()?,
            )
            .await?;
            let saved = parse_event(&data, source)?;
            anyhow::ensure!(
                saved.id == id
                    && super::same_event_content(&saved, event)
                    && (editing
                        || data["extendedProperties"]["private"]["shepCreateId"].as_str()
                            == Some(event.key().as_str())),
                "This event changed on the server. Sync calendar before editing it again."
            );
            return Ok(saved);
        }
        let response = super::successful(response)?;
        // The server has committed at this point. A missing/malformed response body
        // requires a refresh, not another create operation.
        if let Ok(data) = super::response_json(response).await
            && let Ok(saved) = parse_event(&data, source)
            && saved.id == id
        {
            return Ok(saved);
        }
        let mut saved = event.clone();
        saved.id = id;
        saved.etag = None;
        saved.remote_url = Some(url.to_string());
        Ok(saved)
    }

    async fn delete(&self, source: &CalendarSource, event: &CalendarEvent) -> anyhow::Result<()> {
        super::ensure_event_access(source, event, true)?;
        anyhow::ensure!(
            source.id == event.source_id,
            "This event belongs to another calendar."
        );
        let etag = event
            .etag
            .as_deref()
            .context("Sync calendar before deleting this event.")?;
        let response = self
            .http
            .delete(self.url(source, &event.id)?)
            .bearer_auth(self.token)
            .header("If-Match", etag)
            .send()
            .await?;
        if !matches!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
        ) {
            super::successful(response)?;
        }
        Ok(())
    }
}

fn parse_event(item: &Value, source: &CalendarSource) -> anyhow::Result<CalendarEvent> {
    let parse = |key: &str| -> anyhow::Result<DateTime<Utc>> {
        if let Some(s) = item[key]["dateTime"].as_str() {
            Ok(DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc))
        } else {
            Ok(NaiveDate::parse_from_str(
                item[key]["date"].as_str().context("Missing event date")?,
                "%Y-%m-%d",
            )?
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc())
        }
    };
    Ok(CalendarEvent {
        id: item["id"]
            .as_str()
            .context("Missing Google event ID")?
            .into(),
        source_id: source.id.clone(),
        title: item["summary"].as_str().unwrap_or("Untitled event").into(),
        start: parse("start")?,
        end: parse("end")?,
        location: item["location"].as_str().unwrap_or("").into(),
        description: item["description"].as_str().unwrap_or("").into(),
        all_day: item["start"]["date"].is_string(),
        etag: item["etag"].as_str().map(str::to_owned),
        // Marks this as a remote event even if a nonconforming server omitted its ETag.
        remote_url: Some(item["id"].as_str().unwrap_or_default().into()),
    })
}

fn event_body(event: &CalendarEvent) -> Value {
    let date = |time: DateTime<Utc>| {
        if event.all_day {
            json!({"date": time.format("%Y-%m-%d").to_string()})
        } else {
            json!({"dateTime": time.to_rfc3339()})
        }
    };
    json!({"summary": event.title, "location": event.location, "description": event.description,
        "start": date(event.start), "end": date(event.end)})
}

#[async_trait]
impl CalendarProvider for GoogleCalendar {
    async fn events(
        &self,
        source: &CalendarSource,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> anyhow::Result<Vec<CalendarEvent>> {
        let token = self.google.token(&self.preferences).await?;
        Api {
            http: &self.google.http,
            base: url::Url::parse("https://www.googleapis.com/calendar/v3/calendars/")?,
            token: token.expose_secret(),
        }
        .events(source, start, end)
        .await
    }
    async fn save_event(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
    ) -> anyhow::Result<CalendarEvent> {
        let token = self.google.token(&self.preferences).await?;
        Api {
            http: &self.google.http,
            base: url::Url::parse("https://www.googleapis.com/calendar/v3/calendars/")?,
            token: token.expose_secret(),
        }
        .save(source, event)
        .await
    }
    async fn delete_event(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
    ) -> anyhow::Result<()> {
        let token = self.google.token(&self.preferences).await?;
        Api {
            http: &self.google.http,
            base: url::Url::parse("https://www.googleapis.com/calendar/v3/calendars/")?,
            token: token.expose_secret(),
        }
        .delete(source, event)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::calendar::test_server::{self, Reply, Server};

    fn remote(event: &CalendarEvent, id: &str, etag: &str) -> Value {
        let mut body = event_body(event);
        body["id"] = json!(id);
        body["etag"] = json!(etag);
        body["extendedProperties"] = json!({"private": {"shepCreateId": event.key()}});
        body
    }

    #[tokio::test]
    async fn google_calendar_recovers_ambiguous_create_and_uses_returned_etag_for_edit() {
        let event = test_server::event();
        let id = format!("shep{:x}", Sha256::digest(event.key().as_bytes()));
        let mut updated = event.clone();
        updated.title = "Later walk".into();
        let mut server = Server::start(vec![
            Reply::disconnect(),
            Reply::new(409, ""),
            Reply::new(200, remote(&event, &id, "\"v1\"").to_string()),
            Reply::new(200, remote(&updated, &id, "\"v2\"").to_string()),
        ])
        .await;
        let http = test_server::client();
        let api = Api {
            http: &http,
            base: server.url.clone(),
            token: "test-token",
        };
        let mut source = test_server::source(&server.url);
        source.url = "sam@example.com".into();
        assert!(api.save(&source, &event).await.is_err());
        let mut saved = api.save(&source, &event).await.unwrap();
        assert_eq!(saved.id, id);
        assert_eq!(saved.etag.as_deref(), Some("\"v1\""));
        saved.title = updated.title;
        let saved = api.save(&source, &saved).await.unwrap();
        assert_eq!(saved.etag.as_deref(), Some("\"v2\""));
        server.finish().await;
        let requests = server.requests();
        assert_eq!(
            requests
                .iter()
                .map(|r| r.method.as_str())
                .collect::<Vec<_>>(),
            ["POST", "POST", "GET", "PATCH"]
        );
        let first: Value = serde_json::from_str(&requests[0].body).unwrap();
        let retry: Value = serde_json::from_str(&requests[1].body).unwrap();
        assert_eq!(first["id"], retry["id"]);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='v').contains(&c))
        );
        assert!(requests[2].target.ends_with(&id));
        assert_eq!(requests[3].headers["if-match"], "\"v1\"");
        assert!(
            requests
                .iter()
                .all(|r| r.headers["authorization"] == "Bearer test-token")
        );
        let patch: Value = serde_json::from_str(&requests[3].body).unwrap();
        assert!(patch.get("attendees").is_none());
        assert!(patch.get("extendedProperties").is_none());
    }

    #[tokio::test]
    async fn google_calendar_acknowledges_commit_with_unreadable_body() {
        let mut server = Server::start(vec![Reply::new(201, "not-json")]).await;
        let http = test_server::client();
        let api = Api {
            http: &http,
            base: server.url.clone(),
            token: "test-token",
        };
        let source = test_server::source(&server.url);
        let event = test_server::event();
        let saved = api.save(&source, &event).await.unwrap();
        assert!(saved.remote_url.is_some());
        assert!(saved.etag.is_none());
        assert!(
            api.save(&source, &saved)
                .await
                .unwrap_err()
                .to_string()
                .contains("Sync calendar")
        );
        server.finish().await;
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn google_calendar_redirect_is_not_acknowledged_as_a_commit() {
        let mut server = Server::start(vec![
            Reply::new(302, "").header("Location", "https://other.example/event"),
        ])
        .await;
        let http = test_server::client();
        let api = Api {
            http: &http,
            base: server.url.clone(),
            token: "test-token",
        };
        assert!(
            api.save(&test_server::source(&server.url), &test_server::event())
                .await
                .is_err()
        );
        server.finish().await;
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn google_calendar_conflict_preserves_server_change() {
        let event = test_server::event();
        let id = format!("shep{:x}", Sha256::digest(event.key().as_bytes()));
        let mut changed = event.clone();
        changed.title = "Someone else's edit".into();
        let mut server = Server::start(vec![
            Reply::new(409, ""),
            Reply::new(200, remote(&changed, &id, "\"v2\"").to_string()),
        ])
        .await;
        let http = test_server::client();
        let api = Api {
            http: &http,
            base: server.url.clone(),
            token: "test-token",
        };
        let source = test_server::source(&server.url);
        assert!(
            api.save(&source, &event)
                .await
                .unwrap_err()
                .to_string()
                .contains("changed on the server")
        );
        server.finish().await;
        assert_eq!(server.requests().len(), 2);
    }

    #[tokio::test]
    async fn google_calendar_pages_events_and_rejects_repeated_tokens_and_large_responses() {
        let event = test_server::event();
        let first = json!({"items": [remote(&event, "first", "v1")], "nextPageToken": "page2"});
        let last = json!({"items": [{"status":"cancelled"}, remote(&event, "second", "v2")]});
        let mut server = Server::start(vec![
            Reply::new(200, first.to_string()),
            Reply::new(200, last.to_string()),
            Reply::new(200, first.to_string()),
            Reply::new(200, first.to_string()),
            Reply::new(200, "").header("Content-Length", "16777217"),
        ])
        .await;
        let http = test_server::client();
        let api = Api {
            http: &http,
            base: server.url.clone(),
            token: "test-token",
        };
        let source = test_server::source(&server.url);
        let events = api.events(&source, event.start, event.end).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].id, "second");
        assert!(
            api.events(&source, event.start, event.end)
                .await
                .unwrap_err()
                .to_string()
                .contains("page token")
        );
        assert!(
            api.events(&source, event.start, event.end)
                .await
                .unwrap_err()
                .to_string()
                .contains("too large")
        );
        server.finish().await;
        assert!(server.requests()[1].target.contains("pageToken=page2"));
    }

    #[tokio::test]
    async fn google_calendar_delete_is_conditional_and_already_absent_is_success() {
        let mut event = test_server::event();
        event.etag = Some("\"v1\"".into());
        let mut server = Server::start(vec![Reply::new(404, ""), Reply::new(412, "")]).await;
        let http = test_server::client();
        let api = Api {
            http: &http,
            base: server.url.clone(),
            token: "test-token",
        };
        let source = test_server::source(&server.url);
        api.delete(&source, &event).await.unwrap();
        assert!(api.delete(&source, &event).await.is_err());
        server.finish().await;
        assert!(
            server
                .requests()
                .iter()
                .all(|r| r.method == "DELETE" && r.headers["if-match"] == "\"v1\"")
        );
    }
}
