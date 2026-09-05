use super::{CalendarProvider, google::Google};
use crate::model::*;
use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use secrecy::ExposeSecret;

pub struct GoogleCalendar {
    pub google: Google,
    pub preferences: Preferences,
}
pub struct CalDav {
    pub http: reqwest::Client,
}

fn google_url(source: &CalendarSource, suffix: &str) -> anyhow::Result<url::Url> {
    let mut url = url::Url::parse("https://www.googleapis.com/calendar/v3/calendars/")?;
    url.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid API URL"))?
        .pop_if_empty()
        .push(&source.url)
        .push("events");
    if !suffix.is_empty() {
        url.path_segments_mut().unwrap().push(suffix);
    }
    Ok(url)
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
        let mut events = Vec::new();
        let mut next = String::new();
        loop {
            let data: serde_json::Value = self
                .google
                .http
                .get(google_url(source, "")?)
                .bearer_auth(token.expose_secret())
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
                .error_for_status()?
                .json()
                .await?;
            if let Some(items) = data["items"].as_array() {
                for item in items {
                    if item["status"] == "cancelled" {
                        continue;
                    }
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
                    events.push(CalendarEvent {
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
                        remote_url: None,
                    });
                }
            }
            anyhow::ensure!(
                events.len() <= 5000,
                "This calendar has more than 5,000 events in the sync window."
            );
            match data["nextPageToken"].as_str() {
                Some(n) => next = n.into(),
                None => break,
            }
        }
        Ok(events)
    }
    async fn save_event(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
    ) -> anyhow::Result<()> {
        let token = self.google.token(&self.preferences).await?;
        let start = if event.all_day {
            serde_json::json!({"date":event.start.format("%Y-%m-%d").to_string()})
        } else {
            serde_json::json!({"dateTime":event.start.to_rfc3339()})
        };
        let end = if event.all_day {
            serde_json::json!({"date":event.end.format("%Y-%m-%d").to_string()})
        } else {
            serde_json::json!({"dateTime":event.end.to_rfc3339()})
        };
        let body = serde_json::json!({"summary":event.title,"location":event.location,"description":event.description,"start":start,"end":end});
        let request = if let Some(etag) = &event.etag {
            self.google
                .http
                .patch(google_url(source, &event.id)?)
                .header("If-Match", etag)
        } else {
            self.google.http.post(google_url(source, "")?)
        };
        request
            .bearer_auth(token.expose_secret())
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    async fn delete_event(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
    ) -> anyhow::Result<()> {
        let token = self.google.token(&self.preferences).await?;
        let mut request = self
            .google
            .http
            .delete(google_url(source, &event.id)?)
            .bearer_auth(token.expose_secret());
        if let Some(etag) = &event.etag {
            request = request.header("If-Match", etag);
        }
        request.send().await?.error_for_status()?;
        Ok(())
    }
}

pub fn validate_caldav_url(input: &str) -> anyhow::Result<url::Url> {
    let url = url::Url::parse(input).context("Enter the full CalDAV calendar collection URL")?;
    anyhow::ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))),
        "Use HTTPS for CalDAV. Plain HTTP is allowed only on localhost."
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Enter the CalDAV username and password in their separate fields."
    );
    Ok(url)
}
#[async_trait]
impl CalendarProvider for CalDav {
    async fn events(
        &self,
        source: &CalendarSource,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> anyhow::Result<Vec<CalendarEvent>> {
        let url = validate_caldav_url(&source.url)?;
        let secret = super::read_secret(&source.id).await?;
        let start = start.format("%Y%m%dT%H%M%SZ");
        let end = end.format("%Y%m%dT%H%M%SZ");
        let xml = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data><c:expand start="{start}" end="{end}"/></c:calendar-data></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{start}" end="{end}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#
        );
        let mut response = self
            .http
            .request(reqwest::Method::from_bytes(b"REPORT")?, url.clone())
            .basic_auth(&source.username, Some(secret.expose_secret()))
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(xml)
            .send()
            .await?
            .error_for_status()?;
        anyhow::ensure!(
            response.content_length().unwrap_or(0) <= 16 * 1024 * 1024,
            "Calendar response is too large."
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                bytes.len() + chunk.len() <= 16 * 1024 * 1024,
                "Calendar response is too large."
            );
            bytes.extend_from_slice(&chunk);
        }
        let text = String::from_utf8(bytes)?;
        anyhow::ensure!(
            text.len() <= 16 * 1024 * 1024,
            "Calendar response is too large."
        );
        let source = source.clone();
        tokio::task::spawn_blocking(move || parse_caldav(&text, &source, &url)).await?
    }
    async fn save_event(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
    ) -> anyhow::Result<()> {
        let base = validate_caldav_url(&source.url)?;
        let url = if let Some(remote) = &event.remote_url {
            base.join(remote)?
        } else {
            base.join(&format!("{}.ics", event.id))?
        };
        anyhow::ensure!(
            url.origin() == base.origin(),
            "Refusing to send calendar credentials to another server."
        );
        let secret = super::read_secret(&source.id).await?;
        let mut request = self
            .http
            .put(url)
            .basic_auth(&source.username, Some(secret.expose_secret()))
            .header("Content-Type", "text/calendar; charset=utf-8");
        request = if let Some(etag) = &event.etag {
            request.header("If-Match", etag)
        } else {
            request.header("If-None-Match", "*")
        };
        request
            .body(encode_ical(event))
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
    async fn delete_event(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
    ) -> anyhow::Result<()> {
        let base = validate_caldav_url(&source.url)?;
        let url = base.join(
            event
                .remote_url
                .as_deref()
                .context("Missing calendar resource URL")?,
        )?;
        anyhow::ensure!(
            url.origin() == base.origin(),
            "Refusing to send calendar credentials to another server."
        );
        let secret = super::read_secret(&source.id).await?;
        let mut request = self
            .http
            .delete(url)
            .basic_auth(&source.username, Some(secret.expose_secret()));
        if let Some(etag) = &event.etag {
            request = request.header("If-Match", etag);
        }
        request.send().await?.error_for_status()?;
        Ok(())
    }
}

pub fn parse_caldav(
    xml: &str,
    source: &CalendarSource,
    base: &url::Url,
) -> anyhow::Result<Vec<CalendarEvent>> {
    let document = roxmltree::Document::parse(xml)?;
    let mut out = Vec::new();
    for response in document
        .descendants()
        .filter(|n| n.has_tag_name(("DAV:", "response")))
    {
        let href = response
            .descendants()
            .find(|n| n.has_tag_name(("DAV:", "href")))
            .and_then(|n| n.text())
            .unwrap_or("");
        anyhow::ensure!(
            base.join(href)?.origin() == base.origin(),
            "CalDAV returned a resource on another server."
        );
        let etag = response
            .descendants()
            .find(|n| n.has_tag_name(("DAV:", "getetag")))
            .and_then(|n| n.text())
            .map(str::to_owned);
        let Some(data) = response
            .descendants()
            .find(|n| n.has_tag_name(("urn:ietf:params:xml:ns:caldav", "calendar-data")))
            .and_then(|n| n.text())
        else {
            continue;
        };
        for calendar in ical::IcalParser::new(std::io::BufReader::new(data.as_bytes())) {
            for event in calendar?.events {
                let value = |key: &str| {
                    event
                        .properties
                        .iter()
                        .find(|p| p.name == key)
                        .and_then(|p| p.value.clone())
                        .unwrap_or_default()
                };
                let start = event
                    .properties
                    .iter()
                    .find(|p| p.name == "DTSTART")
                    .context("Calendar event has no start")?;
                let begin = parse_ical_date(start)?;
                let end = event
                    .properties
                    .iter()
                    .find(|p| p.name == "DTEND")
                    .map(parse_ical_date)
                    .transpose()?
                    .unwrap_or(begin + chrono::Duration::hours(1));
                let uid = value("UID");
                // Expanded recurrence instances are shown but never overwrite an entire recurring resource.
                let recurrence = value("RECURRENCE-ID");
                let recurring =
                    !recurrence.is_empty() || event.properties.iter().any(|p| p.name == "RRULE");
                out.push(CalendarEvent {
                    id: if recurrence.is_empty() {
                        uid
                    } else {
                        format!("{uid}/{recurrence}")
                    },
                    source_id: source.id.clone(),
                    title: unescape(&value("SUMMARY")),
                    start: begin,
                    end,
                    location: unescape(&value("LOCATION")),
                    description: if recurring {
                        format!(
                            "Recurring event · edit the series on your calendar server.\n{}",
                            unescape(&value("DESCRIPTION"))
                        )
                    } else {
                        unescape(&value("DESCRIPTION"))
                    },
                    all_day: start.value.as_ref().is_some_and(|v| v.len() == 8),
                    etag: etag.clone(),
                    remote_url: if recurring { None } else { Some(href.into()) },
                });
            }
        }
    }
    Ok(out)
}
fn parse_ical_date(p: &ical::property::Property) -> anyhow::Result<DateTime<Utc>> {
    let value = p.value.as_deref().context("Empty calendar date")?;
    if value.len() == 8 {
        return Ok(NaiveDate::parse_from_str(value, "%Y%m%d")?
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc());
    }
    let date = NaiveDateTime::parse_from_str(value.trim_end_matches('Z'), "%Y%m%dT%H%M%S")?;
    if value.ends_with('Z') {
        return Ok(date.and_utc());
    }
    if let Some(zone) = p
        .params
        .as_ref()
        .and_then(|params| params.iter().find(|(k, _)| k == "TZID"))
        .and_then(|(_, v)| v.first())
    {
        let zone: chrono_tz::Tz = zone
            .parse()
            .map_err(|_| anyhow::anyhow!("Unsupported calendar timezone: {zone}"))?;
        return Ok(zone
            .from_local_datetime(&date)
            .earliest()
            .context("Invalid calendar local time")?
            .with_timezone(&Utc));
    }
    Ok(chrono::Local
        .from_local_datetime(&date)
        .earliest()
        .context("Invalid local time")?
        .with_timezone(&Utc))
}
fn unescape(s: &str) -> String {
    s.replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\r', "")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}
pub fn encode_ical(e: &CalendarEvent) -> String {
    let (start, end) = if e.all_day {
        (
            format!("DTSTART;VALUE=DATE:{}", e.start.format("%Y%m%d")),
            format!("DTEND;VALUE=DATE:{}", e.end.format("%Y%m%d")),
        )
    } else {
        (
            format!("DTSTART:{}", e.start.format("%Y%m%dT%H%M%SZ")),
            format!("DTEND:{}", e.end.format("%Y%m%dT%H%M%SZ")),
        )
    };
    let text = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Shep//Calendar 1.0//EN\r\nBEGIN:VEVENT\r\nUID:{}\r\nDTSTAMP:{}\r\n{start}\r\n{end}\r\nSUMMARY:{}\r\nLOCATION:{}\r\nDESCRIPTION:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        escape(&e.id),
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        escape(&e.title),
        escape(&e.location),
        escape(&e.description)
    );
    let mut folded = String::new();
    for line in text.split("\r\n").filter(|l| !l.is_empty()) {
        let mut width = 0;
        for ch in line.chars() {
            if width + ch.len_utf8() > 74 {
                folded.push_str("\r\n ");
                width = 1;
            }
            folded.push(ch);
            width += ch.len_utf8();
        }
        folded.push_str("\r\n");
    }
    folded
}
