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
    collections::{HashMap, HashSet},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{Mutex, mpsc};
const MOVE_RAW:&[u8]=b"From: Mail fixture <sender@example.test>\r\nReply-To: Support <support@example.test>\r\nTo: mailbox@example.test, Peer <peer@example.test>\r\nCc: Peer <peer@example.test>, Copy <copy@example.test>\r\nMessage-ID: <original@example.test>\r\nReferences: <root@example.test>\r\nSubject: The beta transport fixture\r\nDate: Sun, 6 Sep 2026 10:00:00 +0000\r\n\r\nThis fictional message passed through the Rust mail API into browser storage.";

#[derive(Default)]
struct BrowserMail {
    unread: AtomicBool,
    starred: AtomicBool,
    attachment_unread: AtomicBool,
    attachment_starred: AtomicBool,
    attachment_flags: AtomicUsize,
    sends: AtomicUsize,
    moves: AtomicUsize,
    location: Mutex<Option<(String, String)>>,
    sent: Arc<BrowserSent>,
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
                ["INBOX", "Archive", "Sent Mail"]
                    .into_iter()
                    .map(|name| shep_mail_core::folders::Mailbox::flat(name.into()))
                    .collect(),
            ))
            .await?;
        let location = self
            .location
            .lock()
            .await
            .clone()
            .unwrap_or(("INBOX".into(), "42.7".into()));
        output
            .send(MailSyncItem::SentFolder(
                c.account.id.clone(),
                Some("Sent Mail".into()),
            ))
            .await?;
        let mut live_ids = HashSet::new();
        if folder == location.0 {
            let mail = parse_mail(
                &c.account.id,
                &location.1,
                folder,
                MOVE_RAW.to_vec(),
                self.unread.load(Ordering::SeqCst),
                self.starred.load(Ordering::SeqCst),
            )?;
            live_ids.insert(mail.summary.id.clone());
            output.send(MailSyncItem::Message(mail)).await?;
        }
        for copy in self.sent.copies.lock().await.values() {
            if copy.folder == folder {
                let mail = parse_mail(
                    &c.account.id,
                    &copy.remote,
                    folder,
                    copy.raw.clone(),
                    copy.unread,
                    copy.starred,
                )?;
                live_ids.insert(mail.summary.id.clone());
                output.send(MailSyncItem::Message(mail)).await?;
            }
        }
        if folder == "INBOX" {
            let fixture: serde_json::Value =
                serde_json::from_str(include_str!("../../shared/attachment-fixtures.json"))?;
            let find: serde_json::Value =
                serde_json::from_str(include_str!("../../shared/find-preview.json"))?;
            let raw = fixture[0]["raw"].as_str().unwrap().replace(
                "Cached incoming files.",
                &find["body"].as_str().unwrap().replace('\n', "\r\n"),
            );
            let mail = parse_mail(
                &c.account.id,
                "42.99",
                folder,
                raw.into_bytes(),
                self.attachment_unread.load(Ordering::SeqCst),
                self.attachment_starred.load(Ordering::SeqCst),
            )?;
            live_ids.insert(mail.summary.id.clone());
            output.send(MailSyncItem::Message(mail)).await?;
        }
        output
            .send(MailSyncItem::Reconcile {
                account: c.account.id.clone(),
                folder: folder.into(),
                live_ids,
            })
            .await?;
        Ok(vec!["INBOX".into(), "Archive".into(), "Sent Mail".into()])
    }
    async fn flags(&self, c: &Connection, mail: &Mail, flags: Flags) -> anyhow::Result<()> {
        self.probe(c, false).await?;
        if let Some(copy) = self
            .sent
            .copies
            .lock()
            .await
            .values_mut()
            .find(|copy| copy.folder == mail.folder && copy.remote == mail.remote_id)
        {
            if let Some(value) = flags.unread {
                copy.unread = value;
            }
            if let Some(value) = flags.starred {
                copy.starred = value;
            }
            self.sent.flags.fetch_add(1, Ordering::SeqCst);
            return Ok(());
        }
        if mail.folder == "INBOX" && mail.remote_id == "42.99" {
            if let Some(value) = flags.unread {
                self.attachment_unread.store(value, Ordering::SeqCst);
            }
            if let Some(value) = flags.starred {
                self.attachment_starred.store(value, Ordering::SeqCst);
            }
            self.attachment_flags.fetch_add(1, Ordering::SeqCst);
            return Ok(());
        }
        let location = self
            .location
            .lock()
            .await
            .clone()
            .unwrap_or(("INBOX".into(), "42.7".into()));
        anyhow::ensure!(
            location == (mail.folder.clone(), mail.remote_id.clone()),
            "Stale flag UID"
        );
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
        if let Some(copy) = self
            .sent
            .copies
            .lock()
            .await
            .values_mut()
            .find(|copy| copy.folder == mail.folder && copy.remote == mail.remote_id)
        {
            let uid = self.sent.moves.fetch_add(1, Ordering::SeqCst) + 100;
            copy.folder = folder.into();
            copy.remote = format!("91.{uid}");
            return Ok(Some(copy.remote.clone()));
        }
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
        if let Some(copy) = self.sent.copies.lock().await.values().find(|copy| {
            copy.folder == receipt.folder
                && receipt
                    .fingerprint
                    .as_ref()
                    .is_some_and(|f| f.matches(&copy.raw))
        }) {
            if let Some(current) = &receipt.current {
                anyhow::ensure!(current.remote_id == copy.remote, "Stale Sent recovery UID");
            }
            return Ok(parse_mail(
                &c.account.id,
                &copy.remote,
                &copy.folder,
                copy.raw.clone(),
                copy.unread,
                copy.starred,
            )?
            .summary);
        }
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
    async fn sent(
        &self,
        c: &Connection,
    ) -> anyhow::Result<Box<dyn shep_mail_core::providers::mail::sent::SentConnection>> {
        self.probe(c, false).await?;
        Ok(Box::new(BrowserSentMailbox {
            data: self.sent.clone(),
            folder: if c.account.sent_folder.is_empty() {
                "Sent Mail".into()
            } else {
                c.account.sent_folder.clone()
            },
        }))
    }
    async fn send(
        &self,
        c: &Connection,
        envelope: lettre::address::Envelope,
        bytes: Vec<u8>,
    ) -> Result<(), DeliveryFailure> {
        assert!(self.probe(c, true).await.is_ok());
        self.sends.fetch_add(1, Ordering::SeqCst);
        use mailparse::MailHeaderMap;
        let (headers, _) = mailparse::parse_headers(&bytes).unwrap();
        self.sent.original.lock().await.insert(
            headers.get_first_value("Message-ID").unwrap(),
            bytes.clone(),
        );
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

#[derive(Default)]
struct BrowserSent {
    original: Mutex<HashMap<String, Vec<u8>>>,
    copies: Mutex<HashMap<String, BrowserSentCopy>>,
    appends: AtomicUsize,
    moves: AtomicUsize,
    flags: AtomicUsize,
}
struct BrowserSentCopy {
    raw: Vec<u8>,
    folder: String,
    remote: String,
    unread: bool,
    starred: bool,
}
struct BrowserSentMailbox {
    data: Arc<BrowserSent>,
    folder: String,
}
#[async_trait]
impl shep_mail_core::providers::mail::sent::SentConnection for BrowserSentMailbox {
    fn folder(&self) -> &str {
        &self.folder
    }
    async fn find(
        &mut self,
        id: &str,
    ) -> anyhow::Result<Option<shep_mail_core::providers::mail::sent::SentReceipt>> {
        Ok(self
            .data
            .copies
            .lock()
            .await
            .get(id)
            .filter(|copy| copy.folder == self.folder)
            .map(|copy| shep_mail_core::providers::mail::sent::SentReceipt {
                folder: self.folder.clone(),
                remote_id: Some(copy.remote.clone()),
            }))
    }
    async fn append(
        &mut self,
        raw: &[u8],
        _: i64,
    ) -> anyhow::Result<shep_mail_core::providers::mail::sent::SentReceipt> {
        use mailparse::MailHeaderMap;
        let (headers, _) = mailparse::parse_headers(raw)?;
        let id = headers.get_first_value("Message-ID").unwrap();
        assert_eq!(
            self.data.original.lock().await.get(&id).unwrap(),
            raw,
            "APPEND uses the immutable bytes originally submitted to SMTP"
        );
        let index = self.data.appends.fetch_add(1, Ordering::SeqCst);
        self.data.copies.lock().await.insert(
            id,
            BrowserSentCopy {
                raw: raw.to_vec(),
                folder: self.folder.clone(),
                remote: format!("91.{}", index + 4),
                unread: false,
                starred: false,
            },
        );
        anyhow::ensure!(
            !String::from_utf8_lossy(raw).contains("Uncertain delivery fixture reviewed"),
            "Synthetic lost APPEND acknowledgment"
        );
        Ok(shep_mail_core::providers::mail::sent::SentReceipt {
            folder: self.folder.clone(),
            remote_id: None,
        })
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
        profile_namespace: Some("so.shep.browser-fixture".into()),
    });
    config.validate().unwrap();
    let profiles = crate::profiles::tests::FixtureProvider::browser_gate();
    let mut state = AppState::new(config.clone(), Arc::new(BrowserGoogle), profiles.clone());
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
        transport.attachment_flags.load(Ordering::SeqCst),
        2,
        "The captured group flags and restores the second physical message exactly once"
    );
    assert_eq!(
        transport.moves.load(Ordering::SeqCst),
        4,
        "Queued Undo and refresh Undo use the acknowledged destination identity"
    );
    assert_eq!(
        transport.sent.moves.load(Ordering::SeqCst),
        2,
        "Sent Archive and Undo route current physical identities"
    );
    assert_eq!(
        transport.sent.flags.load(Ordering::SeqCst),
        2,
        "Sent flag and pre-handover Undo reach the provider"
    );
    assert_eq!(
        transport.sent.appends.load(Ordering::SeqCst),
        2,
        "One delivered and one explicitly reviewed Sent copy; receipt recovery and lookup never repeat APPEND"
    );
    assert_eq!(
        transport.sends.load(Ordering::SeqCst),
        4,
        "One delivered, two explicitly submitted uncertain copies and one rejection; review, preparation and status checks never send"
    );
    assert_eq!(
        profiles.exchanges.load(Ordering::SeqCst),
        2,
        "One denied consent never exchanges; one accepted consent and one reconnect exchange exactly once each"
    );
    assert!(
        profiles.drive_calls.load(Ordering::SeqCst) >= 3,
        "The consent stage verifies the Drive principal and lists app data through the proxy"
    );
}
