//! This identity provider exists only in the Rust test binary, never the server.
use super::*;
use crate::google::VerificationFailed;
use crate::mail::{
    Connection, HostedMail, MailHub,
    policy::{Endpoint, Service},
};
use async_trait::async_trait;
use shep_mail_core::{mail_actions::Flags, model::*, providers::mail::DeliveryFailure};
use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{Mutex, mpsc};
const MOVE_RAW:&[u8]=b"From: Mail fixture <sender@example.test>\r\nReply-To: Support <support@example.test>\r\nTo: mailbox@example.test, Peer <peer@example.test>\r\nCc: Peer <peer@example.test>, Copy <copy@example.test>\r\nMessage-ID: <original@example.test>\r\nReferences: <root@example.test>\r\nSubject: The beta transport fixture\r\nDate: Sun, 6 Sep 2026 10:00:00 +0000\r\n\r\nThis fictional message passed through the Rust mail API into browser storage.";

#[derive(Default)]
struct BrowserMail {
    unread: AtomicBool,
    starred: AtomicBool,
    sends: AtomicUsize,
    moves: AtomicUsize,
    location: Mutex<Option<(String, String)>>,
}
#[async_trait]
impl HostedMail for BrowserMail {
    async fn probe(&self, c: &Connection, _: bool) -> anyhow::Result<()> {
        use secrecy::ExposeSecret;
        anyhow::ensure!(
            c.password.expose_secret() == "synthetic-password",
            "Synthetic wrong password"
        );
        Ok(())
    }
    async fn sync(
        &self,
        c: &Connection,
        _: &HashSet<String>,
        folder: &str,
        output: mpsc::Sender<MailSyncItem>,
    ) -> anyhow::Result<Vec<String>> {
        self.probe(c, false).await?;
        output
            .send(MailSyncItem::Folders(
                c.account.id.clone(),
                vec!["INBOX".into(), "Archive".into()],
            ))
            .await?;
        let location = self
            .location
            .lock()
            .await
            .clone()
            .unwrap_or(("INBOX".into(), "42.7".into()));
        if folder == location.0 {
            let mail = parse_mail(
                &c.account.id,
                &location.1,
                folder,
                MOVE_RAW.to_vec(),
                self.unread.load(Ordering::SeqCst),
                self.starred.load(Ordering::SeqCst),
            )?;
            let id = mail.summary.id.clone();
            output.send(MailSyncItem::Message(mail)).await?;
            output
                .send(MailSyncItem::Reconcile {
                    account: c.account.id.clone(),
                    folder: folder.into(),
                    live_ids: [id].into(),
                })
                .await?;
        } else {
            output
                .send(MailSyncItem::Reconcile {
                    account: c.account.id.clone(),
                    folder: folder.into(),
                    live_ids: HashSet::new(),
                })
                .await?;
        }
        Ok(vec!["INBOX".into(), "Archive".into()])
    }
    async fn flags(&self, c: &Connection, _: &Mail, flags: Flags) -> anyhow::Result<()> {
        self.probe(c, false).await?;
        if let Some(value) = flags.unread {
            self.unread.store(value, Ordering::SeqCst);
        }
        if let Some(value) = flags.starred {
            self.starred.store(value, Ordering::SeqCst);
        }
        Ok(())
    }
    async fn move_mail(
        &self,
        c: &Connection,
        mail: &Mail,
        folder: &str,
    ) -> anyhow::Result<Option<String>> {
        self.probe(c, false).await?;
        let mut location = self.location.lock().await;
        let current = location.clone().unwrap_or(("INBOX".into(), "42.7".into()));
        anyhow::ensure!(
            current == (mail.folder.clone(), mail.remote_id.clone()),
            "A stale UID must never reach the mutation transport"
        );
        anyhow::ensure!(folder != "Trash", "Move rejection fixture");
        let uid = self.moves.fetch_add(1, Ordering::SeqCst) + 20;
        let remote = format!("{}.{uid}", if folder == "INBOX" { 42 } else { 91 });
        *location = Some((folder.into(), remote.clone()));
        Ok(Some(remote))
    }
    async fn resolve_move(
        &self,
        c: &Connection,
        receipt: &shep_mail_core::mail_actions::MoveReceipt,
    ) -> anyhow::Result<Mail> {
        self.probe(c, false).await?;
        let location = self.location.lock().await.clone().unwrap();
        anyhow::ensure!(
            receipt.folder == location.0
                && receipt
                    .fingerprint
                    .as_ref()
                    .is_some_and(|f| f.matches(MOVE_RAW)),
            "Invalid recovery proof"
        );
        if let Some(current) = &receipt.current {
            anyhow::ensure!(current.remote_id == location.1, "Stale recovery UID");
        }
        Ok(parse_mail(
            &c.account.id,
            &location.1,
            &location.0,
            MOVE_RAW.to_vec(),
            self.unread.load(Ordering::SeqCst),
            self.starred.load(Ordering::SeqCst),
        )?
        .summary)
    }
    async fn send(
        &self,
        c: &Connection,
        envelope: lettre::address::Envelope,
        bytes: Vec<u8>,
    ) -> Result<(), DeliveryFailure> {
        assert!(self.probe(c, true).await.is_ok());
        self.sends.fetch_add(1, Ordering::SeqCst);
        assert!(!String::from_utf8_lossy(&bytes).contains("Bcc:"));
        if String::from_utf8_lossy(&bytes).contains("Uncertain delivery fixture") {
            Err(DeliveryFailure::Uncertain)
        } else if String::from_utf8_lossy(&bytes).contains("Rejected delivery fixture") {
            Err(DeliveryFailure::Rejected("Synthetic SMTP rejection".into()))
        } else {
            use mailparse::MailHeaderMap;
            let parsed = mailparse::parse_mail(&bytes).unwrap();
            assert_eq!(
                parsed.headers.get_first_value("In-Reply-To").as_deref(),
                Some("<original@example.test>")
            );
            assert_eq!(
                parsed.headers.get_first_value("References").as_deref(),
                Some("<root@example.test> <original@example.test>")
            );
            assert_eq!(
                parsed.headers.get_first_value("To").as_deref(),
                Some("Support <support@example.test>, Peer <peer@example.test>")
            );
            assert_eq!(
                parsed.headers.get_first_value("Cc").as_deref(),
                Some("Copy <copy@example.test>")
            );
            assert_eq!(envelope.to().len(), 4);
            assert_eq!(parsed.subparts.len(), 2);
            assert_eq!(
                parsed.subparts[0].get_body().unwrap().trim_end(),
                "Saved before SMTP begins."
            );
            assert_eq!(
                parsed.subparts[1].get_body_raw().unwrap(),
                [0, 255, 1, 13, 10]
            );
            assert_eq!(
                parsed.subparts[1].get_content_disposition().params["filename"],
                "binary.bin"
            );
            Ok(())
        }
    }
}

struct BrowserGoogle;
#[async_trait]
impl LoginVerifier for BrowserGoogle {
    async fn exchange(
        &self,
        code: &str,
        verifier: &str,
        nonce: &str,
    ) -> Result<Identity, VerificationFailed> {
        if verifier.len() != 43 || nonce.len() != 43 {
            return Err(VerificationFailed);
        }
        let email = match code {
            "owner-fixture" => "owner@example.test",
            "denied-fixture" => "not-invited@example.test",
            _ => return Err(VerificationFailed),
        };
        Ok(Identity {
            subject: format!("synthetic-{code}"),
            email: email.into(),
        })
    }
}

#[tokio::test]
#[ignore = "requires built web/dist, Node and Playwright Chromium; run explicitly in client CI"]
async fn real_browser_beta_gate() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    assert!(
        root.join("web/dist/index.html").is_file(),
        "Build web first"
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // Reserve a port briefly for the HTTPS-only browser proxy. Node fails if
    // another process takes it; it never attaches to an existing server.
    let proxy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let proxy_port = proxy.local_addr().unwrap().port();
    let config = Arc::new(Config {
        origin: format!("https://127.0.0.1:{proxy_port}"),
        google_client_id: "synthetic-browser-client".into(),
        google_client_secret: Zeroizing::new("synthetic-unused-secret".into()),
        allowed_emails: ["owner@example.test".into()].into(),
        allowed_subjects: Default::default(),
        web_dir: root.join("web/dist"),
        bind: address,
        mail_endpoints: vec![
            Endpoint {
                host: "mail.example.test".into(),
                port: 993,
                service: Service::Imap,
                address: "127.0.0.1:1993".parse().unwrap(),
            },
            Endpoint {
                host: "mail.example.test".into(),
                port: 465,
                service: Service::Smtp,
                address: "127.0.0.1:1465".parse().unwrap(),
            },
        ],
    });
    config.validate().unwrap();
    let mut state = AppState::new(config.clone(), Arc::new(BrowserGoogle));
    let transport = Arc::new(BrowserMail {
        unread: AtomicBool::new(true),
        ..Default::default()
    });
    let mut hub = MailHub::new(config.mail_endpoints.clone());
    hub.transport = transport.clone();
    state.mail = Arc::new(hub);
    let router = app(state);
    let (shutdown, done) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = done.await;
            })
            .await
    });
    drop(proxy);
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new("node")
            .args([
                "web/e2e/beta-gateway.mjs",
                &address.port().to_string(),
                &proxy_port.to_string(),
            ])
            .current_dir(root)
            .output()
    })
    .await
    .unwrap()
    .expect("Node must be installed for this explicitly selected browser test");
    let _ = shutdown.send(());
    server.await.unwrap().unwrap();
    assert!(
        output.status.success(),
        "Browser beta test failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        transport.moves.load(Ordering::SeqCst),
        4,
        "Queued Undo and refresh Undo use the acknowledged destination identity"
    );
    assert_eq!(
        transport.sends.load(Ordering::SeqCst),
        4,
        "One delivered, two explicitly submitted uncertain copies and one rejection; review, preparation and status checks never send"
    );
}
