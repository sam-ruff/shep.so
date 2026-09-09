use crate::tests::{profile, request, seed};
use serde_json::{Value, json};

#[tokio::test]
async fn profile_codec_bridge_preserves_common_fixtures_without_applying_them() {
    let (_dir, profile) = profile().await;
    seed(&profile, 3).await;
    let original = request(&profile, json!({"op":"accounts"})).await;
    // Profile metadata validation owns separate capacity from network/mail work.
    let _held_network = profile.operations.hold_network_capacity().await;
    let cases: Value =
        serde_json::from_str(include_str!("../../../shared/profile-cases.json")).unwrap();
    let golden: Value =
        serde_json::from_str(include_str!("../../../shared/profile-operation.json")).unwrap();
    for case in cases["cases"].as_array().unwrap() {
        let mut input = golden.clone();
        for patch in case["patches"].as_array().unwrap() {
            *input.pointer_mut(patch["path"].as_str().unwrap()).unwrap() = patch["value"].clone();
        }
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            profile.request(
                json!({"op":"validate_profile_operation","record":input.to_string()}).to_string(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        let result: Value = serde_json::from_str(&response).unwrap();
        if case["error"].is_null() {
            let output: Value =
                serde_json::from_str(result["data"]["record"].as_str().unwrap()).unwrap();
            assert_eq!(output, input, "{}", case["name"]);
        } else {
            let error = result["error"].as_str().unwrap();
            assert!(!error.contains("fictional-do-not-log"));
        }
    }
    assert_eq!(request(&profile, json!({"op":"accounts"})).await, original);
    profile
        .database
        .read(|db| {
            let n: i64 = db.query_row("SELECT COUNT(*) FROM mail", [], |r| r.get(0))?;
            assert_eq!(n, 3);
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn profile_history_bridges_two_native_devices_with_provider_capacity_occupied() {
    let (_one, desktop) = profile().await;
    let (_two, phone) = profile().await;
    seed(&desktop, 3).await;
    seed(&phone, 2).await;
    let original = request(&phone, json!({"op":"accounts"})).await;
    let _held = phone.operations.hold_network_capacity().await;
    let binding = json!({"namespace":"so.shep.fixture","principal":"drive:fixture-owner","profile":"00000000-0000-0000-0000-000000000001","generation":"00000000-0000-0000-0000-000000000002"});
    let command =
        |command: Value| json!({"op":"profile_history","binding":binding,"command":command});
    let initial = request(&desktop, command(json!({"kind":"state"}))).await;
    let edit = json!({"operation":"00000000-0000-0000-0000-000000000010","expected_revision":initial["value"]["revision"],"changes":[{"kind":"setting","key":"appearance","value":"Dark"}]});
    request(&desktop, command(json!({"kind":"edit","edit":edit}))).await;
    let outgoing = request(&desktop, command(json!({"kind":"next_upload"}))).await;
    let received = request(
        &phone,
        command(json!({"kind":"import","record":outgoing["value"]["record"]})),
    )
    .await;
    assert_eq!(received["value"]["fields"], 1);
    assert_eq!(received["value"]["queued"], 0);
    request(
        &desktop,
        json!({"op":"close_profile_history","binding":binding}),
    )
    .await;
    let reopened = request(&desktop, command(json!({"kind":"state"}))).await;
    assert_eq!(reopened["value"]["queued"], 1);
    let retry = request(&desktop, command(json!({"kind":"edit","edit":edit}))).await;
    assert_eq!(retry["value"]["operations"], 1);
    assert_eq!(request(&phone, json!({"op":"accounts"})).await, original);
    let page = request(&phone, json!({"op":"page","folder":"Inbox","offset":0})).await;
    assert_eq!(page["total"], 2);
    let alternate = json!({"namespace":"so.shep.fixture","principal":"drive:other-owner","profile":binding["profile"],"generation":binding["generation"]});
    let switched = request(
        &phone,
        json!({"op":"profile_history","binding":alternate,"command":{"kind":"state"}}),
    )
    .await;
    assert_eq!(switched["value"]["operations"], 0);
    let stale: Value = serde_json::from_str(
        &phone
            .request(json!({"op":"close_profile_history","binding":binding}).to_string())
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(stale["error"].as_str().unwrap().contains("changed"));
    let restored = request(&phone, command(json!({"kind":"state"}))).await;
    assert_eq!(restored["value"]["operations"], 1);
    request(
        &phone,
        json!({"op":"close_profile_history","binding":binding}),
    )
    .await;
    request(
        &desktop,
        json!({"op":"close_profile_history","binding":binding}),
    )
    .await;
}
