use super::response_json;
use crate::model::{CalendarAccess, CalendarKind, CalendarSource};
use anyhow::Context;
use std::collections::HashSet;

pub(crate) async fn list(
    http: &reqwest::Client,
    endpoint: url::Url,
    token: &str,
) -> anyhow::Result<Vec<CalendarSource>> {
    let mut next = String::new();
    let mut pages = HashSet::new();
    let mut ids = HashSet::new();
    let mut sources = Vec::new();
    loop {
        anyhow::ensure!(
            pages.len() < 100 && pages.insert(next.clone()),
            "Google repeated a calendar page. Try syncing again."
        );
        let data = response_json(
            http.get(endpoint.clone())
                .bearer_auth(token)
                .query(&[
                    ("maxResults", "250"),
                    ("minAccessRole", "reader"),
                    ("pageToken", next.as_str()),
                ])
                .send()
                .await?,
        )
        .await?;
        anyhow::ensure!(
            data.is_object() && (data["items"].is_null() || data["items"].is_array()),
            "Google returned an invalid calendar list."
        );
        if let Some(items) = data["items"].as_array() {
            for item in items {
                let role = item["accessRole"]
                    .as_str()
                    .context("Google did not report calendar access. Try syncing again.")?;
                if matches!(role, "freeBusyReader" | "none") || item["deleted"] == true {
                    continue;
                }
                let access = match role {
                    "reader" => CalendarAccess::READ_ONLY,
                    "writer" | "writerWithoutPrivateAccess" | "owner" => CalendarAccess::default(),
                    _ => anyhow::bail!("Google returned an unknown calendar access role."),
                };
                let id = item["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 4096)
                    .context("Google returned an invalid calendar identity.")?;
                anyhow::ensure!(
                    ids.insert(id.to_string()),
                    "Google returned a duplicate calendar. Try syncing again."
                );
                sources.push(CalendarSource {
                    id: format!("google:{id}"),
                    name: item["summaryOverride"]
                        .as_str()
                        .or_else(|| item["summary"].as_str())
                        .unwrap_or("Google Calendar")
                        .chars()
                        .take(512)
                        .collect(),
                    kind: CalendarKind::Google,
                    url: id.into(),
                    username: String::new(),
                    access,
                });
                anyhow::ensure!(
                    sources.len() <= 2000,
                    "Google returned more than 2,000 calendars."
                );
            }
        }
        match data.get("nextPageToken") {
            None | Some(serde_json::Value::Null) => return Ok(sources),
            Some(serde_json::Value::String(token)) if token.is_empty() => return Ok(sources),
            Some(serde_json::Value::String(token)) if token.len() <= 4096 => next = token.clone(),
            _ => anyhow::bail!("Google returned an invalid calendar page token."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::calendar::test_server::{Reply, Server, client};
    #[tokio::test]
    async fn google_calendar_list_pages_roles_and_filters_free_busy() {
        let mut server = Server::start(vec![
            Reply::new(200, r#"{"items":[{"id":"work","summary":"Work","accessRole":"writer"}],"nextPageToken":"next"}"#),
            Reply::new(200, r#"{"items":[{"id":"holidays","summary":"Holidays","accessRole":"reader"},{"id":"busy","accessRole":"freeBusyReader"},{"id":"shared","accessRole":"writerWithoutPrivateAccess"}]}"#),
        ]).await;
        let sources = list(&client(), server.url.clone(), "fixture-token")
            .await
            .unwrap();
        server.finish().await;
        assert_eq!(sources.len(), 3);
        assert!(sources[0].access.create && sources[0].access.update && sources[0].access.delete);
        assert!(sources[1].access.read_only());
        assert!(sources[2].access.update);
        assert!(server.requests()[1].target.contains("pageToken=next"));
        assert!(server.requests()[0].target.contains("minAccessRole=reader"));
    }
    #[tokio::test]
    async fn google_calendar_list_rejects_loops_duplicates_missing_roles_and_bad_responses() {
        for replies in [
            vec![
                Reply::new(200, r#"{"nextPageToken":"x"}"#),
                Reply::new(200, r#"{"nextPageToken":"x"}"#),
            ],
            vec![Reply::new(
                200,
                r#"{"items":[{"id":"a","accessRole":"owner"},{"id":"a","accessRole":"reader"}]}"#,
            )],
            vec![Reply::new(200, r#"{"items":[{"id":"a"}]}"#)],
            vec![Reply::new(200, r#"{"items":5}"#)],
            vec![Reply::new(302, "").header("Location", "https://other.example")],
            vec![Reply::new(403, "remote secret")],
            vec![Reply::new(200, "").header("Content-Length", "16777217")],
        ] {
            let mut server = Server::start(replies).await;
            let error = list(&client(), server.url.clone(), "fixture-token")
                .await
                .unwrap_err();
            assert!(!format!("{error:#}").contains("remote secret"));
            server.finish().await;
        }
    }
}
