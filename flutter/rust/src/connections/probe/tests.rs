use super::*;
use crate::tests::{profile, request, seed};
use serde_json::json;

struct HeldProbe {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait]
impl ConnectionProbe for HeldProbe {
    async fn check(&self, _: Account, _: SecretString, _: bool) -> Result<()> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(())
    }
}

#[tokio::test]
async fn cancellation_after_provider_start_does_not_wait_for_probe() {
    let (_directory, profile) = profile().await;
    seed(&profile, 1).await;
    let attempt = prepared(&profile).await;
    let api = HeldProbe {
        started: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    };
    let (result, ()) = tokio::join!(
        run(
            &profile,
            &api,
            attempt.clone(),
            SecretString::from("secret"),
            false
        ),
        async {
            api.started.notified().await;
            request(
                &profile,
                json!({"op":"abandon_account_connection","attempt":attempt}),
            )
            .await;
            api.release.notify_one();
        }
    );
    result.expect("owned readonly probe drains");
    let mut refused = MockConnectionProbe::new();
    refused.expect_check().times(0);
    assert!(
        run(
            &profile,
            &refused,
            attempt,
            SecretString::from("secret"),
            true
        )
        .await
        .is_err()
    );
}

async fn prepared(profile: &crate::api::MobileProfile) -> String {
    let account = request(profile, json!({"op":"accounts"})).await["accounts"][0].clone();
    request(
        profile,
        json!({"op":"prepare_account","account":account,"expected":account}),
    )
    .await["attempt"]
        .as_str()
        .expect("attempt")
        .to_owned()
}

#[tokio::test]
async fn removal_after_old_validation_prevents_bound_probe() {
    let (_directory, profile) = profile().await;
    seed(&profile, 1).await;
    let attempt = prepared(&profile).await;
    request(
        &profile,
        json!({"op":"validate_account_connection","attempt":attempt}),
    )
    .await;
    let review = request(
        &profile,
        json!({"op":"account_removal_preview","id":"fixture"}),
    )
    .await;
    request(
        &profile,
        json!({"op":"remove_account","review":review,"discard_unresolved":true}),
    )
    .await;
    let mut api = MockConnectionProbe::new();
    api.expect_check().times(0);
    assert!(
        run(&profile, &api, attempt, SecretString::from("secret"), false)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancellation_after_old_validation_prevents_bound_probe() {
    let (_directory, profile) = profile().await;
    seed(&profile, 1).await;
    let attempt = prepared(&profile).await;
    request(
        &profile,
        json!({"op":"validate_account_connection","attempt":attempt}),
    )
    .await;
    request(
        &profile,
        json!({"op":"abandon_account_connection","attempt":attempt}),
    )
    .await;
    let mut api = MockConnectionProbe::new();
    api.expect_check().times(0);
    assert!(
        run(&profile, &api, attempt, SecretString::from("secret"), false)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn busy_account_or_provider_capacity_never_waits_with_other_owner() {
    let (_directory, profile) = profile().await;
    seed(&profile, 1).await;
    let attempt = prepared(&profile).await;
    let mut api = MockConnectionProbe::new();
    api.expect_check().times(0);
    let owner = profile.operations.account("fixture").await;
    assert!(
        run(
            &profile,
            &api,
            attempt.clone(),
            SecretString::from("secret"),
            false
        )
        .await
        .is_err()
    );
    drop(owner);
    let permits: Vec<_> = (0..8)
        .map(|_| {
            profile
                .operations
                .try_connection_capacity()
                .expect("capacity")
        })
        .collect();
    assert!(
        run(&profile, &api, attempt, SecretString::from("secret"), false)
            .await
            .is_err()
    );
    assert!(profile.operations.try_account("fixture").await.is_ok());
    drop(permits);
}

#[tokio::test]
async fn bound_probe_uses_saved_account_and_reports_provider_failure() {
    let (_directory, profile) = profile().await;
    seed(&profile, 1).await;
    let attempt = prepared(&profile).await;
    let mut api = MockConnectionProbe::new();
    api.expect_check()
        .withf(|account, _, smtp| account.id == "fixture" && !smtp)
        .times(1)
        .returning(|_, _, _| Ok(()));
    run(
        &profile,
        &api,
        attempt.clone(),
        SecretString::from("secret"),
        false,
    )
    .await
    .expect("probe");
    let mut api = MockConnectionProbe::new();
    api.expect_check()
        .times(1)
        .returning(|_, _, _| anyhow::bail!("private failure"));
    let failure = run(&profile, &api, attempt, SecretString::from("secret"), true)
        .await
        .expect_err("refusal");
    assert!(!failure.to_string().contains("private"));
}
