use super::*;
use serde_json::json;
use shep_calendar_core::{Source, http::MockCalendarProvider};

const REQUEST: &str = "01234567-89ab-cdef-0123-456789abcdef";

fn grant(access: &str, expired: bool) -> Grant {
    Grant {
        generation: token(),
        refresh_owner: Arc::new(Mutex::new(())),
        access_token: Zeroizing::new(access.into()),
        refresh_token: Some(Zeroizing::new("refresh".into())),
        expires: Instant::now() + Duration::from_secs(if expired { 0 } else { 3600 }),
        requested: Requested {
            drive: true,
            calendar: Calendar::Edit,
        },
        access: Access {
            drive: true,
            calendar_read: true,
            calendar_write: true,
        },
        principal: None,
    }
}

#[tokio::test]
async fn held_refresh_releases_global_grants_and_cannot_restore_disconnected_or_replaced_grant() {
    for (reconnect, drive) in [(false, false), (true, false), (false, true), (true, true)] {
        let (mut state, _) = crate::tests::state("owner@example.test");
        let (cookie, _) = crate::tests::login(&state).await;
        let mut headers = HeaderMap::new();
        headers.insert("cookie", cookie.parse().expect("cookie"));
        let key = session_key(&headers).expect("key");
        let session = state.sessions.lock().await[&key].clone();
        state
            .profiles
            .grants
            .lock()
            .await
            .insert(key.clone(), grant("old", true));
        let (started, observing) = tokio::sync::oneshot::channel();
        let (release, held) = tokio::sync::oneshot::channel();
        let mut provider = MockProfileProvider::new();
        provider.expect_refresh().times(1).return_once(move |_| {
            Box::pin(async move {
                started.send(()).expect("notify start");
                held.await.expect("release refresh");
                Ok(Tokens {
                    access_token: Zeroizing::new("late-refreshed".into()),
                    refresh_token: None,
                    expires_in: 3600,
                    scope: None,
                    subject: Some("owner-subject".into()),
                })
            })
        });
        state.provider = Arc::new(provider);
        let worker_state = state.clone();
        let worker = tokio::spawn(async move {
            if drive {
                grants::access_token(&worker_state, &session, &headers, grants::Permission::Drive)
                    .await
                    .map_err(|(_, message)| ProviderFailure::waiting(message))
            } else {
                access_token(&worker_state, &session, &headers, true).await
            }
        });
        observing.await.expect("refresh started");
        let mut grants = tokio::time::timeout(Duration::from_secs(1), state.profiles.grants.lock())
            .await
            .expect("unrelated grants remain available");
        grants.remove(&key);
        if reconnect {
            grants.insert(key.clone(), grant("new-connection", false));
        }
        drop(grants);
        release.send(()).expect("release");
        assert_eq!(
            worker
                .await
                .expect("worker")
                .expect_err("old generation rejected")
                .kind,
            FailureKind::Waiting
        );
        let grants = state.profiles.grants.lock().await;
        assert_eq!(
            grants.get(&key).map(|grant| grant.access_token.as_str()),
            reconnect.then_some("new-connection")
        );
    }
}

#[tokio::test]
async fn current_token_requires_a_current_session_without_refresh() {
    let (mut state, _) = crate::tests::state("owner@example.test");
    let (cookie, _) = crate::tests::login(&state).await;
    let mut headers = HeaderMap::new();
    headers.insert("cookie", cookie.parse().expect("cookie"));
    let key = session_key(&headers).expect("key");
    let session = state.sessions.lock().await[&key].clone();
    state
        .profiles
        .grants
        .lock()
        .await
        .insert(key.clone(), grant("current", false));
    let mut provider = MockProfileProvider::new();
    provider.expect_refresh().times(0);
    state.provider = Arc::new(provider);
    state.sessions.lock().await.remove(&key);
    assert!(
        grants::access_token(&state, &session, &headers, grants::Permission::Drive)
            .await
            .is_err()
    );
    assert!(
        access_token(&state, &session, &headers, true)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn concurrent_refreshes_share_one_grant_owner_and_return_current_token() {
    let (mut state, _) = crate::tests::state("owner@example.test");
    let (cookie, _) = crate::tests::login(&state).await;
    let mut headers = HeaderMap::new();
    headers.insert("cookie", cookie.parse().expect("cookie"));
    let key = session_key(&headers).expect("key");
    let session = state.sessions.lock().await[&key].clone();
    state
        .profiles
        .grants
        .lock()
        .await
        .insert(key, grant("expired", true));
    let mut provider = MockProfileProvider::new();
    provider.expect_refresh().times(1).returning(|_| {
        Box::pin(async {
            tokio::task::yield_now().await;
            Ok(Tokens {
                access_token: Zeroizing::new("current".into()),
                refresh_token: None,
                expires_in: 3600,
                scope: None,
                subject: Some("owner-subject".into()),
            })
        })
    });
    state.provider = Arc::new(provider);
    let (first, second) = tokio::join!(
        grants::access_token(&state, &session, &headers, grants::Permission::Drive),
        access_token(&state, &session, &headers, true)
    );
    assert_eq!(first.expect("first").as_str(), "current");
    assert_eq!(second.expect("second").as_str(), "current");
}

fn event() -> Event {
    Event {
        id: "event".into(),
        source_id: "calendar".into(),
        title: "Retained title".into(),
        start: "2026-09-20T10:00:00Z".parse().expect("date"),
        end: "2026-09-20T11:00:00Z".parse().expect("date"),
        location: "Room".into(),
        description: "Retained description".into(),
        all_day: false,
        etag: Some("v1".into()),
        remote_url: Some("event".into()),
    }
}

fn mutation() -> Mutation {
    let before = event();
    let mut after = before.clone();
    after.title = "Newer title".into();
    Mutation::Save {
        before: Some(before),
        after,
    }
}

fn source(read_only: bool) -> Source {
    Source {
        id: "calendar".into(),
        name: "Work".into(),
        read_only,
    }
}

#[tokio::test]
async fn edit_preserves_provider_identity_and_returns_exact_receipt() {
    let mut provider = MockCalendarProvider::new();
    provider
        .expect_sources()
        .times(1)
        .returning(|_| Box::pin(async { Ok(vec![source(false)]) }));
    provider
        .expect_save()
        .times(1)
        .withf(|token, request, event| {
            token == "token"
                && request == REQUEST
                && event.etag.as_deref() == Some("v1")
                && event.remote_url.as_deref() == Some("event")
                && event.description == "Retained description"
        })
        .returning(|_, _, event| {
            let mut saved = event.clone();
            saved.etag = Some("v2".into());
            Box::pin(async move { Ok(saved) })
        });
    let value = perform(
        &provider,
        "token",
        Operation::Mutate {
            request_id: REQUEST.into(),
            mutation: mutation(),
        },
    )
    .await
    .expect("saved");
    assert_eq!(value["receipt"]["before"]["etag"], "v1");
    assert_eq!(value["receipt"]["after"]["etag"], "v2");
    assert_eq!(value["receipt"]["request_id"], REQUEST);
}

#[tokio::test]
async fn changed_edit_identity_never_reaches_provider() {
    let provider = MockCalendarProvider::new();
    let mut changed = event();
    changed.etag = None;
    changed.remote_url = None;
    let error = perform(
        &provider,
        "token",
        Operation::Mutate {
            request_id: REQUEST.into(),
            mutation: Mutation::Save {
                before: Some(event()),
                after: changed,
            },
        },
    )
    .await
    .expect_err("identity refusal");
    assert_eq!(error.kind, FailureKind::Rejected);
}

#[tokio::test]
async fn read_only_or_failed_source_observation_never_dispatches_write() {
    for fail in [false, true] {
        let mut provider = MockCalendarProvider::new();
        provider.expect_sources().times(1).returning(move |_| {
            Box::pin(async move {
                if fail {
                    Err(ProviderFailure::waiting("offline"))
                } else {
                    Ok(vec![source(true)])
                }
            })
        });
        let error = perform(
            &provider,
            "token",
            Operation::Mutate {
                request_id: REQUEST.into(),
                mutation: mutation(),
            },
        )
        .await
        .expect_err("no dispatch");
        assert_eq!(
            error.kind,
            if fail {
                FailureKind::Waiting
            } else {
                FailureKind::Rejected
            }
        );
    }
}

#[tokio::test]
async fn uncertain_save_is_not_retried_or_followed_by_cache_read() {
    let mut provider = MockCalendarProvider::new();
    provider
        .expect_sources()
        .times(1)
        .returning(|_| Box::pin(async { Ok(vec![source(false)]) }));
    provider
        .expect_save()
        .times(1)
        .returning(|_, _, _| Box::pin(async { Err(ProviderFailure::uncertain("reply lost")) }));
    let error = perform(
        &provider,
        "token",
        Operation::Mutate {
            request_id: REQUEST.into(),
            mutation: mutation(),
        },
    )
    .await
    .expect_err("unknown");
    assert_eq!(error.kind, FailureKind::Uncertain);
}

#[tokio::test]
async fn inspection_reads_stable_create_identity_without_mutating() {
    let mut provider = MockCalendarProvider::new();
    let stable = format!("shep{}", REQUEST.replace('-', ""));
    provider
        .expect_read()
        .times(1)
        .withf(move |_, event| event.id == stable && event.source_id == "calendar")
        .returning(|_, _| Box::pin(async { Ok(None) }));
    let mut after = event();
    after.etag = None;
    after.remote_url = None;
    let value = perform(
        &provider,
        "token",
        Operation::Inspect {
            request_id: REQUEST.into(),
            mutation: Mutation::Save {
                before: None,
                after,
            },
        },
    )
    .await
    .expect("observed");
    assert!(value["current"].is_null());
}

#[tokio::test]
async fn route_requires_current_identity_and_calendar_grant() {
    use axum::{body::Body, http::Request as HttpRequest};
    use tower::ServiceExt;
    let (state, _) = crate::tests::state("owner@example.test");
    let state = state.with_calendar_provider(Arc::new(MockCalendarProvider::new()));
    let (cookie, csrf) = crate::tests::login(&state).await;
    for (binding, operation, expected) in [
        ("wrong".into(), json!({"kind":"sources"}), "rejected"),
        (
            hash("fixture-client\0owner-subject"),
            json!({"kind":"sources"}),
            "waiting",
        ),
        (
            hash("fixture-client\0owner-subject"),
            json!({"kind":"mutate","request_id":"invalid","mutation":mutation()}),
            "rejected",
        ),
    ] {
        let response = crate::app(state.clone())
            .oneshot(
                HttpRequest::post("/api/calendar")
                    .header("cookie", &cookie)
                    .header("x-shep-csrf", &csrf)
                    .header("origin", "https://shep.example.test")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"binding":binding,"operation":operation}).to_string(),
                    ))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let value: Value = serde_json::from_str(&crate::tests::body(response).await).expect("json");
        assert_eq!(value["state"], expected);
    }
}

#[tokio::test]
async fn granted_route_returns_observation_without_exposing_credentials() {
    use axum::{body::Body, http::Request as HttpRequest};
    use tower::ServiceExt;
    let (state, _) = crate::tests::state("owner@example.test");
    let mut provider = MockCalendarProvider::new();
    provider
        .expect_sources()
        .times(1)
        .withf(|token| token == "calendar-fixture-secret")
        .returning(|_| Box::pin(async { Ok(vec![source(false)]) }));
    let state = state.with_calendar_provider(Arc::new(provider));
    let (cookie, csrf) = crate::tests::login(&state).await;
    let mut headers = HeaderMap::new();
    headers.insert("cookie", cookie.parse().expect("cookie header"));
    let key = session_key(&headers).expect("session key");
    state.profiles.grants.lock().await.insert(
        key,
        Grant {
            generation: token(),
            refresh_owner: Arc::new(Mutex::new(())),
            access_token: Zeroizing::new("calendar-fixture-secret".into()),
            refresh_token: None,
            expires: Instant::now() + Duration::from_secs(3600),
            requested: Requested {
                drive: false,
                calendar: Calendar::Read,
            },
            access: Access {
                drive: false,
                calendar_read: true,
                calendar_write: false,
            },
            principal: None,
        },
    );
    let response = crate::app(state).oneshot(HttpRequest::post("/api/calendar")
        .header("cookie", cookie).header("x-shep-csrf", csrf)
        .header("origin", "https://shep.example.test").header("content-type", "application/json")
        .body(Body::from(json!({"binding":hash("fixture-client\0owner-subject"),"operation":{"kind":"sources"}}).to_string())).expect("request"))
        .await.expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let text = crate::tests::body(response).await;
    assert!(!text.contains("calendar-fixture-secret"));
    let value: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(value["state"], "observed");
    assert_eq!(value["value"]["sources"][0]["id"], "calendar");
}
