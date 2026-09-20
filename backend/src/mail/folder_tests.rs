use super::*;
use crate::mail::folders::MockFolderProvider;
use serde_json::json;
use shep_mail_core::folders::Mailbox;

#[tokio::test]
async fn authenticated_folder_observations_preserve_exact_provider_metadata() {
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let expected = Mailbox::flat("Projects.with.dot".into());
    let value = expected.clone();
    let mut provider = MockFolderProvider::new();
    provider
        .expect_catalog()
        .times(1)
        .return_once(move || Ok(vec![value]));
    *fake.folder_provider.lock().await = Some(Box::new(provider));
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/folders/catalog",
        json!({"connection": connection()}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Vec<Mailbox>>(&body(response).await).expect("catalog"),
        std::slice::from_ref(&expected)
    );

    let value = expected.clone();
    let mut provider = MockFolderProvider::new();
    provider
        .expect_plan()
        .with(
            mockall::predicate::eq(None),
            mockall::predicate::eq("Projects.with.dot".to_string()),
        )
        .times(1)
        .return_once(move |_, _| Ok(value));
    *fake.folder_provider.lock().await = Some(Box::new(provider));
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/folders/plan",
        json!({"connection": connection(), "parent": null, "name": "Projects.with.dot"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        serde_json::from_str::<Mailbox>(&body(response).await).expect("plan"),
        expected.clone()
    );

    let mut provider = MockFolderProvider::new();
    provider
        .expect_inspect()
        .with(mockall::predicate::eq(expected.clone()))
        .times(1)
        .return_once(|_| Ok(None));
    *fake.folder_provider.lock().await = Some(Box::new(provider));
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/folders/inspect",
        json!({"connection": connection(), "target": expected}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body(response).await, "null");
}

#[tokio::test]
async fn folder_routes_refuse_unowned_destinations_invalid_input_and_bad_csrf_before_provider() {
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let mut unowned = connection();
    unowned["account"]["host"] = json!("unowned.example.test");
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/folders/catalog",
        json!({"connection": unowned}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/folders/plan",
        json!({"connection": connection(), "parent": null, "name": "bad\r\nCREATE other"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = post_json(
        &state,
        &cookie,
        "wrong",
        "/api/mail/folders/catalog",
        json!({"connection": connection()}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failed_observation_never_means_absent_or_leaks_provider_details() {
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let mut provider = MockFolderProvider::new();
    provider
        .expect_inspect()
        .times(1)
        .return_once(|_| Err(anyhow::anyhow!("synthetic folder password")));
    *fake.folder_provider.lock().await = Some(Box::new(provider));
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/folders/inspect",
        json!({"connection": connection(), "target": Mailbox::flat("Fixture".into())}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let error = body(response).await;
    assert!(!error.contains("password"));
    assert!(error.contains("could not be checked"));
}

#[tokio::test]
async fn folder_create_returns_its_receipt_before_any_catalog_request() {
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let target = Mailbox::flat("Fixture".into());
    let mut provider = MockFolderProvider::new();
    provider.expect_inspect().times(1).return_once(|_| Ok(None));
    provider
        .expect_create()
        .with(mockall::predicate::eq(target.clone()))
        .times(1)
        .return_once(|_| shep_mail_core::folder_actions::creation::CreateOutcome::Acknowledged);
    *fake.folder_provider.lock().await = Some(Box::new(provider));
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/folders/create",
        json!({"connection": connection(), "target": target}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let receipt: serde_json::Value = serde_json::from_str(&body(response).await).expect("receipt");
    assert_eq!(receipt, json!({"state": "acknowledged", "target": target}));
}

#[tokio::test]
async fn subtree_routes_check_authentication_and_frozen_identity_before_provider() {
    use shep_mail_core::folder_actions::{Action, Plan};
    use shep_mail_core::folders::Tree;
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let plan = Plan::new(
        &Tree::new(&[Mailbox::flat("Fixture".into())]),
        "Fixture",
        Action::Delete,
    )
    .expect("fixture plan");
    for route in ["review", "step", "check-step"] {
        let request = if route == "review" {
            json!({"connection": connection(), "source": "Fixture", "action": "Delete"})
        } else {
            json!({"connection": connection(), "plan": plan, "completed": 0})
        };
        let path = format!("/api/mail/folders/{route}");
        let response = post_json(&state, &cookie, "wrong", &path, request.clone()).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let mut unowned = request;
        unowned["connection"]["account"]["host"] = json!("unowned.example.test");
        let response = post_json(&state, &cookie, &csrf, &path, unowned).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    let mut invalid = serde_json::to_value(plan).expect("serialise plan");
    invalid["source"] = json!("Other");
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/folders/step",
        json!({"connection": connection(), "plan": invalid, "completed": 0}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn subtree_review_returns_the_exact_catalog_and_mutation_receipt_before_cache_work() {
    use shep_mail_core::folder_actions::{Action, Outcome, Plan, Step};
    use shep_mail_core::folders::Tree;
    let (state, fake) = setup();
    let (cookie, csrf) = login(&state).await;
    let catalog = vec![
        Mailbox::flat("Fixture".into()),
        Mailbox::flat("Unrelated".into()),
    ];
    let mut provider = MockFolderProvider::new();
    let observed = catalog.clone();
    provider
        .expect_catalog()
        .times(1)
        .return_once(move || Ok(observed));
    provider.expect_apply().never();
    *fake.folder_provider.lock().await = Some(Box::new(provider));
    let response = post_json(&state, &cookie, &csrf, "/api/mail/folders/review", json!({"connection": connection(), "source": "Fixture", "action": {"Rename":{"name":"Renamed"}}})).await;
    assert_eq!(response.status(), StatusCode::OK);
    let response: serde_json::Value =
        serde_json::from_str(&body(response).await).expect("review response");
    assert_eq!(
        response["catalog"],
        serde_json::to_value(&catalog).expect("catalog")
    );
    let plan = Plan::new(
        &Tree::new(&catalog),
        "Fixture",
        Action::Rename {
            name: "Renamed".into(),
        },
    )
    .expect("plan");
    assert_eq!(
        response["plan"],
        serde_json::to_value(&plan).expect("plan json")
    );
    let mut provider = MockFolderProvider::new();
    provider
        .expect_catalog()
        .times(1)
        .return_once(move || Ok(catalog));
    provider
        .expect_apply()
        .with(mockall::predicate::eq(Step::Rename {
            source: "Fixture".into(),
            destination: "Renamed".into(),
        }))
        .times(1)
        .return_once(|_| Outcome::Applied);
    *fake.folder_provider.lock().await = Some(Box::new(provider));
    let response = post_json(
        &state,
        &cookie,
        &csrf,
        "/api/mail/folders/step",
        json!({"connection": connection(), "plan": plan, "completed": 0}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response: serde_json::Value =
        serde_json::from_str(&body(response).await).expect("receipt response");
    assert_eq!(
        response,
        json!({"state":"acknowledged","step":{"Rename":{"source":"Fixture","destination":"Renamed"}}})
    );
}
