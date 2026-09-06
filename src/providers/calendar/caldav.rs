use super::{response_text, validate_caldav_url};
use crate::{
    model::*,
    providers::{self, CalendarProvider},
};
use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use secrecy::ExposeSecret;

pub struct CalDav {
    pub http: reqwest::Client,
}

impl CalDav {
    async fn fetch(
        &self,
        source: &CalendarSource,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        secret: &str,
    ) -> anyhow::Result<Vec<CalendarEvent>> {
        let url = validate_caldav_url(&source.url)?;
        let start = start.format("%Y%m%dT%H%M%SZ");
        let end = end.format("%Y%m%dT%H%M%SZ");
        let xml = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data><c:expand start="{start}" end="{end}"/></c:calendar-data></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{start}" end="{end}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#
        );
        let response = self
            .http
            .request(reqwest::Method::from_bytes(b"REPORT")?, url.clone())
            .basic_auth(&source.username, Some(secret))
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(xml)
            .send()
            .await?
            .error_for_status()?;
        let text = response_text(response).await?;
        let source = source.clone();
        tokio::task::spawn_blocking(move || super::parse_caldav(&text, &source, &url)).await?
    }

    async fn resource(
        &self,
        source: &CalendarSource,
        url: &url::Url,
        secret: &str,
    ) -> anyhow::Result<(CalendarEvent, String)> {
        let response = self
            .http
            .get(url.clone())
            .basic_auth(&source.username, Some(secret))
            .send()
            .await?
            .error_for_status()?;
        let etag = strong_etag(&response)
            .context("The calendar server did not return an ETag. Sync calendar before editing.")?;
        let data = response_text(response).await?;
        let mut events = super::parse_resource(&data, source, url.as_str(), Some(etag))?;
        anyhow::ensure!(
            events.len() == 1 && events[0].remote_url.is_some(),
            "Edit recurring events on your calendar server."
        );
        Ok((events.remove(0), data))
    }

    async fn save(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
        secret: &str,
    ) -> anyhow::Result<CalendarEvent> {
        anyhow::ensure!(
            source.id == event.source_id,
            "This event belongs to another calendar."
        );
        let base = validate_caldav_url(&source.url)?;
        let editing = event.remote_url.is_some() || event.etag.is_some();
        let url = if editing {
            base.join(
                event
                    .remote_url
                    .as_deref()
                    .context("Edit recurring events on your calendar server.")?,
            )?
        } else {
            let mut url = base.clone();
            url.path_segments_mut()
                .map_err(|_| anyhow::anyhow!("Invalid calendar URL"))?
                .pop_if_empty()
                .push(&format!("{}.ics", event.id));
            url
        };
        anyhow::ensure!(
            url.origin() == base.origin(),
            "Refusing to send calendar credentials to another server."
        );
        let (body, request) = if editing {
            let etag = event
                .etag
                .as_deref()
                .context("Sync calendar before editing this event again.")?;
            // REPORT expansion may omit data. Always edit the complete resource and
            // preserve alarms, attendees, timezones and unknown extension properties.
            let (current, original) = self.resource(source, &url, secret).await?;
            anyhow::ensure!(
                current.id == event.id,
                "The calendar resource belongs to another event."
            );
            if current.etag.as_deref() != Some(etag) {
                anyhow::ensure!(
                    super::same_event_content(&current, event),
                    "This event changed on the server. Sync calendar before editing it again."
                );
                return Ok(current);
            }
            (
                super::edit_ical(&original, &current, event)?,
                self.http.put(url.clone()).header("If-Match", etag),
            )
        } else {
            (
                super::encode_ical(event),
                self.http.put(url.clone()).header("If-None-Match", "*"),
            )
        };
        let response = request
            .basic_auth(&source.username, Some(secret))
            .header("Content-Type", "text/calendar; charset=utf-8")
            .body(body)
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::PRECONDITION_FAILED {
            let (current, _) = self.resource(source, &url, secret).await?;
            anyhow::ensure!(
                current.id == event.id && super::same_event_content(&current, event),
                "This event changed on the server. Sync calendar before editing it again."
            );
            return Ok(current);
        }
        let response = super::successful(response)?;
        let mut saved = event.clone();
        saved.etag = strong_etag(&response);
        saved.remote_url = Some(url.to_string());
        // PUT may legally omit ETag after normalizing the resource. The committed
        // event remains visible; require a sync before any further mutation.
        Ok(saved)
    }

    async fn delete(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
        secret: &str,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            source.id == event.source_id,
            "This event belongs to another calendar."
        );
        let base = validate_caldav_url(&source.url)?;
        let url = base.join(
            event
                .remote_url
                .as_deref()
                .context("Edit recurring events on your calendar server.")?,
        )?;
        anyhow::ensure!(
            url.origin() == base.origin(),
            "Refusing to send calendar credentials to another server."
        );
        let etag = event
            .etag
            .as_deref()
            .context("Sync calendar before deleting this event.")?;
        let response = self
            .http
            .delete(url)
            .basic_auth(&source.username, Some(secret))
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

fn strong_etag(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|h| h.to_str().ok())
        .filter(|s| s.starts_with('"') && s.ends_with('"'))
        .map(str::to_owned)
}

#[async_trait]
impl CalendarProvider for CalDav {
    async fn events(
        &self,
        source: &CalendarSource,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> anyhow::Result<Vec<CalendarEvent>> {
        let secret = providers::read_secret(&source.id).await?;
        self.fetch(source, start, end, secret.expose_secret()).await
    }
    async fn save_event(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
    ) -> anyhow::Result<CalendarEvent> {
        let secret = providers::read_secret(&source.id).await?;
        self.save(source, event, secret.expose_secret()).await
    }
    async fn delete_event(
        &self,
        source: &CalendarSource,
        event: &CalendarEvent,
    ) -> anyhow::Result<()> {
        let secret = providers::read_secret(&source.id).await?;
        self.delete(source, event, secret.expose_secret()).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::calendar::{
        self,
        test_server::{self, Reply, Server},
    };

    fn resource() -> String {
        calendar::encode_ical(&test_server::event())
            .replace("BEGIN:VEVENT", "X-WR-CALNAME:Home\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/London\r\nBEGIN:STANDARD\r\nDTSTART:19701025T020000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT")
            .replace("END:VEVENT", "SEQUENCE:4\r\nORGANIZER:mailto:owner@example.com\r\nATTENDEE;CN=Friend:mailto:friend@example.com\r\nX-HOME-NOTE:retain me\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nDESCRIPTION:Keep this alarm\r\nEND:VALARM\r\nEND:VEVENT")
    }

    #[tokio::test]
    async fn caldav_edit_preserves_alarm_attendees_extensions_and_uses_etags() {
        let original = resource();
        let mut server = Server::start(vec![
            Reply::new(200, &original).header("ETag", "\"v1\""),
            Reply::new(204, "").header("ETag", "\"v2\""),
        ])
        .await;
        let provider = CalDav {
            http: test_server::client(),
        };
        let source = test_server::source(&server.url);
        let mut event = test_server::event();
        event.etag = Some("\"v1\"".into());
        event.remote_url = Some("walk.ics".into());
        event.title = "Walk later".into();
        event.description = "Bring snacks".into();
        event.start += chrono::Duration::hours(1);
        event.end += chrono::Duration::hours(1);
        let saved = provider
            .save(&source, &event, "test-password")
            .await
            .unwrap();
        assert_eq!(saved.etag.as_deref(), Some("\"v2\""));
        server.finish().await;
        let requests = server.requests();
        assert_eq!((&*requests[0].method, &*requests[1].method), ("GET", "PUT"));
        assert_eq!(requests[1].headers["if-match"], "\"v1\"");
        let body = &requests[1].body;
        for property in [
            "BEGIN:VTIMEZONE",
            "X-WR-CALNAME:Home",
            "ORGANIZER:mailto:owner@example.com",
            "ATTENDEE;CN=Friend:mailto:friend@example.com",
            "X-HOME-NOTE:retain me",
            "TRIGGER:-PT15M",
            "DESCRIPTION:Keep this alarm",
            "SEQUENCE:5",
            "DESCRIPTION:Bring snacks",
            "SUMMARY:Walk later",
        ] {
            assert!(
                body.contains(property),
                "Missing preserved/updated property: {property}"
            );
        }
        let parsed = calendar::parse_resource(body, &source, "walk.ics", saved.etag).unwrap();
        assert!(calendar::same_event_content(&parsed[0], &event));
    }

    #[tokio::test]
    async fn caldav_missing_put_etag_is_saved_and_requires_sync_before_editing() {
        let mut server = Server::start(vec![Reply::new(201, "")]).await;
        let provider = CalDav {
            http: test_server::client(),
        };
        let source = test_server::source(&server.url);
        let saved = provider
            .save(&source, &test_server::event(), "test-password")
            .await
            .unwrap();
        assert!(saved.etag.is_none());
        assert!(saved.remote_url.is_some());
        assert!(
            provider
                .save(&source, &saved, "test-password")
                .await
                .unwrap_err()
                .to_string()
                .contains("Sync calendar")
        );
        server.finish().await;
        assert_eq!(server.requests()[0].headers["if-none-match"], "*");
    }

    #[tokio::test]
    async fn caldav_redirect_is_not_acknowledged_as_a_commit() {
        let mut server = Server::start(vec![
            Reply::new(301, "").header("Location", "https://other.example/event"),
        ])
        .await;
        let provider = CalDav {
            http: test_server::client(),
        };
        let source = test_server::source(&server.url);
        assert!(
            provider
                .save(&source, &test_server::event(), "test-password")
                .await
                .is_err()
        );
        server.finish().await;
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn caldav_recovers_lost_create_response_without_overwriting() {
        let event = test_server::event();
        let mut server = Server::start(vec![
            Reply::disconnect(),
            Reply::new(412, ""),
            Reply::new(200, calendar::encode_ical(&event)).header("ETag", "\"created\""),
        ])
        .await;
        let provider = CalDav {
            http: test_server::client(),
        };
        let source = test_server::source(&server.url);
        assert!(
            provider
                .save(&source, &event, "test-password")
                .await
                .is_err()
        );
        let saved = provider
            .save(&source, &event, "test-password")
            .await
            .unwrap();
        assert_eq!(saved.etag.as_deref(), Some("\"created\""));
        server.finish().await;
        let requests = server.requests();
        assert_eq!(requests[0].target, requests[1].target);
        assert!(
            requests[..2]
                .iter()
                .all(|r| r.method == "PUT" && r.headers["if-none-match"] == "*")
        );
        assert_eq!(requests[2].method, "GET");
    }

    #[tokio::test]
    async fn caldav_stale_edit_refuses_to_overwrite_other_client_changes() {
        let mut server = Server::start(vec![
            Reply::new(200, resource()).header("ETag", "\"other-edit\""),
        ])
        .await;
        let provider = CalDav {
            http: test_server::client(),
        };
        let source = test_server::source(&server.url);
        let mut event = test_server::event();
        event.title = "My change".into();
        event.etag = Some("\"old\"".into());
        event.remote_url = Some("walk.ics".into());
        assert!(
            provider
                .save(&source, &event, "test-password")
                .await
                .unwrap_err()
                .to_string()
                .contains("changed on the server")
        );
        server.finish().await;
        assert_eq!(server.requests().len(), 1);
    }

    #[tokio::test]
    async fn caldav_report_auth_namespace_and_idempotent_delete_contract() {
        let xml = format!(
            r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/calendars/walk.ics</d:href><d:propstat><d:prop><d:getetag>"v1"</d:getetag><c:calendar-data><![CDATA[{}]]></c:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#,
            resource()
        );
        let mut server = Server::start(vec![
            Reply::new(207, &xml),
            Reply::new(404, ""),
            Reply::new(412, ""),
            Reply::new(207, xml.replace("200 OK", "403 Forbidden")),
        ])
        .await;
        let provider = CalDav {
            http: test_server::client(),
        };
        let source = test_server::source(&server.url);
        let event = test_server::event();
        let events = provider
            .fetch(&source, event.start, event.end, "test-password")
            .await
            .unwrap();
        provider
            .delete(&source, &events[0], "test-password")
            .await
            .unwrap();
        assert!(
            provider
                .delete(&source, &events[0], "test-password")
                .await
                .is_err()
        );
        assert!(
            provider
                .fetch(&source, event.start, event.end, "test-password")
                .await
                .is_err()
        );
        server.finish().await;
        let requests = server.requests();
        assert_eq!(requests[0].method, "REPORT");
        assert_eq!(requests[0].headers["depth"], "1");
        assert!(
            requests[0]
                .body
                .contains("c:expand start=\"20260906T090000Z\"")
        );
        assert_eq!(requests[1].headers["if-match"], "\"v1\"");
        assert!(
            requests
                .iter()
                .all(|r| r.headers["authorization"].starts_with("Basic "))
        );
    }
}
