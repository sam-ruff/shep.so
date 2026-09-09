use serde_json::{Value, json};
use shep_profile_core::{Action, Error, MAX_CHANGES, MAX_PARENTS, MAX_RECORD_BYTES, Operation};

fn golden() -> Value {
    serde_json::from_str(include_str!("../../profile-operation.json")).unwrap()
}
fn decode(value: &Value) -> Result<Operation, Error> {
    Operation::decode(&serde_json::to_vec(value).unwrap())
}

#[test]
fn common_native_browser_contract_cases() {
    let cases: Value = serde_json::from_str(include_str!("../../profile-cases.json")).unwrap();
    for case in cases["cases"].as_array().unwrap() {
        let mut value = golden();
        for patch in case["patches"].as_array().unwrap() {
            *value.pointer_mut(patch["path"].as_str().unwrap()).unwrap() = patch["value"].clone();
        }
        let result = decode(&value);
        if let Some(error) = case["error"].as_str() {
            assert_eq!(
                format!("{:?}", result.unwrap_err()),
                error,
                "{}",
                case["name"]
            );
        } else {
            let operation = result.unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
            let returned: Value = serde_json::from_slice(&operation.encode().unwrap()).unwrap();
            assert_eq!(
                returned, value,
                "unknown optional data must survive: {}",
                case["name"]
            );
        }
    }
}

#[test]
fn editing_known_fields_retains_future_envelope_change_and_connection_data() {
    let mut value = golden();
    value["changes"][0]["account"]["future_server"] = json!({"a":[1,2,{"b":"é"}]});
    let mut operation = decode(&value).unwrap();
    let Action::AccountName { name, .. } = &mut operation.changes[1].action else {
        panic!()
    };
    *name = "Renamed".into();
    value["changes"][1]["name"] = "Renamed".into();
    let encoded: Value = serde_json::from_slice(&operation.encode().unwrap()).unwrap();
    assert_eq!(encoded, value);
}

#[test]
fn duplicate_fields_and_extension_collisions_are_never_last_writer_wins() {
    let raw = serde_json::to_string(&golden()).unwrap();
    for invalid in [
        raw.replacen("\"major\":1", "\"major\":2,\"major\":1", 1),
        raw.replacen(
            "\"smtp_security\":\"StartTls\"",
            "\"smtp_security\":\"Tls\",\"smtp_security\":\"StartTls\"",
            1,
        ),
        raw.replacen(
            "\"future_optional\":",
            "\"future_optional\":{},\"future_optional\":",
            1,
        ),
    ] {
        assert_eq!(
            Operation::decode(invalid.as_bytes()).unwrap_err(),
            Error::Invalid
        );
    }
    let mut operation = decode(&golden()).unwrap();
    operation
        .extra
        .insert("operation".into(), json!(operation.operation.to_string()));
    assert_eq!(operation.encode().unwrap_err(), Error::Invalid);
    let mut operation = decode(&golden()).unwrap();
    operation.changes[0]
        .extra
        .insert("kind".into(), "profile_removed".into());
    assert_eq!(operation.encode().unwrap_err(), Error::Invalid);
}

#[test]
fn malformed_bounded_records_never_echo_remote_content() {
    for raw in [
        b"not-json secret-token".as_slice(),
        b"{}{}",
        b"null",
        b"[]",
        b"{\"major\":1}",
        &[0xff],
    ] {
        let error = Operation::decode(raw).unwrap_err().to_string();
        assert!(!error.contains("secret-token"));
    }
    assert_eq!(
        Operation::decode(&vec![b' '; MAX_RECORD_BYTES + 1]).unwrap_err(),
        Error::TooLarge
    );
    let mut value = golden();
    value["future_optional"] = json!("x".repeat(MAX_RECORD_BYTES));
    assert_eq!(decode(&value).unwrap_err(), Error::TooLarge);
    let mut operation = decode(&golden()).unwrap();
    operation
        .extra
        .insert("large".into(), json!("x".repeat(MAX_RECORD_BYTES)));
    assert_eq!(operation.encode().unwrap_err(), Error::TooLarge);
    let mut deep = Value::Null;
    for _ in 0..100 {
        deep = json!([deep]);
    }
    operation.extra.clear();
    operation.extra.insert("deep".into(), deep);
    assert_eq!(operation.encode().unwrap_err(), Error::Invalid);
    let raw = format!("{}0{}", "[".repeat(150), "]".repeat(150));
    assert_eq!(
        Operation::decode(raw.as_bytes()).unwrap_err(),
        Error::Invalid
    );
}

#[test]
fn parent_and_change_bounds_do_not_limit_total_history() {
    let mut value = golden();
    value["parents"] = json!(
        (1..=MAX_PARENTS)
            .map(|n| format!("90000000-0000-4000-8000-{n:012x}"))
            .collect::<Vec<_>>()
    );
    decode(&value).unwrap();
    value["parents"]
        .as_array_mut()
        .unwrap()
        .push(json!("90000000-0000-4000-8000-999999999999"));
    assert_eq!(decode(&value).unwrap_err(), Error::Invalid);
    value = golden();
    value["changes"] = json!((1..=MAX_CHANGES).map(|n| json!({"kind":"account_name","id":format!("90000000-0000-4000-8000-{n:012x}"),"name":"Fixture"})).collect::<Vec<_>>());
    decode(&value).unwrap();
    value["changes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind":"profile_name","name":"Extra"}));
    assert_eq!(decode(&value).unwrap_err(), Error::Invalid);
    // There is no whole-history collection in this codec: many bounded records
    // remain independently decodable, with distinct durable operation IDs.
    for n in 1..=300 {
        let mut value = golden();
        value["operation"] = json!(format!("90000000-0000-4000-8000-{n:012x}"));
        decode(&value).unwrap();
    }
}

#[test]
fn explicit_security_is_required_and_local_fields_cannot_hide_in_extensions() {
    for field in [
        "incoming_security",
        "incoming_auth",
        "smtp_security",
        "smtp_auth",
        "smtp_username",
        "smtp_separate_password",
    ] {
        let mut value = golden();
        value["changes"][0]["account"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert_eq!(decode(&value).unwrap_err(), Error::Upgrade, "{field}");
    }
    for field in [
        "password",
        "smtp_password",
        "access_token",
        "client_secret",
        "credential_slot",
        "google_grant",
        "last_backup",
        "delivery_journal",
        "window_position",
    ] {
        let mut value = golden();
        value["changes"][0]["account"][field] = "fictional-secret".into();
        let error = decode(&value).unwrap_err();
        assert_eq!(error, Error::LocalData);
        assert!(!error.to_string().contains("fictional-secret"));
    }
}
