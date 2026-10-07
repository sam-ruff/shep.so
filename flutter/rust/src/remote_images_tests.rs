use crate::{
    api::MobileProfile,
    operations,
    tests::{profile, request, seed},
};
use async_trait::async_trait;
use base64::Engine;
use serde_json::{Value, json};
use shep_mail_core::{
    model::parse_mail,
    remote_images::{FetchError, MockTransport, Reply, Transport},
};
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::{Notify, mpsc};

const PIXEL: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

fn pixel() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(PIXEL)
        .expect("Fixture pixel")
}

fn public(port: u16) -> Vec<SocketAddr> {
    vec![SocketAddr::from(([93, 184, 216, 34], port))]
}

async fn cached_html(profile: &MobileProfile) -> String {
    seed(profile, 0).await;
    profile
        .database
        .write(|db| {
            let raw = b"From: News <News@Example.TEST>\r\nTo: alex@example.test\r\nSubject: Images\r\nContent-Type: text/html\r\n\r\n<p>Before</p><img src=\"https://images.example.test/a.png\" alt=\"A\"><p style=\"background:url(https://images.example.test/b.png)\">After</p>".to_vec();
            let mail = parse_mail("fixture", "7", "INBOX", raw, true, false)?;
            let id = mail.summary.id.clone();
            operations::insert_mail(db, mail, false)?;
            Ok(id)
        })
        .await
        .expect("Cached message")
}

async fn reply(profile: &MobileProfile, payload: Value) -> Value {
    serde_json::from_str(
        &profile
            .request(payload.to_string())
            .await
            .expect("Bridge reply"),
    )
    .expect("Reply JSON")
}

fn allow_sender() -> Value {
    json!({"policy": "BlockAll", "senders": ["news@example.test"]})
}

async fn keys(profile: &MobileProfile, id: &str) -> Vec<String> {
    let prepared = request(
        profile,
        json!({"op": "formatted", "id": id, "options": {"generation": "g1", "remote_placeholders": true}}),
    )
    .await;
    assert_eq!(prepared["sender_address"], "news@example.test");
    assert_eq!(prepared["sender_domain"], "example.test");
    let document = prepared["document"].as_str().expect("Document");
    assert!(!document.contains("images.example.test"));
    prepared["remote_images"]
        .as_array()
        .expect("Remote images")
        .iter()
        .map(|image| {
            let key = image["key"].as_str().expect("Key").to_owned();
            assert!(document.contains(&format!("urn:shep-remote:{key}")));
            key
        })
        .collect()
}

fn print_images(printed: &Value) -> usize {
    let document = printed["document"].as_str().expect("Print document");
    let start = document
        .find("<script id=\"shep-print-data\" type=\"application/json\">")
        .expect("Print data")
        + "<script id=\"shep-print-data\" type=\"application/json\">".len();
    let end = start + document[start..].find("</script>").expect("Print data end");
    let data: Value = serde_json::from_str(&document[start..end]).expect("Print JSON");
    data["images"].as_object().expect("Images").len()
}

fn serving(times: usize) -> MockTransport {
    let mut transport = MockTransport::new();
    transport
        .expect_resolve()
        .times(times)
        .returning(|_, port| Ok(public(port)));
    transport.expect_get().times(times).returning(|_, _, _| {
        Ok(Reply {
            status: 200,
            location: None,
            body: pixel(),
        })
    });
    transport
}

#[tokio::test]
async fn permitted_images_come_only_from_discovered_urls_and_are_cached() {
    let (_dir, profile) = profile().await;
    let id = cached_html(&profile).await;
    let keys = keys(&profile, &id).await;
    assert_eq!(keys.len(), 2);
    profile
        .operations
        .remote_images
        .set_transport(Arc::new(serving(2)));
    for _ in 0..2 {
        let loaded = request(
            &profile,
            json!({"op": "remote_images", "id": id, "keys": keys, "rules": allow_sender()}),
        )
        .await;
        assert!(loaded["failed"].as_object().expect("Failed").is_empty());
        for key in &keys {
            let image = &loaded["images"][key];
            assert_eq!(
                (image["width"].as_u64(), image["height"].as_u64()),
                (Some(1), Some(1))
            );
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(image["bytes"].as_str().expect("Bytes"))
                .expect("Base64");
            assert_eq!(&bytes[8..12], b"WEBP");
        }
    }
    for rules in [
        json!({"policy": "BlockAll", "domains": ["example.test"]}),
        json!({"policy": "Contacts", "contacts": ["NEWS@example.test"]}),
        json!({"policy": "AllowAll"}),
        json!({"policy": "BlockAll", "messages": [id]}),
    ] {
        request(
            &profile,
            json!({"op": "remote_images", "id": id, "keys": [keys[0]], "rules": rules}),
        )
        .await;
    }
}

#[tokio::test]
async fn blocked_messages_and_foreign_or_invalid_keys_never_reach_the_network() {
    let (_dir, profile) = profile().await;
    let id = cached_html(&profile).await;
    let keys = keys(&profile, &id).await;
    profile
        .operations
        .remote_images
        .set_transport(Arc::new(serving(0)));
    let foreign = format!("{:064x}", 7);
    for (keys, rules, message) in [
        (
            keys.clone(),
            json!({"policy": "BlockAll"}),
            "blocked for this message",
        ),
        (
            keys.clone(),
            json!({"policy": "Contacts"}),
            "blocked for this message",
        ),
        (
            keys.clone(),
            json!({"policy": "BlockAll", "domains": ["mail.example.test"]}),
            "blocked for this message",
        ),
        (
            keys.clone(),
            json!({"policy": "BlockAll", "messages": ["another"]}),
            "blocked for this message",
        ),
        (vec![foreign], allow_sender(), "not part of the message"),
        (
            vec!["../a.png".into()],
            allow_sender(),
            "Invalid image request",
        ),
        (Vec::new(), allow_sender(), "Invalid image request"),
        (
            vec![keys[0].clone(); 9],
            allow_sender(),
            "Invalid image request",
        ),
    ] {
        let refused = reply(
            &profile,
            json!({"op": "remote_images", "id": id, "keys": keys, "rules": rules}),
        )
        .await;
        assert!(
            refused["error"]
                .as_str()
                .is_some_and(|error| error.contains(message)),
            "{refused}"
        );
    }
    let unknown = json!({"op": "remote_images", "id": id, "keys": [keys[0]], "rules": {"policy": "BlockAll", "hosts": ["a"]}});
    assert!(profile.request(unknown.to_string()).await.is_err());
}

#[tokio::test]
async fn failures_are_reported_per_image_with_fixed_messages() {
    let (_dir, profile) = profile().await;
    let id = cached_html(&profile).await;
    let keys = keys(&profile, &id).await;
    let mut transport = MockTransport::new();
    transport
        .expect_resolve()
        .returning(|_, port| Ok(public(port)));
    transport.expect_get().times(2).returning(|url, _, _| {
        Ok(if url.path() == "/a.png" {
            Reply {
                status: 403,
                location: None,
                body: b"private server detail".to_vec(),
            }
        } else {
            Reply {
                status: 302,
                location: Some("https://10.0.0.8/b.png".into()),
                body: Vec::new(),
            }
        })
    });
    profile
        .operations
        .remote_images
        .set_transport(Arc::new(transport));
    let loaded = request(
        &profile,
        json!({"op": "remote_images", "id": id, "keys": keys, "rules": allow_sender()}),
    )
    .await;
    assert!(loaded["images"].as_object().expect("Images").is_empty());
    let failed = loaded["failed"].as_object().expect("Failed");
    assert_eq!(failed.len(), 2);
    let messages: Vec<_> = failed.values().filter_map(Value::as_str).collect();
    assert!(messages.contains(&FetchError::Rejected.to_string().as_str()));
    assert!(messages.contains(&FetchError::PrivateAddress.to_string().as_str()));
    assert!(!loaded.to_string().contains("private server detail"));
}

#[tokio::test]
async fn print_includes_only_cached_images_of_a_permitted_message() {
    let (_dir, profile) = profile().await;
    let id = cached_html(&profile).await;
    let keys = keys(&profile, &id).await;
    let print = |images: Option<Value>| {
        let mut payload = json!({"op": "print", "id": id, "options": {"generation": "p1"}});
        if let Some(images) = images {
            payload["images"] = images;
        }
        payload
    };
    profile
        .operations
        .remote_images
        .set_transport(Arc::new(serving(1)));
    assert_eq!(
        print_images(&request(&profile, print(Some(allow_sender()))).await),
        0
    );
    request(
        &profile,
        json!({"op": "remote_images", "id": id, "keys": [keys[0]], "rules": allow_sender()}),
    )
    .await;
    assert_eq!(
        print_images(&request(&profile, print(Some(allow_sender()))).await),
        1
    );
    assert_eq!(print_images(&request(&profile, print(None)).await), 0);
    assert_eq!(
        print_images(&request(&profile, print(Some(json!({"policy": "BlockAll"})))).await),
        0
    );
    request(&profile, json!({"op": "forget_remote_images"})).await;
    assert_eq!(
        print_images(&request(&profile, print(Some(allow_sender()))).await),
        0
    );
}

/// Holds the first GET until the test releases it.
struct Held {
    started: mpsc::Sender<()>,
    release: Arc<Notify>,
}

#[async_trait]
impl Transport for Held {
    async fn resolve(&self, _: &str, port: u16) -> Result<Vec<SocketAddr>, FetchError> {
        Ok(public(port))
    }
    async fn get(&self, _: &reqwest::Url, _: &[SocketAddr], _: usize) -> Result<Reply, FetchError> {
        let _ = self.started.send(()).await;
        self.release.notified().await;
        Ok(Reply {
            status: 200,
            location: None,
            body: pixel(),
        })
    }
}

#[tokio::test]
async fn revocation_fences_a_running_fetch_and_its_cache_entry() {
    let (_dir, profile) = profile().await;
    let id = cached_html(&profile).await;
    let keys = keys(&profile, &id).await;
    let (started, mut running) = mpsc::channel(1);
    let release = Arc::new(Notify::new());
    profile
        .operations
        .remote_images
        .set_transport(Arc::new(Held {
            started,
            release: release.clone(),
        }));
    let pending = {
        let payload =
            json!({"op": "remote_images", "id": id, "keys": [keys[0]], "rules": allow_sender()});
        let profile = MobileProfile {
            database: profile.database.clone(),
            operations: profile.operations.clone(),
        };
        tokio::spawn(async move { reply(&profile, payload).await })
    };
    running.recv().await.expect("Fetch started");
    request(&profile, json!({"op": "forget_remote_images"})).await;
    release.notify_one();
    let refused = pending.await.expect("Request task");
    assert!(
        refused["error"]
            .as_str()
            .is_some_and(|error| error.contains("permission changed")),
        "{refused}"
    );
    assert!(!refused.to_string().contains("WEBP"));
    let printed = request(
        &profile,
        json!({"op": "print", "id": id, "options": {"generation": "p2"}, "images": allow_sender()}),
    )
    .await;
    assert_eq!(print_images(&printed), 0);
}
