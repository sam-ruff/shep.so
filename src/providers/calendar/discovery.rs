//! Bounded, same-origin CalDAV discovery. No events or credentials are persisted here.
use super::{CalDav, response_text, validate_caldav_url};
use crate::model::CalendarAccess;
use anyhow::Context;
use roxmltree::Node;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use url::Url;

const DAV: &str = "DAV:";
const CAL: &str = "urn:ietf:params:xml:ns:caldav";
const PROPS: &str = r#"<?xml version="1.0"?><d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:prop><d:resourcetype/><d:displayname/><d:current-user-principal/><c:calendar-home-set/><c:supported-calendar-component-set/><d:current-user-privilege-set/></d:prop></d:propfind>"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredCalendar {
    pub name: String,
    pub url: String,
    pub access: CalendarAccess,
}
#[derive(Default)]
struct Properties {
    calendars: Vec<DiscoveredCalendar>,
    principals: Vec<Url>,
    homes: Vec<Url>,
}

fn resolve(base: &Url, href: &str) -> anyhow::Result<Url> {
    let url = base
        .join(href.trim())
        .context("The server returned an invalid calendar address.")?;
    validate_caldav_url(url.as_str())?;
    anyhow::ensure!(
        url.origin() == base.origin(),
        "The calendar service points to another server. Enter that server's HTTPS address to connect; credentials have not been forwarded."
    );
    Ok(url)
}
fn child<'a, 'i>(node: Node<'a, 'i>, ns: &str, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name((ns, name)))
}
fn code(node: Node<'_, '_>) -> Option<u16> {
    node.text()?.split_whitespace().nth(1)?.parse().ok()
}
fn parse(xml: &str, base: &Url, depth: u8) -> anyhow::Result<Properties> {
    let doc = roxmltree::Document::parse(xml)
        .context("The server did not return a valid CalDAV response.")?;
    let root = doc.root_element();
    anyhow::ensure!(
        root.has_tag_name((DAV, "multistatus")),
        "The address did not return a CalDAV calendar listing."
    );
    let mut result = Properties::default();
    for (index, response) in root
        .children()
        .filter(|n| n.has_tag_name((DAV, "response")))
        .enumerate()
    {
        anyhow::ensure!(
            index < 2000,
            "The calendar listing is too large. Enter a calendar home or collection URL."
        );
        let url = resolve(
            base,
            child(response, DAV, "href")
                .and_then(|n| n.text())
                .context("A calendar listing is missing its address.")?,
        )?;
        // A Depth:0 answer may describe only the requested resource.
        anyhow::ensure!(
            depth != 0
                || (url.path().trim_end_matches('/') == base.path().trim_end_matches('/')
                    && url.query() == base.query()),
            "The server returned properties for a different calendar address."
        );
        if let Some(status) = child(response, DAV, "status") {
            anyhow::ensure!(
                code(status).is_some_and(|s| (200..300).contains(&s)),
                "The calendar listing is incomplete. Check access and try again."
            );
        }
        let mut props = Vec::new();
        for ps in response
            .children()
            .filter(|n| n.has_tag_name((DAV, "propstat")))
        {
            let status = child(ps, DAV, "status")
                .and_then(code)
                .context("A calendar property response is missing its status.")?;
            if status == 200 {
                if let Some(prop) = child(ps, DAV, "prop") {
                    props.extend(prop.children().filter(Node::is_element));
                }
            } else {
                // Individual optional properties are commonly unsupported or protected.
                anyhow::ensure!(
                    matches!(status, 403 | 404),
                    "The server could not read calendar properties. Try again."
                );
            }
        }
        let prop = |ns, name| props.iter().copied().find(|n| n.has_tag_name((ns, name)));
        if let Some(principal) = prop(DAV, "current-user-principal")
            && let Some(href) = child(principal, DAV, "href").and_then(|n| n.text())
        {
            result.principals.push(resolve(base, href)?);
        }
        if let Some(home) = prop(CAL, "calendar-home-set") {
            for href in home
                .children()
                .filter(|n| n.has_tag_name((DAV, "href")))
                .filter_map(|n| n.text())
            {
                result.homes.push(resolve(base, href)?);
            }
        }
        let calendar =
            prop(DAV, "resourcetype").is_some_and(|p| child(p, CAL, "calendar").is_some());
        let events = prop(CAL, "supported-calendar-component-set").is_none_or(|p| {
            p.children()
                .any(|n| n.has_tag_name((CAL, "comp")) && n.attribute("name") == Some("VEVENT"))
        });
        if !calendar || !events {
            continue;
        }
        let access = prop(DAV, "current-user-privilege-set")
            .map(|p| {
                let has = |name| {
                    p.children()
                        .filter(|n| n.has_tag_name((DAV, "privilege")))
                        .any(|p| child(p, DAV, name).is_some())
                };
                let write = has("all") || has("write");
                CalendarAccess {
                    create: write || has("bind"),
                    update: write || has("write-content"),
                    delete: write || has("unbind"),
                }
            })
            .unwrap_or_default();
        let name = prop(DAV, "displayname")
            .and_then(|p| p.text())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                url.path()
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("Calendar")
                    .to_owned()
            });
        result.calendars.push(DiscoveredCalendar {
            name: name.chars().take(512).collect(),
            url: url.to_string(),
            access,
        });
    }
    Ok(result)
}

impl CalDav {
    pub async fn discover(
        &self,
        input: &str,
        username: &str,
        secret: &str,
    ) -> anyhow::Result<Vec<DiscoveredCalendar>> {
        let origin = validate_caldav_url(input.trim())?;
        anyhow::ensure!(
            !username.trim().is_empty() && !secret.is_empty(),
            "Enter your calendar username and password, then find calendars."
        );
        let mut pending = VecDeque::from([(origin.clone(), 0u8)]);
        let mut seen = HashSet::new();
        let mut calendars = BTreeMap::new();
        let mut requests = 0;
        let mut fallback = false;
        while let Some((mut url, depth)) = pending.pop_front() {
            if !seen.insert((url.to_string(), depth)) {
                continue;
            }
            let response = loop {
                requests += 1;
                anyhow::ensure!(
                    requests <= 32,
                    "Calendar discovery reached too many addresses. Enter your calendar home or collection URL."
                );
                let response = self
                    .http
                    .request(reqwest::Method::from_bytes(b"PROPFIND")?, url.clone())
                    .basic_auth(username, Some(secret))
                    .header("Depth", depth.to_string())
                    .header("Content-Type", "application/xml; charset=utf-8")
                    .body(PROPS)
                    .send()
                    .await?;
                if response.status().is_redirection() {
                    let next = response
                        .headers()
                        .get(reqwest::header::LOCATION)
                        .and_then(|h| h.to_str().ok())
                        .context("The calendar server redirected without providing an address.")?;
                    let next = resolve(&url, next)?;
                    anyhow::ensure!(
                        seen.insert((next.to_string(), depth)),
                        "The calendar server returned a redirect loop."
                    );
                    url = next;
                    continue;
                }
                break response;
            };
            let status = response.status();
            anyhow::ensure!(
                !matches!(status.as_u16(), 401 | 403),
                "Calendar sign-in failed. Check the username, app password and calendar access."
            );
            if matches!(status.as_u16(), 200 | 404 | 405) && depth == 0 && calendars.is_empty() {
                if !fallback {
                    fallback = true;
                    pending.push_back((origin.join("/.well-known/caldav")?, 0));
                } else if url.path() == "/.well-known/caldav" {
                    pending.push_back((origin.join("/")?, 0));
                }
                continue;
            }
            anyhow::ensure!(
                status.as_u16() == 207,
                "The address did not return CalDAV properties (HTTP {}). Enter your server's CalDAV address.",
                status.as_u16()
            );
            let xml = response_text(response).await?;
            let parsed_url = url.clone();
            let properties =
                tokio::task::spawn_blocking(move || parse(&xml, &parsed_url, depth)).await??;
            let found = !properties.calendars.is_empty();
            for calendar in properties.calendars {
                calendars.insert(calendar.url.clone(), calendar);
            }
            anyhow::ensure!(
                calendars.len() <= 256,
                "More than 256 calendars were found. Enter a more specific calendar home URL."
            );
            // An explicitly entered collection should select only that collection.
            if found && depth == 0 {
                return Ok(calendars.into_values().collect());
            }
            if !properties.homes.is_empty() {
                for home in properties.homes {
                    pending.push_back((home, 1));
                }
            } else if depth == 0 {
                let mut followed = false;
                for principal in properties.principals {
                    if !seen.contains(&(principal.to_string(), 0)) {
                        followed = true;
                        pending.push_back((principal, 0));
                    }
                }
                if !followed {
                    // Direct calendar-home input (including Radicale user roots).
                    pending.push_back((url, 1));
                }
            }
            if pending.is_empty() && calendars.is_empty() && !fallback {
                fallback = true;
                pending.push_back((origin.join("/.well-known/caldav")?, 0));
            }
        }
        anyhow::ensure!(
            !calendars.is_empty(),
            "No event calendars were found. Enter your calendar home or collection URL and check this user's access."
        );
        Ok(calendars.into_values().collect())
    }
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
