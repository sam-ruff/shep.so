use crate::model::*;
use anyhow::Context;
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};

mod caldav;
pub mod discovery;
mod google_calendar;
mod google_sources;
pub(crate) use google_sources::list as google_sources;
#[cfg(test)]
mod test_server;
pub use caldav::CalDav;
pub use google_calendar::GoogleCalendar;

pub fn ensure_event_access(
    source: &CalendarSource,
    event: &CalendarEvent,
    deleting: bool,
) -> anyhow::Result<()> {
    let allowed = if deleting {
        source.access.delete
    } else if event.etag.is_some() || event.remote_url.is_some() {
        source.access.update
    } else {
        source.access.create
    };
    anyhow::ensure!(
        allowed,
        "This calendar does not allow this change. Choose a writable calendar or ask its owner for access."
    );
    Ok(())
}

pub fn validate_caldav_url(input: &str) -> anyhow::Result<url::Url> {
    anyhow::ensure!(input.len() <= 8192, "The calendar URL is too long.");
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
    anyhow::ensure!(
        url.fragment().is_none(),
        "Remove the fragment after # from the calendar URL."
    );
    Ok(url)
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
            .context("The calendar server returned an event without a resource URL.")?;
        anyhow::ensure!(
            base.join(href)?.origin() == base.origin(),
            "CalDAV returned a resource on another server."
        );
        let etag = response
            .descendants()
            .find(|n| n.has_tag_name(("DAV:", "getetag")))
            .and_then(|n| n.text())
            .map(str::to_owned);
        for status in response
            .descendants()
            .filter(|n| n.has_tag_name(("DAV:", "status")))
        {
            anyhow::ensure!(
                status.text().and_then(|s| s.split_whitespace().nth(1)) == Some("200"),
                "The calendar server returned an incomplete response. Check calendar access and sync again."
            );
        }
        let Some(data) = response
            .descendants()
            .find(|n| n.has_tag_name(("urn:ietf:params:xml:ns:caldav", "calendar-data")))
            .and_then(|n| n.text())
        else {
            continue;
        };
        out.extend(parse_resource(data, source, href, etag)?);
        anyhow::ensure!(
            out.len() <= 5000,
            "This calendar has more than 5,000 events in the sync window."
        );
    }
    Ok(out)
}

fn parse_resource(
    data: &str,
    source: &CalendarSource,
    href: &str,
    etag: Option<String>,
) -> anyhow::Result<Vec<CalendarEvent>> {
    let mut out = Vec::new();
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
                .map(Ok)
                .unwrap_or_else(|| event_default_end(start, &value("DURATION")))?;
            let uid = value("UID");
            anyhow::ensure!(!uid.is_empty(), "Calendar event has no UID.");
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
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(ch @ (',' | ';' | '\\')) => out.push(ch),
            other => {
                out.push('\\');
                if let Some(ch) = other {
                    out.push(ch);
                }
            }
        }
    }
    out
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
    fold_lines(text.lines())
}

fn fold_lines<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let mut folded = String::new();
    for line in lines.filter(|l| !l.is_empty()) {
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

fn successful(response: reqwest::Response) -> anyhow::Result<reqwest::Response> {
    let response = response.error_for_status()?;
    anyhow::ensure!(
        response.status().is_success(),
        "The calendar server returned HTTP {} instead of a successful response.",
        response.status()
    );
    Ok(response)
}

async fn response_text(response: reqwest::Response) -> anyhow::Result<String> {
    let mut response = successful(response)?;
    const LIMIT: usize = 16 * 1024 * 1024;
    anyhow::ensure!(
        response.content_length().unwrap_or(0) <= LIMIT as u64,
        "Calendar response is too large."
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= LIMIT,
            "Calendar response is too large."
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8(bytes)?)
}

async fn response_json(response: reqwest::Response) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::from_str(&response_text(response).await?)?)
}

fn same_event_content(a: &CalendarEvent, b: &CalendarEvent) -> bool {
    a.source_id == b.source_id
        && a.title == b.title
        && a.start == b.start
        && a.end == b.end
        && a.location == b.location
        && a.description == b.description
        && a.all_day == b.all_day
}

fn edit_ical(
    original: &str,
    current: &CalendarEvent,
    event: &CalendarEvent,
) -> anyhow::Result<String> {
    anyhow::ensure!(
        current.id == event.id && current.remote_url.is_some(),
        "Cannot edit this calendar resource."
    );
    let mut lines: Vec<String> = Vec::new();
    for line in original.lines() {
        if let Some(continuation) = line.strip_prefix([' ', '\t']) {
            lines
                .last_mut()
                .context("Invalid folded calendar property")?
                .push_str(continuation);
        } else {
            lines.push(line.into());
        }
    }
    anyhow::ensure!(
        lines
            .iter()
            .filter(|s| s.eq_ignore_ascii_case("BEGIN:VEVENT"))
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
                        .context("Invalid calendar sequence")?
                        .1
                        .parse::<u32>()?;
                    out.push(format!(
                        "SEQUENCE:{}",
                        sequence
                            .checked_add(1)
                            .context("Calendar sequence overflow")?
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
    anyhow::ensure!(depth == 0, "Incomplete calendar resource");
    Ok(fold_lines(out.iter().map(String::as_str)))
}

fn event_default_end(
    start: &ical::property::Property,
    duration: &str,
) -> anyhow::Result<DateTime<Utc>> {
    let value = start.value.as_deref().context("Missing event start")?;
    let all_day = value.len() == 8;
    if duration.is_empty() {
        return parse_ical_date(start)?
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
    parse_ical_date(&shifted)?
        .checked_add_signed(
            chrono::Duration::try_seconds(seconds).context("Event duration overflow")?,
        )
        .context("Calendar date overflow")
}
