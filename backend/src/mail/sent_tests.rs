use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use shep_mail_core::providers::mail::sent::{SentConnection, SentReceipt};

const RAW: &[u8] = b"From: sender@example.test\r\nTo: peer@example.test\r\nMessage-ID: <sent-fixture@example.test>\r\nSubject: Synthetic copy\r\n\r\nExact original bytes.\r\n";
#[derive(Default)]
pub(super) struct Fixture {
    pub opens: AtomicUsize,
    appends: AtomicUsize,
    found: AtomicBool,
    fail_find: AtomicBool,
    fail_append: AtomicBool,
    panic_append: AtomicBool,
    hold: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    raw: Mutex<Vec<u8>>,
}
pub(super) struct Mailbox {
    pub fixture: Arc<Fixture>,
    pub folder: String,
}
#[async_trait]
impl SentConnection for Mailbox {
    fn folder(&self) -> &str {
        &self.folder
    }
    async fn find(&mut self, id: &str) -> anyhow::Result<Option<SentReceipt>> {
        self.fixture.opens.fetch_add(1, Ordering::SeqCst);
        assert_eq!(id, "<sent-fixture@example.test>");
        anyhow::ensure!(
            !self.fixture.fail_find.load(Ordering::SeqCst),
            "synthetic secret find error"
        );
        Ok(self
            .fixture
            .found
            .load(Ordering::SeqCst)
            .then(|| SentReceipt {
                folder: self.folder.clone(),
                remote_id: Some("91.4".into()),
            }))
    }
    async fn append(&mut self, raw: &[u8], timestamp: i64) -> anyhow::Result<SentReceipt> {
        self.fixture.appends.fetch_add(1, Ordering::SeqCst);
        assert_eq!(timestamp, 1_788_696_000);
        *self.fixture.raw.lock().await = raw.to_vec();
        if self.fixture.hold.load(Ordering::SeqCst) {
            self.fixture.entered.notify_one();
            self.fixture.release.notified().await;
        }
        assert!(
            !self.fixture.panic_append.load(Ordering::SeqCst),
            "Synthetic APPEND panic"
        );
        anyhow::ensure!(
            !self.fixture.fail_append.load(Ordering::SeqCst),
            "synthetic secret APPEND error"
        );
        Ok(SentReceipt {
            folder: self.folder.clone(),
            remote_id: None,
        })
    }
}
fn content() -> Value {
    let mut c = connection();
    c["account"]["sent_copy"] = "Automatic".into();
    c["account"]["sent_folder"] = "Sent Mail".into();
    json!({"connection":c,"wire":{"envelope":{"from":"sender@example.test","to":["peer@example.test"]},"raw":STANDARD.encode(RAW)},"timestamp":1_788_696_000})
}
async fn data(response: Response) -> Value {
    assert!(response.status().is_success(), "{}", body(response).await);
    serde_json::from_str(&body(response).await).unwrap()
}
async fn reserve_copy(s: &AppState, cookie: &str, csrf: &str, retry: bool) -> Value {
    let mut value = content();
    value["reviewed_retry"] = retry.into();
    data(post_json(s, cookie, csrf, "/api/mail/sent/reserve", value).await).await
}
async fn copy(s: &AppState, cookie: &str, csrf: &str, id: &str) -> Value {
    data(
        post_json(
            s,
            cookie,
            csrf,
            &format!("/api/mail/sent/{id}/copy"),
            content(),
        )
        .await,
    )
    .await
}
async fn status(s: &AppState, cookie: &str, id: &str) -> Response {
    crate::app(s.clone())
        .oneshot(
            HttpRequest::builder()
                .uri(format!("/api/mail/sent/{id}"))
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}
#[tokio::test]
async fn sent_check_never_uploads_and_policies_auth_and_endpoint_pins_apply() {
    let (s, fake) = setup();
    let (cookie, csrf) = login(&s).await;
    let mut request = json!({"connection":connection(),"message_id":"<sent-fixture@example.test>"});
    let checked =
        data(post_json(&s, &cookie, &csrf, "/api/mail/sent/check", request.clone()).await).await;
    assert_eq!(checked, json!({"folder":"Sent Mail","receipt":null}));
    fake.sent.found.store(true, Ordering::SeqCst);
    let checked =
        data(post_json(&s, &cookie, &csrf, "/api/mail/sent/check", request.clone()).await).await;
    assert_eq!(checked["receipt"]["remote_id"], "91.4");
    assert_eq!(
        post_json(&s, "", &csrf, "/api/mail/sent/check", request.clone())
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        post_json(
            &s,
            &cookie,
            "wrong",
            "/api/mail/sent/check",
            request.clone()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    request["connection"]["account"]["host"] = "evil.example.test".into();
    assert_eq!(
        post_json(&s, &cookie, &csrf, "/api/mail/sent/check", request)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    for policy in ["ServerManaged", "LocalOnly"] {
        let mut request = content();
        request["connection"]["account"]["sent_copy"] = policy.into();
        assert_eq!(
            post_json(&s, &cookie, &csrf, "/api/mail/sent/reserve", request)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(fake.sent.appends.load(Ordering::SeqCst), 0);
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn sent_reservations_bind_exact_content_account_destination_and_date() {
    let (s, fake) = setup();
    let (cookie, csrf) = login(&s).await;
    let reserved = reserve_copy(&s, &cookie, &csrf, false).await;
    let id = reserved["id"].as_str().unwrap();
    assert_eq!(reserved["state"], "reserved");
    assert_eq!(reserve_copy(&s, &cookie, &csrf, false).await, reserved);
    assert_eq!(fake.sent.appends.load(Ordering::SeqCst), 0);
    for field in ["raw", "timestamp", "folder", "account"] {
        let mut request = content();
        match field {
            "raw" => request["wire"]["raw"] = STANDARD.encode([RAW, b"altered"].concat()).into(),
            "timestamp" => request["timestamp"] = (1_788_696_000 + 1).into(),
            "folder" => request["connection"]["account"]["sent_folder"] = "Different Sent".into(),
            _ => request["connection"]["account"]["id"] = "different-account".into(),
        }
        assert_eq!(
            post_json(
                &s,
                &cookie,
                &csrf,
                &format!("/api/mail/sent/{id}/copy"),
                request
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
    }
    let mut request = content();
    request["connection"]["password"] = "corrected-password".into();
    let saved = data(
        post_json(
            &s,
            &cookie,
            &csrf,
            &format!("/api/mail/sent/{id}/copy"),
            request,
        )
        .await,
    )
    .await;
    assert_eq!(saved["state"], "saved");
    assert_eq!(
        saved["receipt"],
        json!({"folder":"Sent Mail","remote_id":null})
    );
    assert_eq!(copy(&s, &cookie, &csrf, id).await, saved);
    assert_eq!(reserve_copy(&s, &cookie, &csrf, true).await, saved);
    assert_eq!(fake.sent.appends.load(Ordering::SeqCst), 1);
    assert_eq!(*fake.sent.raw.lock().await, RAW);
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn sent_upload_outlives_http_disconnect_and_keeps_busy_receipts() {
    let (s, fake) = setup();
    let (cookie, csrf) = login(&s).await;
    let reserved = reserve_copy(&s, &cookie, &csrf, false).await;
    let id = reserved["id"].as_str().unwrap().to_owned();
    fake.sent.hold.store(true, Ordering::SeqCst);
    let waiter = tokio::spawn({
        let s = s.clone();
        let cookie = cookie.clone();
        let csrf = csrf.clone();
        let id = id.clone();
        async move { copy(&s, &cookie, &csrf, &id).await }
    });
    fake.sent.entered.notified().await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    assert_eq!(copy(&s, &cookie, &csrf, &id).await["state"], "copying");
    assert_eq!(reserve_copy(&s, &cookie, &csrf, true).await["id"], id);
    assert_eq!(s.mail.slots.available_permits(), 7);
    fake.sent.release.notify_one();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if data(status(&s, &cookie, &id).await).await["state"] == "saved" {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fake.sent.appends.load(Ordering::SeqCst), 1);
    assert_eq!(s.mail.slots.available_permits(), 8);
}
#[tokio::test]
async fn sent_unknown_or_expired_reservations_never_begin_an_upload() {
    let (s, fake) = setup();
    let (cookie, csrf) = login(&s).await;
    let reserved = reserve_copy(&s, &cookie, &csrf, false).await;
    let id = reserved["id"].as_str().unwrap();
    s.mail.copies.lock().await.get_mut(id).unwrap().created -= Duration::from_secs(1801);
    assert_eq!(
        post_json(
            &s,
            &cookie,
            &csrf,
            &format!("/api/mail/sent/{id}/copy"),
            content()
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    s.mail.copies.lock().await.clear();
    assert_eq!(
        status(&s, &cookie, id).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post_json(
            &s,
            &cookie,
            &csrf,
            &format!("/api/mail/sent/{id}/copy"),
            content()
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(fake.sent.appends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn sent_ambiguous_upload_requires_review_and_retry_checks_for_a_copy_first() {
    let (s, fake) = setup();
    let (cookie, csrf) = login(&s).await;
    let reserved = reserve_copy(&s, &cookie, &csrf, false).await;
    let id = reserved["id"].as_str().unwrap();
    fake.sent.fail_append.store(true, Ordering::SeqCst);
    let uncertain = copy(&s, &cookie, &csrf, id).await;
    assert_eq!(uncertain["state"], "uncertain");
    assert!(!uncertain.to_string().contains("synthetic secret"));
    assert_eq!(copy(&s, &cookie, &csrf, id).await, uncertain);
    assert_eq!(reserve_copy(&s, &cookie, &csrf, false).await["id"], id);
    let retry = reserve_copy(&s, &cookie, &csrf, true).await;
    assert_ne!(retry["id"], id);
    fake.sent.found.store(true, Ordering::SeqCst);
    assert_eq!(
        copy(&s, &cookie, &csrf, retry["id"].as_str().unwrap()).await["state"],
        "saved"
    );
    assert_eq!(fake.sent.appends.load(Ordering::SeqCst), 1);
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn sent_lookup_failure_and_supervised_append_panic_have_safe_receipts() {
    let (s, fake) = setup();
    let (cookie, csrf) = login(&s).await;
    let reserved = reserve_copy(&s, &cookie, &csrf, false).await;
    fake.sent.fail_find.store(true, Ordering::SeqCst);
    assert_eq!(
        copy(&s, &cookie, &csrf, reserved["id"].as_str().unwrap()).await["state"],
        "failed"
    );
    assert_eq!(fake.sent.appends.load(Ordering::SeqCst), 0);
    fake.sent.fail_find.store(false, Ordering::SeqCst);
    fake.sent.panic_append.store(true, Ordering::SeqCst);
    let retry = reserve_copy(&s, &cookie, &csrf, true).await;
    assert_eq!(
        copy(&s, &cookie, &csrf, retry["id"].as_str().unwrap()).await["state"],
        "uncertain"
    );
    assert_eq!(s.mail.slots.available_permits(), 8);
}
#[tokio::test]
async fn sent_invalid_message_identity_or_date_never_gets_a_reservation() {
    let (s, fake) = setup();
    let (cookie, csrf) = login(&s).await;
    for (raw, date) in [
        (
            b"Message-ID: <one@example.test>\r\nMessage-ID: <two@example.test>\r\n\r\n".as_slice(),
            1_788_696_000,
        ),
        (RAW, i64::MAX),
    ] {
        let mut request = content();
        request["wire"]["raw"] = STANDARD.encode(raw).into();
        request["timestamp"] = date.into();
        assert_eq!(
            post_json(&s, &cookie, &csrf, "/api/mail/sent/reserve", request)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert!(s.mail.copies.lock().await.is_empty());
    assert_eq!(fake.sent.appends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn sent_receipts_are_identity_scoped_and_readable_when_mail_capacity_is_full() {
    let (s, fake) = setup();
    let (cookie, csrf) = login(&s).await;
    let reserved = reserve_copy(&s, &cookie, &csrf, false).await;
    let id = reserved["id"].as_str().unwrap();
    let first = s.mail.admit("owner-subject").await.unwrap();
    let second = s.mail.admit("owner-subject").await.unwrap();
    assert_eq!(
        data(status(&s, &cookie, id).await).await["state"],
        "reserved"
    );
    assert_eq!(
        post_json(
            &s,
            &cookie,
            &csrf,
            &format!("/api/mail/sent/{id}/copy"),
            content()
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    drop((first, second));
    s.mail.copies.lock().await.get_mut(id).unwrap().subject = "another-subject".into();
    assert_eq!(
        status(&s, &cookie, id).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        post_json(
            &s,
            &cookie,
            &csrf,
            &format!("/api/mail/sent/{id}/copy"),
            content()
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    assert_ne!(reserve_copy(&s, &cookie, &csrf, false).await["id"], id);
    assert_eq!(fake.sent.appends.load(Ordering::SeqCst), 0);
}
