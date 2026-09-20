use crate::{Event, Source};
use anyhow::Context;
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};

pub fn validate_url(input: &str) -> anyhow::Result<url::Url> {
    anyhow::ensure!(input.len() <= 8192, "The calendar URL is too long.");
    let url = url::Url::parse(input).context("Enter the full CalDAV calendar URL")?;
    anyhow::ensure!(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"))),
        "Use HTTPS for CalDAV. Plain HTTP is allowed only on localhost."
    );
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none(),
        "Enter the CalDAV username and password separately."
    );
    anyhow::ensure!(url.fragment().is_none(), "Remove the URL fragment after #.");
    Ok(url)
}
#[cfg(feature = "http")]
use crate::{ProviderFailure, http::CalendarProvider};
#[cfg(feature = "http")]
use async_trait::async_trait;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CalDavConnection {
    pub id: String,
    pub url: String,
    pub username: String,
}

impl CalDavConnection {
    pub fn is_bounded(&self) -> bool {
        !self.id.is_empty()
            && self.id.len() <= 1024
            && self.url.len() <= 8192
            && self.username.len() <= 1024
    }
}

#[cfg(feature = "http")]
async fn bounded_text(mut response: reqwest::Response) -> Result<String, ProviderFailure> {
    const LIMIT: usize = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ProviderFailure::waiting("Could not read the CalDAV response."))?
    {
        if bytes.len().saturating_add(chunk.len()) > LIMIT {
            return Err(ProviderFailure::waiting(
                "The CalDAV response is too large.",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map_err(|_| ProviderFailure::waiting("CalDAV returned unreadable text."))
}

#[cfg(feature = "http")]
pub struct CalDavProvider {
    client: reqwest::Client,
    connection: CalDavConnection,
}

#[cfg(feature = "http")]
impl CalDavProvider {
    pub fn new(connection: CalDavConnection) -> anyhow::Result<Self> {
        anyhow::ensure!(
            connection.is_bounded(),
            "This CalDAV connection is invalid."
        );
        validate_url(&connection.url)?;
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(45))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            connection,
        })
    }

    fn source(&self, read_only: bool) -> Source {
        Source {
            id: self.connection.id.clone(),
            name: self.connection.id.clone(),
            read_only,
        }
    }

    fn event_url(&self, event: &Event) -> Result<url::Url, ProviderFailure> {
        let base = validate_url(&self.connection.url)
            .map_err(|error| ProviderFailure::rejected(error.to_string()))?;
        let url = match event.remote_url.as_deref() {
            Some(remote) => base
                .join(remote)
                .map_err(|error| ProviderFailure::rejected(error.to_string()))?,
            None => {
                let mut url = base.clone();
                url.path_segments_mut()
                    .map_err(|_| ProviderFailure::rejected("The CalDAV URL cannot hold events."))?
                    .pop_if_empty()
                    .push(&format!("{}.ics", event.id));
                url
            }
        };
        if !within_collection(&base, &url) {
            return Err(ProviderFailure::rejected(
                "Refusing to send calendar credentials outside this collection.",
            ));
        }
        Ok(url)
    }

    fn auth(&self, request: reqwest::RequestBuilder, password: &str) -> reqwest::RequestBuilder {
        request.basic_auth(&self.connection.username, Some(password))
    }

    async fn resource(
        &self,
        password: &str,
        event: &Event,
    ) -> Result<(Event, String), ProviderFailure> {
        let url = self.event_url(event)?;
        let response = self
            .auth(self.client.get(url.clone()), password)
            .send()
            .await
            .map_err(|_| ProviderFailure::waiting("Could not read the complete CalDAV event."))?;
        let status = response.status();
        let etag = strong_etag(&response);
        let text = bounded_text(response).await?;
        if !status.is_success() {
            return Err(caldav_failure(status, false));
        }
        let source = self.source(false);
        let mut events = parse_resource(&text, &source, url.as_str(), etag)
            .map_err(|error| ProviderFailure::rejected(error.to_string()))?;
        if events.len() != 1 || events[0].id != event.id {
            return Err(ProviderFailure::rejected(
                "Edit recurring events on your calendar server.",
            ));
        }
        Ok((events.remove(0), text))
    }
}

#[cfg(feature = "http")]
fn caldav_failure(status: reqwest::StatusCode, mutation: bool) -> ProviderFailure {
    let message = format!("CalDAV returned HTTP {status}.");
    if matches!(
        status,
        reqwest::StatusCode::UNAUTHORIZED
            | reqwest::StatusCode::FORBIDDEN
            | reqwest::StatusCode::REQUEST_TIMEOUT
            | reqwest::StatusCode::TOO_MANY_REQUESTS
    ) {
        ProviderFailure::waiting(message)
    } else if status.is_client_error() {
        ProviderFailure::rejected(message)
    } else if mutation {
        ProviderFailure::uncertain(message)
    } else {
        ProviderFailure::waiting(message)
    }
}

#[cfg(feature = "http")]
fn strong_etag(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .filter(|value| is_strong_etag(value))
        .map(str::to_owned)
}

fn is_strong_etag(value: &str) -> bool {
    value.len() >= 2
        && value.starts_with('"')
        && value.ends_with('"')
        && value.as_bytes()[1..value.len() - 1]
            .iter()
            .all(|byte| *byte == 0x21 || (0x23..=0x7e).contains(byte) || *byte >= 0x80)
}

fn within_collection(base: &url::Url, candidate: &url::Url) -> bool {
    if candidate.origin() != base.origin() {
        return false;
    }
    let mut prefix = base.path().to_owned();
    if !prefix.ends_with('/') {
        prefix.push('/');
    }
    candidate.path().starts_with(&prefix)
}

#[cfg(feature = "http")]
#[async_trait]
impl CalendarProvider for CalDavProvider {
    async fn sources(&self, password: &str) -> Result<Vec<Source>, ProviderFailure> {
        let url = validate_url(&self.connection.url)
            .map_err(|error| ProviderFailure::rejected(error.to_string()))?;
        let body = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:resourcetype/><d:displayname/><d:current-user-privilege-set/></d:prop></d:propfind>"#;
        let response = self
            .auth(
                self.client
                    .request(
                        reqwest::Method::from_bytes(b"PROPFIND")
                            .map_err(|error| ProviderFailure::rejected(error.to_string()))?,
                        url.clone(),
                    )
                    .header("Depth", "0")
                    .header("Content-Type", "application/xml; charset=utf-8")
                    .body(body),
                password,
            )
            .send()
            .await
            .map_err(|_| {
                ProviderFailure::waiting("Could not connect to CalDAV. Retry when online.")
            })?;
        let status = response.status();
        let text = bounded_text(response).await?;
        if !status.is_success() && status.as_u16() != 207 {
            return Err(caldav_failure(status, false));
        }
        let source = parse_collection(&text, &url, &self.connection.id)
            .map_err(|error| ProviderFailure::rejected(error.to_string()))?;
        Ok(vec![source])
    }

    async fn events(
        &self,
        password: &str,
        source: &Source,
        start: chrono::DateTime<chrono::Utc>,
        end: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Event>, ProviderFailure> {
        if source.id != self.connection.id {
            return Err(ProviderFailure::rejected(
                "This calendar connection changed.",
            ));
        }
        let url = validate_url(&self.connection.url)
            .map_err(|error| ProviderFailure::rejected(error.to_string()))?;
        let body = format!(
            r#"<?xml version="1.0"?><c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:getetag/><c:calendar-data><c:expand start="{}" end="{}"/></c:calendar-data></d:prop><c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"><c:time-range start="{}" end="{}"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"#,
            start.format("%Y%m%dT%H%M%SZ"),
            end.format("%Y%m%dT%H%M%SZ"),
            start.format("%Y%m%dT%H%M%SZ"),
            end.format("%Y%m%dT%H%M%SZ")
        );
        let response = self
            .auth(
                self.client
                    .request(
                        reqwest::Method::from_bytes(b"REPORT")
                            .map_err(|error| ProviderFailure::rejected(error.to_string()))?,
                        url.clone(),
                    )
                    .header("Depth", "1")
                    .header("Content-Type", "application/xml; charset=utf-8")
                    .body(body),
                password,
            )
            .send()
            .await
            .map_err(|_| {
                ProviderFailure::waiting("Could not load CalDAV events. Retry when online.")
            })?;
        let status = response.status();
        let text = bounded_text(response).await?;
        if !status.is_success() && status.as_u16() != 207 {
            return Err(caldav_failure(status, false));
        }
        parse_report(&text, source, &url)
            .map_err(|error| ProviderFailure::rejected(error.to_string()))
    }

    async fn save(
        &self,
        password: &str,
        _request_id: &str,
        event: &Event,
    ) -> Result<Event, ProviderFailure> {
        if event.source_id != self.connection.id || !event.is_bounded() {
            return Err(ProviderFailure::rejected("This calendar event is invalid."));
        }
        let url = self.event_url(event)?;
        let creating = event.remote_url.is_none() && event.etag.is_none();
        let (request, body) = if creating {
            (
                self.client.put(url.clone()).header("If-None-Match", "*"),
                encode(event),
            )
        } else {
            if event.remote_url.is_none() {
                return Err(ProviderFailure::rejected(
                    "Edit recurring events on your calendar server.",
                ));
            }
            let etag = event
                .etag
                .as_deref()
                .filter(|value| is_strong_etag(value))
                .ok_or_else(|| ProviderFailure::rejected("Sync this event before editing it."))?;
            let (current, original) = self.resource(password, event).await?;
            if current.etag.as_deref() != Some(etag) {
                return Err(ProviderFailure::rejected(
                    "This event changed on the server. Sync before editing it.",
                ));
            }
            let body = edit(&original, &current, event)
                .map_err(|error| ProviderFailure::rejected(error.to_string()))?;
            (self.client.put(url.clone()).header("If-Match", etag), body)
        };
        let response = self
            .auth(
                request
                    .header("Content-Type", "text/calendar; charset=utf-8")
                    .body(body),
                password,
            )
            .send()
            .await
            .map_err(|error| {
                if error.is_builder() {
                    ProviderFailure::rejected(error.to_string())
                } else {
                    ProviderFailure::uncertain(
                        "The event may have reached CalDAV. Inspect it before retrying.",
                    )
                }
            })?;
        let status = response.status();
        if !status.is_success() {
            return Err(caldav_failure(status, true));
        }
        let etag = strong_etag(&response).ok_or_else(|| {
            ProviderFailure::uncertain(
                "CalDAV accepted the event without a usable version. Inspect it before retrying.",
            )
        })?;
        let mut saved = event.clone();
        saved.etag = Some(etag);
        saved.remote_url = Some(url.to_string());
        Ok(saved)
    }

    async fn read(&self, password: &str, event: &Event) -> Result<Option<Event>, ProviderFailure> {
        let url = self.event_url(event)?;
        let response = self
            .auth(self.client.get(url.clone()), password)
            .send()
            .await
            .map_err(|_| {
                ProviderFailure::waiting("Could not inspect CalDAV. Retry when online.")
            })?;
        if matches!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
        ) {
            return Ok(None);
        }
        let status = response.status();
        let etag = strong_etag(&response);
        let text = bounded_text(response).await?;
        if !status.is_success() {
            return Err(caldav_failure(status, false));
        }
        let etag = etag.ok_or_else(|| {
            ProviderFailure::rejected("CalDAV returned an event without a usable version.")
        })?;
        let source = self.source(false);
        let mut events = parse_resource(&text, &source, url.as_str(), Some(etag))
            .map_err(|error| ProviderFailure::rejected(error.to_string()))?;
        if events.len() != 1 || events[0].id != event.id {
            return Err(ProviderFailure::rejected(
                "The CalDAV resource no longer identifies this event.",
            ));
        }
        Ok(events.pop())
    }

    async fn delete(&self, password: &str, event: &Event) -> Result<(), ProviderFailure> {
        if event.source_id != self.connection.id || event.remote_url.is_none() {
            return Err(ProviderFailure::rejected(
                "Sync this event before deleting it. Edit recurring events on your calendar server.",
            ));
        }
        let etag = event
            .etag
            .as_deref()
            .filter(|value| is_strong_etag(value))
            .ok_or_else(|| ProviderFailure::rejected("Sync this event before deleting it."))?;
        let response = self
            .auth(
                self.client
                    .delete(self.event_url(event)?)
                    .header("If-Match", etag),
                password,
            )
            .send()
            .await
            .map_err(|error| {
                if error.is_builder() {
                    ProviderFailure::rejected(error.to_string())
                } else {
                    ProviderFailure::uncertain(
                        "The delete may have reached CalDAV. Inspect it before retrying.",
                    )
                }
            })?;
        if matches!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::GONE
        ) || response.status().is_success()
        {
            Ok(())
        } else {
            Err(caldav_failure(response.status(), true))
        }
    }
}

fn parse_collection(xml: &str, base: &url::Url, id: &str) -> anyhow::Result<Source> {
    let document = roxmltree::Document::parse(xml)?;
    anyhow::ensure!(
        document
            .root_element()
            .has_tag_name(("DAV:", "multistatus")),
        "CalDAV returned an invalid property response."
    );
    let mut matches = document.descendants().filter(|node| {
        node.has_tag_name(("DAV:", "response"))
            && node
                .children()
                .find(|child| child.has_tag_name(("DAV:", "href")))
                .and_then(|child| child.text())
                .and_then(|href| base.join(href).ok())
                .is_some_and(|url| url == *base)
    });
    let response = matches
        .next()
        .context("CalDAV did not identify this calendar collection.")?;
    anyhow::ensure!(
        matches.next().is_none(),
        "CalDAV returned duplicate collection properties."
    );
    let properties: Vec<_> = response
        .children()
        .filter(|node| node.has_tag_name(("DAV:", "propstat")))
        .filter(|node| {
            node.children()
                .find(|child| child.has_tag_name(("DAV:", "status")))
                .and_then(|child| child.text())
                .and_then(|text| text.split_whitespace().nth(1))
                == Some("200")
        })
        .filter_map(|node| {
            node.children()
                .find(|child| child.has_tag_name(("DAV:", "prop")))
        })
        .flat_map(|node| node.children().filter(|child| child.is_element()))
        .collect();
    anyhow::ensure!(
        properties.iter().any(|node| {
            node.has_tag_name(("DAV:", "resourcetype"))
                && node
                    .children()
                    .any(|child| child.has_tag_name(("urn:ietf:params:xml:ns:caldav", "calendar")))
        }),
        "Enter a CalDAV calendar collection URL."
    );
    let read_only = !properties.iter().any(|node| {
        node.has_tag_name(("DAV:", "current-user-privilege-set"))
            && node.descendants().any(|child| {
                child.has_tag_name(("DAV:", "write"))
                    || child.has_tag_name(("DAV:", "write-content"))
                    || child.has_tag_name(("DAV:", "all"))
            })
    });
    let name = properties
        .iter()
        .find(|node| node.has_tag_name(("DAV:", "displayname")))
        .and_then(|node| node.text())
        .filter(|name| !name.is_empty())
        .unwrap_or(id);
    anyhow::ensure!(name.len() <= 1024, "The CalDAV calendar name is too long.");
    Ok(Source {
        id: id.into(),
        name: name.into(),
        read_only,
    })
}

pub fn parse_report(xml: &str, source: &Source, base: &url::Url) -> anyhow::Result<Vec<Event>> {
    anyhow::ensure!(
        xml.len() <= 16 * 1024 * 1024,
        "The CalDAV response is too large."
    );
    let document = roxmltree::Document::parse(xml)?;
    anyhow::ensure!(
        document
            .root_element()
            .has_tag_name(("DAV:", "multistatus")),
        "CalDAV returned an invalid event response."
    );
    let mut out = Vec::new();
    for response in document
        .descendants()
        .filter(|node| node.has_tag_name(("DAV:", "response")))
    {
        let href = response
            .descendants()
            .find(|node| node.has_tag_name(("DAV:", "href")))
            .and_then(|node| node.text())
            .context("CalDAV returned an event without a resource URL.")?;
        let resource_url = base.join(href)?;
        anyhow::ensure!(
            within_collection(base, &resource_url),
            "CalDAV returned a resource outside this collection."
        );
        for status in response
            .descendants()
            .filter(|node| node.has_tag_name(("DAV:", "status")))
        {
            anyhow::ensure!(
                status
                    .text()
                    .and_then(|value| value.split_whitespace().nth(1))
                    == Some("200"),
                "CalDAV returned an incomplete response."
            );
        }
        let etag = response
            .descendants()
            .find(|node| node.has_tag_name(("DAV:", "getetag")))
            .and_then(|node| node.text())
            .filter(|value| is_strong_etag(value))
            .map(str::to_owned);
        let data = response
            .descendants()
            .find(|node| node.has_tag_name(("urn:ietf:params:xml:ns:caldav", "calendar-data")))
            .and_then(|node| node.text())
            .context("CalDAV returned an event without calendar data.")?;
        anyhow::ensure!(
            etag.is_some(),
            "CalDAV returned an event without a usable version."
        );
        let events = parse_resource(data, source, resource_url.as_str(), etag)?;
        anyhow::ensure!(
            !events.is_empty(),
            "CalDAV returned an empty event resource."
        );
        out.extend(events);
        anyhow::ensure!(
            out.len() <= 5000,
            "This sync window contains more than 5,000 events."
        );
    }
    Ok(out)
}

pub fn parse_resource(
    data: &str,
    source: &Source,
    href: &str,
    etag: Option<String>,
) -> anyhow::Result<Vec<Event>> {
    anyhow::ensure!(
        data.len() <= 16 * 1024 * 1024,
        "The calendar resource is too large."
    );
    let mut out = Vec::new();
    for calendar in ical::IcalParser::new(std::io::BufReader::new(data.as_bytes())) {
        for event in calendar?.events {
            let value = |key: &str| {
                event
                    .properties
                    .iter()
                    .find(|property| property.name == key)
                    .and_then(|property| property.value.clone())
                    .unwrap_or_default()
            };
            let start = event
                .properties
                .iter()
                .find(|property| property.name == "DTSTART")
                .context("Calendar event has no start.")?;
            let begin = parse_date(start)?;
            let end = event
                .properties
                .iter()
                .find(|property| property.name == "DTEND")
                .map(parse_date)
                .transpose()?
                .map(Ok)
                .unwrap_or_else(|| event_default_end(start, &value("DURATION")))?;
            let uid = unescape(&value("UID"));
            anyhow::ensure!(!uid.is_empty(), "Calendar event has no UID.");
            let recurrence = value("RECURRENCE-ID");
            let recurring = !recurrence.is_empty()
                || event
                    .properties
                    .iter()
                    .any(|property| matches!(property.name.as_str(), "RRULE" | "RDATE" | "EXDATE"));
            let parsed = Event {
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
                all_day: start.value.as_ref().is_some_and(|value| value.len() == 8),
                etag: etag.clone(),
                remote_url: (!recurring).then(|| href.to_owned()),
            };
            anyhow::ensure!(
                parsed.is_bounded(),
                "CalDAV returned oversized event metadata."
            );
            out.push(parsed);
        }
    }
    Ok(out)
}

fn event_default_end(
    start: &ical::property::Property,
    duration: &str,
) -> anyhow::Result<DateTime<Utc>> {
    let value = start.value.as_deref().context("Missing event start")?;
    let all_day = value.len() == 8;
    if duration.is_empty() {
        return parse_date(start)?
            .checked_add_signed(chrono::Duration::days(i64::from(all_day)))
            .context("Calendar date overflow");
    }
    let duration = duration.strip_prefix('+').unwrap_or(duration);
    let input = duration
        .strip_prefix('P')
        .context("Invalid event duration")?;
    let mut number = String::new();
    let (mut days, mut seconds, mut time, mut rank, mut components) = (0_i64, 0_i64, false, 0, 0);
    for ch in input.chars() {
        if ch.is_ascii_digit() {
            number.push(ch);
            continue;
        }
        if ch == 'T' {
            anyhow::ensure!(
                !time && number.is_empty() && rank != 1,
                "Invalid event duration"
            );
            time = true;
            continue;
        }
        let n: i64 = number.parse().context("Invalid event duration")?;
        number.clear();
        let (next_rank, multiplier) = match (time, ch) {
            (false, 'W') => (1, 7),
            (false, 'D') => (2, 1),
            (true, 'H') => (3, 3600),
            (true, 'M') => (4, 60),
            (true, 'S') => (5, 1),
            _ => anyhow::bail!("Invalid event duration"),
        };
        anyhow::ensure!(
            next_rank > rank && !(rank == 1 || next_rank == 1 && components > 0),
            "Invalid event duration"
        );
        rank = next_rank;
        components += 1;
        let value = n
            .checked_mul(multiplier)
            .context("Event duration overflow")?;
        if time {
            seconds = seconds
                .checked_add(value)
                .context("Event duration overflow")?;
        } else {
            days = value;
        }
    }
    anyhow::ensure!(
        components > 0 && number.is_empty() && !input.ends_with('T') && (!all_day || !time),
        "Invalid event duration"
    );
    // Nominal days retain wall-clock time across DST; hours/minutes/seconds are exact.
    let mut shifted = start.clone();
    if days != 0 {
        let delta = chrono::Duration::try_days(days).context("Event duration overflow")?;
        shifted.value = Some(if all_day {
            NaiveDate::parse_from_str(value, "%Y%m%d")?
                .checked_add_signed(delta)
                .context("Calendar date overflow")?
                .format("%Y%m%d")
                .to_string()
        } else {
            let date = NaiveDateTime::parse_from_str(value.trim_end_matches('Z'), "%Y%m%dT%H%M%S")?
                .checked_add_signed(delta)
                .context("Calendar date overflow")?;
            format!(
                "{}{}",
                date.format("%Y%m%dT%H%M%S"),
                if value.ends_with('Z') { "Z" } else { "" }
            )
        });
    }
    parse_date(&shifted)?
        .checked_add_signed(
            chrono::Duration::try_seconds(seconds).context("Event duration overflow")?,
        )
        .context("Calendar date overflow")
}

fn parse_date(property: &ical::property::Property) -> anyhow::Result<DateTime<Utc>> {
    let value = property.value.as_deref().context("Empty calendar date.")?;
    if value.len() == 8 {
        return Ok(NaiveDate::parse_from_str(value, "%Y%m%d")?
            .and_hms_opt(0, 0, 0)
            .context("Invalid calendar date.")?
            .and_utc());
    }
    let date = NaiveDateTime::parse_from_str(value.trim_end_matches('Z'), "%Y%m%dT%H%M%S")?;
    if value.ends_with('Z') {
        return Ok(date.and_utc());
    }
    if let Some(zone) = property
        .params
        .as_ref()
        .and_then(|params| params.iter().find(|(key, _)| key == "TZID"))
        .and_then(|(_, value)| value.first())
    {
        let zone: chrono_tz::Tz = zone
            .parse()
            .map_err(|_| anyhow::anyhow!("Unsupported calendar timezone: {zone}"))?;
        return Ok(zone
            .from_local_datetime(&date)
            .earliest()
            .context("Invalid calendar local time.")?
            .with_timezone(&Utc));
    }
    Ok(chrono::Local
        .from_local_datetime(&date)
        .earliest()
        .context("Invalid local calendar time.")?
        .with_timezone(&Utc))
}

pub fn encode(event: &Event) -> String {
    let (start, end) = if event.all_day {
        (
            format!("DTSTART;VALUE=DATE:{}", event.start.format("%Y%m%d")),
            format!("DTEND;VALUE=DATE:{}", event.end.format("%Y%m%d")),
        )
    } else {
        (
            format!("DTSTART:{}", event.start.format("%Y%m%dT%H%M%SZ")),
            format!("DTEND:{}", event.end.format("%Y%m%dT%H%M%SZ")),
        )
    };
    fold(format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Shep//Calendar 1.0//EN\r\nBEGIN:VEVENT\r\nUID:{}\r\nDTSTAMP:{}\r\n{start}\r\n{end}\r\nSUMMARY:{}\r\nLOCATION:{}\r\nDESCRIPTION:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        escape(&event.id),
        Utc::now().format("%Y%m%dT%H%M%SZ"),
        escape(&event.title),
        escape(&event.location),
        escape(&event.description)
    ))
}

pub fn edit(original: &str, current: &Event, event: &Event) -> anyhow::Result<String> {
    anyhow::ensure!(
        current.id == event.id && current.remote_url.is_some(),
        "Cannot edit this calendar resource."
    );
    let mut lines: Vec<String> = Vec::new();
    for line in original.lines() {
        if let Some(continuation) = line.strip_prefix([' ', '\t']) {
            lines
                .last_mut()
                .context("Invalid folded calendar property.")?
                .push_str(continuation);
        } else {
            lines.push(line.into());
        }
    }
    anyhow::ensure!(
        lines
            .iter()
            .filter(|line| line.eq_ignore_ascii_case("BEGIN:VEVENT"))
            .count()
            == 1,
        "Edit recurring events on your calendar server."
    );
    let mut replacements = std::collections::BTreeMap::new();
    for (key, old, new) in [
        ("SUMMARY", &current.title, &event.title),
        ("LOCATION", &current.location, &event.location),
        ("DESCRIPTION", &current.description, &event.description),
    ] {
        if old != new {
            replacements.insert(key, format!("{key}:{}", escape(new)));
        }
    }
    let dates_changed = current.start != event.start
        || current.end != event.end
        || current.all_day != event.all_day;
    if dates_changed {
        for (key, date) in [("DTSTART", event.start), ("DTEND", event.end)] {
            replacements.insert(
                key,
                if event.all_day {
                    format!("{key};VALUE=DATE:{}", date.format("%Y%m%d"))
                } else {
                    format!("{key}:{}", date.format("%Y%m%dT%H%M%SZ"))
                },
            );
        }
    }
    let now = Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    replacements.insert("DTSTAMP", format!("DTSTAMP:{now}"));
    replacements.insert("LAST-MODIFIED", format!("LAST-MODIFIED:{now}"));
    let mut out = Vec::new();
    let mut depth = 0;
    for line in lines {
        if line.eq_ignore_ascii_case("BEGIN:VEVENT") {
            depth = 1;
            out.push(line);
            continue;
        }
        if depth > 0 {
            if line.to_ascii_uppercase().starts_with("BEGIN:") {
                depth += 1;
            }
            if line.to_ascii_uppercase().starts_with("END:") {
                if depth == 1 {
                    out.extend(std::mem::take(&mut replacements).into_values());
                    depth = 0;
                    out.push(line);
                    continue;
                }
                depth -= 1;
                out.push(line);
                continue;
            }
            if depth == 1 {
                let name = line
                    .split([';', ':'])
                    .next()
                    .unwrap_or("")
                    .to_ascii_uppercase();
                if dates_changed && name == "DURATION" {
                    continue;
                }
                if name == "SEQUENCE" {
                    let sequence = line
                        .split_once(':')
                        .context("Invalid calendar sequence.")?
                        .1
                        .parse::<u32>()?;
                    out.push(format!(
                        "SEQUENCE:{}",
                        sequence
                            .checked_add(1)
                            .context("Calendar sequence overflow.")?
                    ));
                    continue;
                }
                if let Some(replacement) = replacements.remove(name.as_str()) {
                    out.push(replacement);
                    continue;
                }
            }
        }
        out.push(line);
    }
    anyhow::ensure!(depth == 0, "Incomplete calendar resource.");
    Ok(fold(out.join("\r\n")))
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\r', "")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}

fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(character @ (',' | ';' | '\\')) => out.push(character),
            other => {
                out.push('\\');
                if let Some(character) = other {
                    out.push(character);
                }
            }
        }
    }
    out
}

fn fold(text: String) -> String {
    let mut folded = String::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let mut width = 0;
        for character in line.chars() {
            if width + character.len_utf8() > 74 {
                folded.push_str("\r\n ");
                width = 1;
            }
            folded.push(character);
            width += character.len_utf8();
        }
        folded.push_str("\r\n");
    }
    folded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_discovery_uses_only_its_successful_properties() {
        let base = validate_url("https://calendar.example.test/home/").expect("URL");
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
            <d:response><d:href>/other/</d:href><d:propstat><d:prop><d:displayname>Other</d:displayname><d:current-user-privilege-set><d:privilege><d:all/></d:privilege></d:current-user-privilege-set></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>
            <d:response><d:href>/home/</d:href><d:propstat><d:prop><d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>Home</d:displayname></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat><d:propstat><d:prop><d:current-user-privilege-set><d:privilege><d:write/></d:privilege></d:current-user-privilege-set></d:prop><d:status>HTTP/1.1 403 Forbidden</d:status></d:propstat></d:response>
            </d:multistatus>"#;
        let source = parse_collection(xml, &base, "home").expect("collection");
        assert_eq!(source.name, "Home");
        assert!(source.read_only);
        let writable = xml.replace("HTTP/1.1 403 Forbidden", "HTTP/1.1 200 OK");
        assert!(
            !parse_collection(&writable, &base, "home")
                .expect("writable")
                .read_only
        );
        assert!(parse_collection(&xml.replace("/home/", "/missing/"), &base, "home").is_err());
        assert!(
            parse_collection(
                &xml.replace("HTTP/1.1 200 OK", "HTTP/1.1 404 Not Found"),
                &base,
                "home"
            )
            .is_err()
        );
    }

    #[test]
    fn end_defaults_duration_and_escaped_uid_preserve_the_resource() {
        let source = Source {
            id: "home".into(),
            name: "Home".into(),
            read_only: false,
        };
        let parse = |start: &str, extra: &str| {
            let raw = format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:team\\,plan\r\n{start}\r\n{extra}END:VEVENT\r\nEND:VCALENDAR\r\n"
            );
            parse_resource(&raw, &source, "plan.ics", Some("\"v1\"".into()))
        };
        let all_day = parse("DTSTART;VALUE=DATE:20260920", "")
            .expect("all day")
            .remove(0);
        assert_eq!(all_day.end - all_day.start, chrono::Duration::days(1));
        assert_eq!(all_day.id, "team,plan");
        let instant = parse("DTSTART:20260920T100000Z", "")
            .expect("instant")
            .remove(0);
        assert_eq!(instant.end, instant.start);
        let duration = parse(
            "DTSTART;TZID=Europe/London:20260328T100000",
            "DURATION:P1D\r\n",
        )
        .expect("duration")
        .remove(0);
        assert_eq!(duration.end - duration.start, chrono::Duration::hours(23));
        let exact = parse(
            "DTSTART;TZID=Europe/London:20260328T100000",
            "DURATION:PT24H\r\n",
        )
        .expect("exact")
        .remove(0);
        assert_eq!(exact.end - exact.start, chrono::Duration::hours(24));
        assert!(parse("DTSTART;VALUE=DATE:20260920", "DURATION:PT1H\r\n").is_err());
        assert!(
            parse(
                "DTSTART:20260920T100000Z",
                "DURATION:P999999999999999999999D\r\n"
            )
            .is_err()
        );
        let round_trip =
            parse_resource(&encode(&all_day), &source, "plan.ics", all_day.etag.clone())
                .expect("round trip");
        assert_eq!(round_trip[0].id, all_day.id);
        assert_eq!(round_trip[0].end, all_day.end);
    }

    #[test]
    fn incomplete_report_cannot_become_an_empty_authoritative_snapshot() {
        let source = Source {
            id: "home".into(),
            name: "Home".into(),
            read_only: false,
        };
        let base = validate_url("https://calendar.example.test/home/").expect("URL");
        assert!(parse_report("<html/>", &source, &base).is_err());
        let empty = r#"<d:multistatus xmlns:d="DAV:"/>"#;
        assert!(
            parse_report(empty, &source, &base)
                .expect("empty calendar")
                .is_empty()
        );
        let missing = r#"<d:multistatus xmlns:d="DAV:"><d:response><d:href>/home/event.ics</d:href><d:propstat><d:prop><d:getetag>"v1"</d:getetag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        assert!(parse_report(missing, &source, &base).is_err());
        let no_event = missing.replace("</d:prop>", "<c:calendar-data xmlns:c=\"urn:ietf:params:xml:ns:caldav\">BEGIN:VCALENDAR\r\nVERSION:2.0\r\nEND:VCALENDAR\r\n</c:calendar-data></d:prop>");
        assert!(parse_report(&no_event, &source, &base).is_err());
    }

    #[test]
    fn rejects_cross_origin_resources_and_round_trips_bounded_event() {
        let source = Source {
            id: "home".into(),
            name: "Home".into(),
            read_only: false,
        };
        let base = validate_url("https://calendar.example.test/home/").expect("url");
        let event = Event {
            id: "walk".into(),
            source_id: source.id.clone(),
            title: "Walk".into(),
            start: DateTime::from_timestamp(1_700_000_000, 0).expect("time"),
            end: DateTime::from_timestamp(1_700_003_600, 0).expect("time"),
            location: String::new(),
            description: String::new(),
            all_day: false,
            etag: Some("\"v1\"".into()),
            remote_url: Some("walk.ics".into()),
        };
        let parsed = parse_resource(&encode(&event), &source, "walk.ics", event.etag.clone())
            .expect("parse");
        assert_eq!(parsed[0].id, event.id);
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>https://other.test/walk.ics</d:href><d:propstat><d:prop><c:calendar-data>BEGIN:VCALENDAR</c:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#;
        assert!(parse_report(xml, &source, &base).is_err());
        let sibling = xml.replace(
            "https://other.test/walk.ics",
            "https://calendar.example.test/other/walk.ics",
        );
        assert!(parse_report(&sibling, &source, &base).is_err());
    }

    #[test]
    fn recurrence_identity_and_escaped_collection_resource_stay_bounded() {
        let source = Source {
            id: "home".into(),
            name: "Home".into(),
            read_only: false,
        };
        let recurring = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:team/plan\r\nRECURRENCE-ID:20260920T100000Z\r\nDTSTART:20260920T100000Z\r\nDTEND:20260920T110000Z\r\nSUMMARY:Review\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let parsed = parse_resource(
            recurring,
            &source,
            "https://calendar.example.test/home/team%2Fplan.ics",
            Some("\"v1\"".into()),
        )
        .expect("recurrence");
        assert_eq!(parsed[0].id, "team/plan/20260920T100000Z");
        assert!(parsed[0].remote_url.is_none());
        assert!(parsed[0].description.starts_with("Recurring event"));

        let provider = CalDavProvider::new(CalDavConnection {
            id: "home".into(),
            url: "https://calendar.example.test/home/".into(),
            username: "sam".into(),
        })
        .expect("provider");
        let mut event = parsed[0].clone();
        event.id = "team/plan".into();
        event.remote_url = None;
        let url = provider.event_url(&event).expect("resource URL");
        assert_eq!(
            url.as_str(),
            "https://calendar.example.test/home/team%2Fplan.ics"
        );
    }
}

#[cfg(all(test, feature = "http"))]
mod http_tests;
