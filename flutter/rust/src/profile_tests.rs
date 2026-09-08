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
