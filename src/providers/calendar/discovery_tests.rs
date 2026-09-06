use super::*;
use crate::providers::calendar::test_server::{Reply, Server, client};

fn xml(rows: &str) -> String {
    format!(
        r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">{rows}</d:multistatus>"#
    )
}
fn resource(href: &str, props: &str) -> String {
    format!(
        r#"<d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat><d:propstat><d:prop><d:unknown/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat></d:response>"#
    )
}
fn calendar(href: &str, name: &str, privileges: &str, component: &str) -> String {
    resource(
        href,
        &format!(
            r#"<d:resourcetype><d:collection/><c:calendar/></d:resourcetype><d:displayname>{name}</d:displayname><c:supported-calendar-component-set><c:comp name="{component}"/></c:supported-calendar-component-set><d:current-user-privilege-set>{privileges}</d:current-user-privilege-set>"#
        ),
    )
}
#[tokio::test]
async fn discovers_through_well_known_principal_and_multiple_calendar_homes() {
    let mut server = Server::start(vec![
        Reply::new(200, "<html>Welcome</html>"),
        Reply::new(301, "").header("Location", "/dav/"),
        Reply::new(207, xml(&resource("/dav/", "<d:current-user-principal><d:href>/principals/me/</d:href></d:current-user-principal>"))),
        Reply::new(207, xml(&resource("/principals/me/", "<c:calendar-home-set><d:href>/home/me/</d:href><d:href>/shared/</d:href></c:calendar-home-set>"))),
        Reply::new(207, xml(&(calendar("/home/me/events/", "My &amp; Home", "<d:privilege><d:all/></d:privilege>", "VEVENT") + &calendar("/home/me/tasks/", "Tasks", "", "VTODO")))),
        Reply::new(207, xml(&calendar("/shared/team/", "Team holidays", "<d:privilege><d:read/></d:privilege>", "VEVENT"))),
    ]).await;
    let found = CalDav { http: client() }
        .discover(server.url.as_str(), "alice", "fixture-secret")
        .await
        .unwrap();
    server.finish().await;
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].name, "My & Home");
    assert_eq!(found[0].access, CalendarAccess::default());
    assert!(found[1].access.read_only());
    let requests = server.requests();
    assert_eq!(
        requests
            .iter()
            .map(|r| r.target.as_str())
            .collect::<Vec<_>>(),
        [
            "/calendars/",
            "/.well-known/caldav",
            "/dav/",
            "/principals/me/",
            "/home/me/",
            "/shared/"
        ]
    );
    assert!(requests.iter().all(|r| r.method == "PROPFIND"
        && r.headers["authorization"].starts_with("Basic ")
        && r.body.contains("current-user-privilege-set")));
    assert_eq!(requests[3].headers["depth"], "0");
    assert_eq!(requests[4].headers["depth"], "1");
}
#[tokio::test]
async fn direct_collection_preserves_canonical_url_and_partial_permissions() {
    let mut server = Server::start(vec![Reply::new(
        207,
        xml(&calendar(
            "/calendars/",
            "Edit existing",
            "<d:privilege><d:write-content/></d:privilege>",
            "VEVENT",
        )),
    )])
    .await;
    let input = server.url.as_str().trim_end_matches('/');
    let found = CalDav { http: client() }
        .discover(input, "alice", "secret")
        .await
        .unwrap();
    server.finish().await;
    assert_eq!(found[0].url, server.url.as_str());
    assert_eq!(
        found[0].access,
        CalendarAccess {
            create: false,
            update: true,
            delete: false
        }
    );
    assert_eq!(server.requests().len(), 1);
}
#[tokio::test]
async fn direct_home_lists_children_and_well_known_can_fall_back_to_root() {
    let mut server = Server::start(vec![
        Reply::new(404, ""),
        Reply::new(404, ""),
        Reply::new(
            207,
            xml(&resource(
                "/",
                "<d:resourcetype><d:collection/></d:resourcetype>",
            )),
        ),
        Reply::new(
            207,
            xml(&calendar(
                "/personal/",
                "Personal",
                "<d:privilege><d:write/></d:privilege>",
                "VEVENT",
            )),
        ),
    ])
    .await;
    let found = CalDav { http: client() }
        .discover(server.url.as_str(), "alice", "secret")
        .await
        .unwrap();
    server.finish().await;
    assert_eq!(found.len(), 1);
    assert_eq!(server.requests().last().unwrap().headers["depth"], "1");
}
#[tokio::test]
async fn never_forwards_credentials_across_origins_and_rejects_redirect_loops() {
    for replies in [
        vec![Reply::new(302, "").header("Location", "https://other.example/dav/")],
        vec![Reply::new(
            207,
            xml(&resource(
                "/calendars/",
                "<d:current-user-principal><d:href>https://other.example/principal/</d:href></d:current-user-principal>",
            )),
        )],
        vec![Reply::new(302, "").header("Location", "/calendars/")],
    ] {
        let mut server = Server::start(replies).await;
        assert!(
            CalDav { http: client() }
                .discover(server.url.as_str(), "alice", "secret")
                .await
                .is_err()
        );
        server.finish().await;
        assert_eq!(server.requests().len(), 1);
    }
}
#[tokio::test]
async fn invalid_or_partial_properties_never_become_successful_discovery() {
    for reply in [
        Reply::new(401, "secret from remote"),
        Reply::new(403, "secret from remote"),
        Reply::new(207, "<html/>"),
        Reply::new(207, "malformed"),
        Reply::new(
            207,
            xml(
                "<d:response><d:href>/calendars/</d:href><d:status>HTTP/1.1 500 Error</d:status></d:response>",
            ),
        ),
        Reply::new(
            207,
            xml(&calendar(
                "/another/",
                "Wrong depth zero resource",
                "",
                "VEVENT",
            )),
        ),
        Reply::new(207, "").header("Content-Length", "16777217"),
    ] {
        let mut server = Server::start(vec![reply]).await;
        let error = CalDav { http: client() }
            .discover(server.url.as_str(), "alice", "secret")
            .await
            .unwrap_err();
        assert!(!format!("{error:#}").contains("secret from remote"));
        server.finish().await;
    }
}
