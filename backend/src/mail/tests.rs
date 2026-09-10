use super::*;
use crate::tests::{body, login};
use axum::http::Request as HttpRequest;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tower::ServiceExt;

#[path = "sent_tests.rs"]
mod sent;

#[path = "forward_tests.rs"]
mod forward;

#[derive(Default)]
struct FakeMail {
    sent: Arc<sent::Fixture>,
    calls: AtomicUsize,
    sends: AtomicUsize,
    fail: AtomicBool,
    uncertain: AtomicBool,
    wire: Mutex<Vec<u8>>,
    hold_send: AtomicBool,
    panic_send: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
#[async_trait]
impl HostedMail for FakeMail {
    async fn probe(&self, _: &Connection, _: bool) -> anyhow::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(
            !self.fail.load(Ordering::SeqCst),
            "synthetic secret must never appear in HTTP errors"
        );
        Ok(())
    }
    async fn sync(
        &self,
        c: &Connection,
        _: &HashSet<String>,
        folder: &str,
        out: mpsc::Sender<MailSyncItem>,
    ) -> anyhow::Result<Vec<String>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        out.send(MailSyncItem::Folders(
            c.account.id.clone(),
            vec![shep_mail_core::folders::Mailbox::flat(folder.into())],
        ))
        .await?;
        let mail = parse_mail(
            &c.account.id,
            "42.7",
            folder,
            b"From: Demo <demo@example.test>\r\nSubject: Protocol fixture\r\n\r\nSynthetic body"
                .to_vec(),
            true,
            false,
        )?;
        out.send(MailSyncItem::Message(mail.clone())).await?;
        if self.fail.load(Ordering::SeqCst) {
            anyhow::bail!("private remote response")
        }
        out.send(MailSyncItem::Reconcile {
            account: c.account.id.clone(),
            folder: folder.into(),
            live_ids: [mail.summary.id].into(),
        })
        .await?;
        Ok(vec![folder.into()])
    }
    async fn flags(&self, c: &Connection, _: &Mail, _: Flags) -> anyhow::Result<()> {
        self.probe(c, false).await
    }
    async fn move_mail(&self, c: &Connection, _: &Mail, _: &str) -> anyhow::Result<Option<String>> {
        self.probe(c, false).await?;
        Ok(Some("91.8".into()))
    }
    async fn resolve_move(&self, c: &Connection, receipt: &MoveReceipt) -> anyhow::Result<Mail> {
        self.probe(c, false).await?;
        receipt
            .current
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Synthetic missing copy"))
    }
    async fn sent(
        &self,
        c: &Connection,
    ) -> anyhow::Result<Box<dyn shep_mail_core::providers::mail::sent::SentConnection>> {
        anyhow::ensure!(
            !self.fail.load(Ordering::SeqCst),
            "synthetic private connection error"
        );
        Ok(Box::new(sent::Mailbox {
            fixture: self.sent.clone(),
            folder: if c.account.sent_folder.is_empty() {
                "Sent Mail".into()
            } else {
                c.account.sent_folder.clone()
            },
        }))
    }
    async fn send(
        &self,
        _: &Connection,
        _: lettre::address::Envelope,
        raw: Vec<u8>,
    ) -> Result<(), DeliveryFailure> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        if self.hold_send.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        assert!(
            !self.panic_send.load(Ordering::SeqCst),
            "Synthetic provider panic"
        );
        *self.wire.lock().await = raw;
        if self.uncertain.load(Ordering::SeqCst) {
            Err(DeliveryFailure::Uncertain)
        } else if self.fail.load(Ordering::SeqCst) {
            Err(DeliveryFailure::Rejected(
                "synthetic private SMTP response".into(),
            ))
        } else {
            Ok(())
        }
    }
}
fn setup() -> (AppState, Arc<FakeMail>) {
    let (mut state, _) = crate::tests::state("owner@example.test");
    let endpoints = vec![
        policy::Endpoint {
            host: "mail.example.test".into(),
            port: 993,
            service: policy::Service::Imap,
            address: "127.0.0.1:1993".parse().unwrap(),
        },
        policy::Endpoint {
            host: "mail.example.test".into(),
            port: 465,
            service: policy::Service::Smtp,
            address: "127.0.0.1:1465".parse().unwrap(),
        },
    ];
    Arc::get_mut(&mut state.config).unwrap().mail_endpoints = endpoints.clone();
    let mut mail = MailHub::new(endpoints);
    let fake = Arc::new(FakeMail::default());
    mail.transport = fake.clone();
    state.mail = Arc::new(mail);
    (state, fake)
}
fn connection() -> serde_json::Value {
    serde_json::json!({"account":{"id":"fixture-account","name":"Fixture","email":"sender@example.test","protocol":"Imap","host":"mail.example.test","port":993,"username":"fixture-user","smtp_host":"mail.example.test","smtp_port":465},"password":"fixture-password"})
}
async fn post_json(
    state: &AppState,
    cookie: &str,
    csrf: &str,
    path: &str,
    value: serde_json::Value,
) -> Response {
    crate::app(state.clone())
        .oneshot(
            HttpRequest::builder()
                .method("POST")
                .uri(path)
                .header(header::COOKIE, cookie)
                .header(header::ORIGIN, &state.config.origin)
                .header("x-shep-csrf", csrf)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}
async fn reserve(state: &AppState, cookie: &str, csrf: &str) -> String {
    let response = post_json(
        state,
        cookie,
        csrf,
        "/api/mail/outgoing/reserve",
        serde_json::json!({}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    serde_json::from_str::<serde_json::Value>(&body(response).await).unwrap()["id"]
        .as_str()
        .unwrap()
        .into()
}
fn compose_request(id: &str) -> serde_json::Value {
    serde_json::json!({"id":id,"connection":connection(),"draft":{"id":"fixture-draft","account_id":"fixture-account","to":"Receiver <receiver@example.test>","cc":"copy@example.test","bcc":"hidden@example.test","subject":"Shared MIME","body":"A fictional message."}})
}
async fn prepare_request(
    state: &AppState,
    cookie: &str,
    csrf: &str,
    id: &str,
) -> serde_json::Value {
    let response = post_json(
        state,
        cookie,
        csrf,
        "/api/mail/outgoing/prepare",
        compose_request(id),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let prepared: serde_json::Value = serde_json::from_str(&body(response).await).unwrap();
    serde_json::json!({"id":id,"connection":connection(),"wire":prepared["wire"]})
}
fn unprepared_request(id: &str) -> serde_json::Value {
    serde_json::json!({"id":id,"connection":connection(),"wire":{"envelope":{"from":"sender@example.test","to":["receiver@example.test"]},"raw":"c3ludGhldGlj"}})
}
#[tokio::test]
async fn endpoints_authentication_and_validation_precede_transport() {
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let denied = post_json(
        &state,
        "",
        &csrf,
        "/api/mail/probe",
        serde_json::json!({"connection":connection()}),
    )
    .await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    for host in [
        "127.0.0.1",
        "mail.example.test.attacker.test",
        "169.254.169.254",
    ] {
        let mut c = connection();
        c["account"]["host"] = host.into();
        assert_eq!(
            post_json(
                &state,
                &cookie,
                &csrf,
                "/api/mail/probe",
                serde_json::json!({"connection":c})
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        post_json(
            &state,
            &cookie,
            &csrf,
            "/api/mail/probe",
            serde_json::json!({"connection":connection()})
        )
        .await
        .status(),
        StatusCode::OK
    );
    fake.fail.store(true, Ordering::SeqCst);
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/probe",
        serde_json::json!({"connection":connection()}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(!body(response).await.contains("synthetic secret"));
}
#[tokio::test]
async fn streaming_sync_keeps_partial_mail_and_never_reports_failed_sync_as_done() {
    for failed in [false, true] {
        let (state, fake) = setup();
        fake.fail.store(failed, Ordering::SeqCst);
        let (cookie, csrf) = login(&state).await;
        let response = post_json(
            &state,
            &cookie,
            &csrf,
            "/api/mail/sync",
            serde_json::json!({"connection":connection(),"folder":"INBOX","known":[]}),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let text = body(response).await;
        let events: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(events[0]["kind"], "folders");
        assert_eq!(events[1]["mail"]["summary"]["subject"], "Protocol fixture");
        assert_eq!(
            events.last().unwrap()["kind"],
            if failed { "error" } else { "done" }
        );
        assert!(!text.contains("private remote response"));
    }
}
#[tokio::test]
async fn mutations_require_matching_account_and_never_fake_acknowledgment() {
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let mail = parse_mail(
        "fixture-account",
        "42.7",
        "INBOX",
        b"Subject: fixture\r\n\r\nbody".to_vec(),
        true,
        false,
    )
    .unwrap()
    .summary;
    let mut flags = serde_json::json!({"connection":connection(),"mail":mail,"unread":false});
    flags["mail"]["account_id"] = "other-account".into();
    assert_eq!(
        post_json(&state, &cookie, &csrf, "/api/mail/flags", flags.clone())
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    flags["mail"]["account_id"] = "fixture-account".into();
    assert_eq!(
        post_json(&state, &cookie, &csrf, "/api/mail/flags", flags)
            .await
            .status(),
        StatusCode::OK
    );
    fake.fail.store(true, Ordering::SeqCst);
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/move",
        serde_json::json!({"connection":connection(),"mail":mail,"folder":"Archive"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(!body(response).await.contains("committed"));
}
#[tokio::test]
async fn smtp_requires_server_reservation_and_retries_never_send_twice() {
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    assert_eq!(
        post_json(
            &state,
            &cookie,
            &csrf,
            "/api/mail/send",
            unprepared_request("invented")
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
    let id = reserve(&state, &cookie, &csrf).await;
    let prepared = prepare_request(&state, &cookie, &csrf, &id).await;
    for _ in 0..2 {
        let response = post_json(&state, &cookie, &csrf, "/api/mail/send", prepared.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(body(response).await.contains("delivered"));
    }
    assert_eq!(fake.sends.load(Ordering::SeqCst), 1);
    let wire = String::from_utf8(fake.wire.lock().await.clone()).unwrap();
    assert!(wire.contains(&format!("<{id}@shep.so>")));
    assert!(!wire.contains("Bcc:"));
    assert!(!wire.contains("hidden@example.test"));
    let mut changed = prepared.clone();
    changed["wire"]["raw"] = "Q2hhbmdlZCBjb250ZW50".into();
    assert_eq!(
        post_json(&state, &cookie, &csrf, "/api/mail/send", changed)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(fake.sends.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn rejected_and_uncertain_sends_require_review_not_automatic_repetition() {
    for uncertain in [false, true] {
        let (state, fake) = setup();
        fake.fail.store(true, Ordering::SeqCst);
        fake.uncertain.store(uncertain, Ordering::SeqCst);
        let (cookie, csrf) = login(&state).await;
        let id = reserve(&state, &cookie, &csrf).await;
        let prepared = prepare_request(&state, &cookie, &csrf, &id).await;
        let status = if uncertain {
            StatusCode::CONFLICT
        } else {
            StatusCode::UNPROCESSABLE_ENTITY
        };
        for _ in 0..2 {
            let response =
                post_json(&state, &cookie, &csrf, "/api/mail/send", prepared.clone()).await;
            assert_eq!(response.status(), status);
            assert!(!body(response).await.contains("synthetic private"));
        }
        assert_eq!(fake.sends.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn expired_or_lost_reservations_and_other_identities_cannot_start_a_send() {
    let (mut state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let id = reserve(&state, &cookie, &csrf).await;
    let prepared = prepare_request(&state, &cookie, &csrf, &id).await;
    state
        .mail
        .outgoing
        .lock()
        .await
        .get_mut(&id)
        .unwrap()
        .created = std::time::Instant::now() - Duration::from_secs(1801);
    assert_eq!(
        post_json(&state, &cookie, &csrf, "/api/mail/send", prepared.clone())
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let id = reserve(&state, &cookie, &csrf).await;
    let prepared = prepare_request(&state, &cookie, &csrf, &id).await;
    state
        .mail
        .outgoing
        .lock()
        .await
        .get_mut(&id)
        .unwrap()
        .subject = "different-subject".into();
    assert_eq!(
        post_json(&state, &cookie, &csrf, "/api/mail/send", prepared.clone())
            .await
            .status(),
        StatusCode::CONFLICT
    );
    state.mail = Arc::new(MailHub::new(state.config.mail_endpoints.clone()));
    assert_eq!(
        post_json(&state, &cookie, &csrf, "/api/mail/send", prepared.clone())
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn bounded_network_admission_does_not_block_cached_session_or_receipt_reads() {
    let (state, _) = setup();
    let (cookie, csrf) = login(&state).await;
    let id = reserve(&state, &cookie, &csrf).await;
    let _first = state.mail.admit("owner-subject").await.unwrap();
    let _second = state.mail.admit("owner-subject").await.unwrap();
    assert_eq!(
        post_json(
            &state,
            &cookie,
            &csrf,
            "/api/mail/probe",
            serde_json::json!({"connection":connection()})
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    for path in [
        "/api/session".to_owned(),
        format!("/api/mail/outgoing/{id}"),
    ] {
        let r = crate::app(state.clone())
            .oneshot(
                HttpRequest::builder()
                    .uri(path)
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
    }
}
#[test]
fn endpoint_policy_uses_exact_pins_and_rejects_ambiguous_configuration() {
    let (state, _) = setup();
    let mut endpoints = state.config.mail_endpoints.clone();
    assert!(policy::validate(&endpoints).is_ok());
    assert_eq!(
        policy::resolve(&endpoints, "MAIL.EXAMPLE.TEST", 993, policy::Service::Imap).unwrap(),
        "127.0.0.1:1993".parse().unwrap()
    );
    assert!(policy::resolve(&endpoints, "mail.example.test", 465, policy::Service::Imap).is_err());
    endpoints.push(endpoints[0].clone());
    assert!(policy::validate(&endpoints).is_err());
    endpoints.pop();
    endpoints[0].host = "*.example.test".into();
    assert!(policy::validate(&endpoints).is_err());
}

#[tokio::test]
async fn disconnected_http_waiter_cannot_cancel_accepted_send_or_lose_panic_receipt() {
    for panic in [false, true] {
        let (state, fake) = setup();
        let (cookie, csrf) = login(&state).await;
        let id = reserve(&state, &cookie, &csrf).await;
        let prepared = prepare_request(&state, &cookie, &csrf, &id).await;
        fake.hold_send.store(true, Ordering::SeqCst);
        fake.panic_send.store(panic, Ordering::SeqCst);
        let (sending, c, token, request) = (
            state.clone(),
            cookie.clone(),
            csrf.clone(),
            prepared.clone(),
        );
        let waiter = tokio::spawn(async move {
            post_json(&sending, &c, &token, "/api/mail/send", request).await
        });
        fake.entered.notified().await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        fake.release.notify_one();
        let expected = if panic { "uncertain" } else { "delivered" };
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let response = crate::app(state.clone())
                    .oneshot(
                        HttpRequest::builder()
                            .uri(format!("/api/mail/outgoing/{id}"))
                            .header(header::COOKIE, &cookie)
                            .body(Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let value: serde_json::Value = serde_json::from_str(&body(response).await).unwrap();
                if value["state"] == expected {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(fake.sends.load(Ordering::SeqCst), 1);
        let retry = post_json(&state, &cookie, &csrf, "/api/mail/send", prepared.clone()).await;
        assert!(body(retry).await.contains(expected));
        assert_eq!(fake.sends.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn move_receipts_and_recovery_remain_scoped_and_fail_closed() {
    use shep_mail_core::mail_actions::Fingerprint;
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let raw = b"Message-ID: <fixture@example.test>\r\n\r\nExact copy";
    let source = parse_mail(
        "fixture-account",
        "42.7",
        "INBOX",
        raw.to_vec(),
        true,
        false,
    )
    .unwrap()
    .summary;
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/move",
        serde_json::json!({"connection":connection(),"mail":source,"folder":"Archive"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let data: serde_json::Value = serde_json::from_str(&body(response).await).unwrap();
    assert_eq!(data["remote_id"], "91.8");
    assert_eq!(data["committed"], true);
    let receipt = MoveReceipt::server(
        &source,
        "fixture-account",
        "Archive",
        Some("91.8".into()),
        Fingerprint::of(raw),
    );
    let request = serde_json::json!({"connection":connection(),"receipt":receipt});
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/resolve-move",
        request.clone(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let data: serde_json::Value = serde_json::from_str(&body(response).await).unwrap();
    assert_eq!(data["mail"]["remote_id"], "91.8");
    let calls = fake.calls.load(Ordering::SeqCst);
    for field in ["account", "current", "bytes", "header"] {
        let mut invalid = request.clone();
        match field {
            "account" => invalid["receipt"]["account"] = "other".into(),
            "current" => invalid["receipt"]["current"]["folder"] = "INBOX".into(),
            "bytes" => invalid["receipt"]["fingerprint"]["bytes"] = (26 * 1024 * 1024).into(),
            _ => invalid["receipt"]["fingerprint"]["message_id"] = "bad\r\nUID MOVE".into(),
        }
        assert_eq!(
            post_json(&state, &cookie, &csrf, "/api/mail/resolve-move", invalid)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(fake.calls.load(Ordering::SeqCst), calls);
    fake.fail.store(true, Ordering::SeqCst);
    let response = post_json(&state, &cookie, &csrf, "/api/mail/resolve-move", request).await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(!body(response).await.contains("synthetic secret"));
}

#[tokio::test]
async fn prepared_wire_and_connection_are_bound_and_cancelled_reservations_never_send() {
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let id = reserve(&state, &cookie, &csrf).await;
    let prepared = prepare_request(&state, &cookie, &csrf, &id).await;
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
    for mut changed in [prepared.clone(), prepared.clone(), prepared.clone()]
        .into_iter()
        .enumerate()
    {
        match changed.0 {
            0 => {
                changed.1["wire"]["envelope"]["to"] = serde_json::json!(["different@example.test"])
            }
            1 => changed.1["wire"]["raw"] = "Q2hhbmdlZA==".into(),
            _ => changed.1["connection"]["account"]["email"] = "changed@example.test".into(),
        }
        assert_eq!(
            post_json(&state, &cookie, &csrf, "/api/mail/send", changed.1)
                .await
                .status(),
            StatusCode::CONFLICT
        );
    }
    let cancel = format!("/api/mail/outgoing/{id}/cancel");
    assert_eq!(
        post_json(&state, &cookie, "wrong", &cancel, serde_json::json!({}))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    for _ in 0..2 {
        let response = post_json(&state, &cookie, &csrf, &cancel, serde_json::json!({})).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(body(response).await.contains("cancelled"));
    }
    let response = post_json(&state, &cookie, &csrf, "/api/mail/send", prepared).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body(response).await.contains("cancelled"));
    assert_eq!(
        post_json(
            &state,
            &cookie,
            &csrf,
            "/api/mail/outgoing/prepare",
            compose_request(&id)
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancelling_cannot_release_an_active_smtp_send_and_prepared_bytes_survive_secret_changes() {
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let id = reserve(&state, &cookie, &csrf).await;
    let mut prepared = prepare_request(&state, &cookie, &csrf, &id).await;
    let wire = prepared["wire"]["raw"].as_str().unwrap().to_owned();
    prepared["connection"]["password"] = "corrected-synthetic-password".into();
    fake.hold_send.store(true, Ordering::SeqCst);
    let (sending, c, t, payload) = (
        state.clone(),
        cookie.clone(),
        csrf.clone(),
        prepared.clone(),
    );
    let task =
        tokio::spawn(async move { post_json(&sending, &c, &t, "/api/mail/send", payload).await });
    fake.entered.notified().await;
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        &format!("/api/mail/outgoing/{id}/cancel"),
        serde_json::json!({}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(body(response).await.contains("submitting"));
    let retry = post_json(&state, &cookie, &csrf, "/api/mail/send", prepared).await;
    assert_eq!(retry.status(), StatusCode::ACCEPTED);
    assert_eq!(fake.sends.load(Ordering::SeqCst), 1);
    fake.release.notify_one();
    assert_eq!(task.await.unwrap().status(), StatusCode::OK);
    use base64::{Engine, engine::general_purpose::STANDARD};
    assert_eq!(*fake.wire.lock().await, STANDARD.decode(wire).unwrap());
}

#[tokio::test]
async fn preparation_failure_or_lost_response_does_not_authorize_a_fresh_or_changed_send() {
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let id = reserve(&state, &cookie, &csrf).await;
    let mut invalid = compose_request(&id);
    invalid["draft"]["to"] = "invalid address".into();
    invalid["draft"]["cc"] = "".into();
    invalid["draft"]["bcc"] = "".into();
    assert_eq!(
        post_json(
            &state,
            &cookie,
            &csrf,
            "/api/mail/outgoing/prepare",
            invalid
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post_json(
            &state,
            &cookie,
            &csrf,
            "/api/mail/send",
            unprepared_request(&id)
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let prepared = prepare_request(&state, &cookie, &csrf, &id).await;
    // Pretend the client lost the response: a second prepare cannot replace it.
    assert_eq!(
        post_json(
            &state,
            &cookie,
            &csrf,
            "/api/mail/outgoing/prepare",
            compose_request(&id)
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    state
        .mail
        .outgoing
        .lock()
        .await
        .get_mut(&id)
        .unwrap()
        .subject = "another-owner".into();
    assert_eq!(
        post_json(
            &state,
            &cookie,
            &csrf,
            &format!("/api/mail/outgoing/{id}/cancel"),
            serde_json::json!({})
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post_json(&state, &cookie, &csrf, "/api/mail/send", prepared)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
}

#[test]
fn sent_handover_identity_requires_one_complete_header_within_the_bound() {
    assert_eq!(
        unique_sent_identity(b"Message-ID: <one@shep.so>\r\nSubject: fixture\r\n\r\nbody"),
        Some("<one@shep.so>".into())
    );
    for raw in [
        b"Message-ID: <one@shep.so>\r\nMessage-ID: <one@shep.so>\r\n\r\n".as_slice(),
        b"Message-ID: <one@shep.so> <two@shep.so>\n\n",
        b"Message-ID: <one@shep.so>\r\n",
    ] {
        assert!(unique_sent_identity(raw).is_none());
    }
    let mut long = b"Message-ID: <one@shep.so>\r\nX-Padding: ".to_vec();
    long.extend(vec![b'a'; 64 * 1024]);
    long.extend_from_slice(b"\r\nMessage-ID: <two@shep.so>\r\n\r\n");
    assert!(unique_sent_identity(&long).is_none());
}
