//! Loopback-only checks of the production transport: real sockets and TLS,
//! fictional hostnames pinned to 127.0.0.1, never an external host.
use super::super::*;
use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        self,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    },
};

#[derive(Default)]
struct Seen {
    connections: usize,
    requests: Vec<String>,
}

/// Records the request before replying, so a client that has its reply can
/// rely on the observation.
async fn answer(
    mut stream: impl AsyncRead + AsyncWrite + Unpin,
    response: &[u8],
    record: &Mutex<Seen>,
) {
    let mut request = Vec::new();
    let mut buffer = [0; 1024];
    while !request.ends_with(b"\r\n\r\n") && request.len() < 16 * 1024 {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => request.extend_from_slice(&buffer[..read]),
        }
    }
    record
        .lock()
        .expect("Record")
        .requests
        .push(String::from_utf8_lossy(&request).to_lowercase());
    let _ = stream.write_all(response).await;
    let _ = stream.shutdown().await;
}

async fn serve(
    response: &'static [u8],
    tls: Option<TlsAcceptor>,
) -> (SocketAddr, Arc<Mutex<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Loopback listener");
    let address = listener.local_addr().expect("Loopback address");
    let seen = Arc::new(Mutex::new(Seen::default()));
    let record = seen.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            record.lock().expect("Record").connections += 1;
            let record = record.clone();
            let tls = tls.clone();
            tokio::spawn(async move {
                match tls {
                    Some(tls) => {
                        if let Ok(stream) = tls.accept(stream).await {
                            answer(stream, response, &record).await;
                        }
                    }
                    None => answer(stream, response, &record).await,
                }
            });
        }
    });
    (address, seen)
}

fn pinned(address: SocketAddr) -> Vec<SocketAddr> {
    vec![address]
}

fn url(value: &str) -> reqwest::Url {
    reqwest::Url::parse(value).expect("Fixture URL")
}

#[tokio::test]
async fn pinned_requests_keep_the_hostname_and_send_no_identity() {
    let (address, seen) = serve(
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nSet-Cookie: id=1\r\n\r\nimage",
        None,
    )
    .await;
    let transport = ReqwestTransport::new();
    let target = url(&format!("http://images.test:{}/a.png", address.port()));
    for _ in 0..2 {
        let reply = transport
            .get(&target, &pinned(address), 64)
            .await
            .expect("Loopback reply");
        assert_eq!(
            (reply.status, reply.body.as_slice()),
            (200, b"image".as_slice())
        );
    }
    let seen = seen.lock().expect("Seen");
    assert_eq!(seen.requests.len(), 2);
    for request in &seen.requests {
        assert!(request.contains(&format!("host: images.test:{}", address.port())));
        assert!(request.contains("accept: image/webp,image/png,image/jpeg,image/gif"));
        for header in ["cookie:", "referer:", "authorization:", "proxy-"] {
            assert!(!request.contains(header), "{header}");
        }
    }
}

#[tokio::test]
async fn redirects_are_returned_without_being_followed() {
    let (address, seen) = serve(
        b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/secret\r\nContent-Length: 0\r\n\r\n",
        None,
    )
    .await;
    let reply = ReqwestTransport::new()
        .get(
            &url(&format!("http://images.test:{}/a.png", address.port())),
            &pinned(address),
            64,
        )
        .await
        .expect("Redirect reply");
    assert_eq!(reply.status, 302);
    assert_eq!(reply.location.as_deref(), Some("http://127.0.0.1/secret"));
    assert_eq!(seen.lock().expect("Seen").connections, 1);
}

#[tokio::test]
async fn declared_and_streamed_bodies_are_bounded() {
    for response in [
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n".as_slice(),
        b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n0123456789012345678901234567890123456789012345678901234567890123456789",
    ] {
        let (address, _) = serve(response, None).await;
        assert_eq!(
            ReqwestTransport::new()
                .get(
                    &url(&format!("http://images.test:{}/a.png", address.port())),
                    &pinned(address),
                    50,
                )
                .await,
            Err(FetchError::TooLarge)
        );
    }
}

#[tokio::test]
async fn the_production_service_refuses_loopback_before_connecting() {
    let (address, seen) = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n", None).await;
    let transport = ReqwestTransport::new();
    for target in [
        format!("http://127.0.0.1:{}/a.png", address.port()),
        format!("http://localhost:{}/a.png", address.port()),
    ] {
        assert_eq!(
            download(&transport, &target).await,
            Err(FetchError::PrivateAddress),
            "{target}"
        );
    }
    assert_eq!(seen.lock().expect("Seen").connections, 0);
}

fn authority() -> (rcgen::Certificate, TlsAcceptor) {
    let mut ca = rcgen::CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
    ca.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().expect("CA key");
    let ca_cert = ca.self_signed(&ca_key).expect("CA certificate");
    let issuer = rcgen::Issuer::new(ca, ca_key);
    let leaf_key = rcgen::KeyPair::generate().expect("Leaf key");
    let leaf = rcgen::CertificateParams::new(vec!["images.test".to_owned()])
        .expect("Leaf parameters")
        .signed_by(&leaf_key, &issuer)
        .expect("Leaf certificate");
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("Protocol versions")
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(leaf.der().to_vec())],
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
    )
    .expect("Server configuration");
    (ca_cert, TlsAcceptor::from(Arc::new(config)))
}

#[tokio::test]
async fn tls_requires_a_trusted_certificate_for_the_requested_host() {
    let (ca, acceptor) = authority();
    let (address, seen) = serve(
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nimage",
        Some(acceptor),
    )
    .await;
    let images = url(&format!("https://images.test:{}/a.png", address.port()));
    assert_eq!(
        ReqwestTransport::new()
            .get(&images, &pinned(address), 64)
            .await,
        Err(FetchError::Unavailable),
        "The bundled web roots do not trust the fixture authority"
    );
    let trusting =
        ReqwestTransport::trusting(reqwest::Certificate::from_der(ca.der()).expect("Fixture root"));
    let reply = trusting
        .get(&images, &pinned(address), 64)
        .await
        .expect("Trusted loopback reply");
    assert_eq!(reply.body, b"image");
    let other = url(&format!("https://other.test:{}/a.png", address.port()));
    assert_eq!(
        trusting.get(&other, &pinned(address), 64).await,
        Err(FetchError::Unavailable),
        "A pinned address keeps hostname verification"
    );
    assert_eq!(seen.lock().expect("Seen").requests.len(), 1);
}
