use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Cases {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    rules: OwnedRules,
    message: String,
    sender: String,
    address: Option<String>,
    domain: Option<String>,
    allowed: bool,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct OwnedRules {
    policy: ImagePolicy,
    messages: Vec<String>,
    senders: Vec<String>,
    domains: Vec<String>,
    contacts: Vec<String>,
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(width, height)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("Encode a synthetic PNG");
    bytes.into_inner()
}

#[test]
fn shared_policy_cases_agree_with_every_client() {
    let cases: Cases = serde_json::from_str(include_str!("../../../remote-image-cases.json"))
        .expect("Read the shared remote image cases");
    assert!(cases.cases.len() >= 10);
    for case in cases.cases {
        let rules = Rules {
            policy: case.rules.policy,
            messages: &case.rules.messages,
            senders: &case.rules.senders,
            domains: &case.rules.domains,
            contacts: &case.rules.contacts,
        };
        assert_eq!(sender_address(&case.sender), case.address, "{}", case.name);
        assert_eq!(sender_domain(&case.sender), case.domain, "{}", case.name);
        assert_eq!(
            rules.allows(&case.message, &case.sender),
            case.allowed,
            "{}",
            case.name
        );
    }
}

#[test]
fn only_globally_routable_addresses_are_public() {
    for address in [
        "127.0.0.1",
        "10.0.0.1",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "198.18.0.1",
        "0.1.2.3",
        "224.0.0.1",
        "255.255.255.255",
        "192.0.2.1",
        "::1",
        "::",
        "fc00::1",
        "fe80::1",
        "2001:db8::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
    ] {
        assert!(!public_ip(address.parse().expect("Address")), "{address}");
    }
    for address in [
        "8.8.8.8",
        "93.184.216.34",
        "2606:4700::1111",
        "::ffff:8.8.8.8",
    ] {
        assert!(public_ip(address.parse().expect("Address")), "{address}");
    }
}

#[test]
fn conversion_keeps_small_images_and_bounds_large_ones() {
    let small = convert_to_webp(&png(32, 16)).expect("Small image");
    assert_eq!((small.width, small.height), (32, 16));
    assert_eq!(&small.bytes[8..12], b"WEBP");
    let scaled = convert_to_webp(&png(1500, 300)).expect("Scaled image");
    assert_eq!((scaled.width, scaled.height), (1024, 205));
    assert!(convert_to_webp(&png(4000, 10)).is_err());
    assert!(convert_to_webp(b"<svg xmlns='http://www.w3.org/2000/svg'/>").is_err());
}

#[cfg(feature = "remote-images")]
mod orchestration {
    use super::{super::*, png};
    use mockall::predicate::eq;
    use std::net::SocketAddr;

    fn public(port: u16) -> Vec<SocketAddr> {
        vec![SocketAddr::from(([93, 184, 216, 34], port))]
    }

    fn reply(status: u16, location: Option<&str>, body: Vec<u8>) -> Reply {
        Reply {
            status,
            location: location.map(str::to_owned),
            body,
        }
    }

    #[tokio::test]
    async fn a_public_image_is_downloaded_from_checked_addresses_and_converted() {
        let mut transport = MockTransport::new();
        transport
            .expect_resolve()
            .with(eq("images.example.com"), eq(443))
            .times(1)
            .returning(|_, port| Ok(public(port)));
        transport
            .expect_get()
            .withf(|url, addresses, limit| {
                url.as_str() == "https://images.example.com/banner.png"
                    && addresses == public(443).as_slice()
                    && *limit == MAX_DOWNLOAD_BYTES
            })
            .times(1)
            .returning(|_, _, _| Ok(reply(200, None, png(40, 20))));
        let image = fetch(&transport, "https://images.example.com/banner.png")
            .await
            .expect("Fetched image");
        assert_eq!((image.width, image.height), (40, 20));
    }

    #[tokio::test]
    async fn private_or_mixed_resolution_never_connects() {
        for resolved in [
            vec![SocketAddr::from(([127, 0, 0, 1], 80))],
            vec![
                SocketAddr::from(([93, 184, 216, 34], 80)),
                SocketAddr::from(([10, 0, 0, 7], 80)),
            ],
            vec![SocketAddr::from(([93, 184, 216, 34], 8080))],
            Vec::new(),
        ] {
            let mut transport = MockTransport::new();
            transport
                .expect_resolve()
                .times(1)
                .returning(move |_, _| Ok(resolved.clone()));
            transport.expect_get().times(0);
            assert_eq!(
                download(&transport, "http://tracker.example.com/pixel.gif").await,
                Err(FetchError::PrivateAddress)
            );
        }
    }

    #[tokio::test]
    async fn private_literals_are_rejected_without_resolving() {
        for url in [
            "http://10.0.0.1/a.png",
            "http://[::1]/a.png",
            "http://[::ffff:127.0.0.1]/a.png",
            "https://169.254.169.254/latest/meta-data",
        ] {
            let mut transport = MockTransport::new();
            transport.expect_resolve().times(0);
            transport.expect_get().times(0);
            assert_eq!(
                download(&transport, url).await,
                Err(FetchError::PrivateAddress),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn unsupported_addresses_make_no_request() {
        for url in [
            "ftp://images.example.com/a.png",
            "https://user:secret@images.example.com/a.png",
            "data:image/png;base64,AAAA",
            "file:///etc/passwd",
            "cid:banner",
            "not a url",
        ] {
            let mut transport = MockTransport::new();
            transport.expect_resolve().times(0);
            transport.expect_get().times(0);
            assert_eq!(
                download(&transport, url).await,
                Err(FetchError::Unsupported),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn every_redirect_is_resolved_and_checked_again() {
        let mut transport = MockTransport::new();
        transport
            .expect_resolve()
            .with(eq("images.example.com"), eq(443))
            .times(1)
            .returning(|_, port| Ok(public(port)));
        transport
            .expect_resolve()
            .with(eq("internal.example.com"), eq(443))
            .times(1)
            .returning(|_, port| Ok(vec![SocketAddr::from(([192, 168, 0, 4], port))]));
        transport.expect_get().times(1).returning(|_, _, _| {
            Ok(reply(
                302,
                Some("https://internal.example.com/x.png"),
                vec![],
            ))
        });
        assert_eq!(
            download(&transport, "https://images.example.com/a.png").await,
            Err(FetchError::PrivateAddress)
        );
    }

    #[tokio::test]
    async fn relative_redirects_are_followed_to_a_public_image() {
        let mut transport = MockTransport::new();
        transport
            .expect_resolve()
            .times(2)
            .returning(|_, port| Ok(public(port)));
        let mut sequence = mockall::Sequence::new();
        transport
            .expect_get()
            .withf(|url, _, _| url.path() == "/a.png")
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _, _| Ok(reply(301, Some("/b.png"), vec![])));
        transport
            .expect_get()
            .withf(|url, _, _| url.as_str() == "https://images.example.com/b.png")
            .times(1)
            .in_sequence(&mut sequence)
            .returning(|_, _, _| Ok(reply(200, None, vec![1, 2, 3])));
        assert_eq!(
            download(&transport, "https://images.example.com/a.png").await,
            Ok(vec![1, 2, 3])
        );
    }

    #[tokio::test]
    async fn redirects_cannot_downgrade_or_loop() {
        let mut transport = MockTransport::new();
        transport
            .expect_resolve()
            .returning(|_, port| Ok(public(port)));
        transport
            .expect_get()
            .times(1)
            .returning(|_, _, _| Ok(reply(307, Some("http://images.example.com/a.png"), vec![])));
        assert_eq!(
            download(&transport, "https://images.example.com/a.png").await,
            Err(FetchError::Unsupported)
        );
        let mut transport = MockTransport::new();
        transport
            .expect_resolve()
            .returning(|_, port| Ok(public(port)));
        transport
            .expect_get()
            .times(4)
            .returning(|_, _, _| Ok(reply(302, Some("/again.png"), vec![])));
        assert_eq!(
            download(&transport, "https://images.example.com/a.png").await,
            Err(FetchError::Redirects)
        );
    }

    #[tokio::test]
    async fn refusals_oversize_and_invalid_images_fail_with_fixed_messages() {
        for (answer, expected) in [
            (
                reply(404, None, b"Not found on secret-host".to_vec()),
                FetchError::Rejected,
            ),
            (reply(302, None, vec![]), FetchError::Rejected),
            (
                reply(200, None, vec![0; MAX_DOWNLOAD_BYTES + 1]),
                FetchError::TooLarge,
            ),
            (reply(200, None, b"<html>".to_vec()), FetchError::Invalid),
            (reply(200, None, png(4000, 4)), FetchError::Invalid),
        ] {
            let mut transport = MockTransport::new();
            transport
                .expect_resolve()
                .returning(|_, port| Ok(public(port)));
            transport
                .expect_get()
                .times(1)
                .returning(move |_, _, _| Ok(answer.clone()));
            let error = fetch(&transport, "https://images.example.com/a.png")
                .await
                .expect_err("Refused image");
            assert_eq!(error, expected);
            assert!(!error.to_string().contains("secret-host"));
        }
        let mut transport = MockTransport::new();
        transport
            .expect_resolve()
            .returning(|_, _| Err(FetchError::Unavailable));
        assert_eq!(
            fetch(&transport, "https://images.example.com/a.png").await,
            Err(FetchError::Unavailable)
        );
    }
}

#[cfg(feature = "remote-images")]
mod loopback;
