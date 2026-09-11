#![cfg(feature = "vault")]
//! Golden credential vault fixtures shared with the Flutter and browser clients.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use shep_profile_core::{
    account::{Connection, IncomingAuth, Protocol, Security, SentCopy, SmtpAuth},
    vault::{self, Error, Field, Key, Slot, Vault},
};
use uuid::Uuid;

const FIXTURES: &str = include_str!("../../credential-vault-fixtures.json");

fn fixtures() -> Value {
    serde_json::from_str(FIXTURES).unwrap()
}
fn id(value: &Value) -> Uuid {
    Uuid::parse_str(value.as_str().expect("UUID text")).unwrap()
}
fn hex(value: &Value) -> Vec<u8> {
    let text = value.as_str().expect("hex text");
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
fn key_for(f: &Value, name: &str, profile: Uuid) -> Key {
    let key = &f["keys"][name];
    Key::new(
        profile,
        id(&f["generation"]),
        id(&key["id"]),
        key["sequence"].as_u64().unwrap() as u32,
        hex(&key["material_hex"]).try_into().unwrap(),
    )
    .unwrap()
}
fn key(f: &Value, name: &str) -> Key {
    key_for(f, name, id(&f["profile"]))
}
fn field(value: &Value) -> Field {
    match value.as_str().unwrap() {
        "incoming" => Field::Incoming,
        "smtp" => Field::Smtp,
        other => panic!("unexpected field {other}"),
    }
}
fn error(value: &Value) -> Error {
    match value.as_str().unwrap() {
        "invalid" => Error::Invalid,
        "upgrade" => Error::Upgrade,
        "too_large" => Error::TooLarge,
        "binding" => Error::Binding,
        "authentication" => Error::Authentication,
        other => panic!("unexpected error {other}"),
    }
}
fn seal_case<'a>(f: &'a Value, name: &str) -> &'a Value {
    f["seals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap()
}

#[test]
fn vault_key_files_round_trip_exact_bytes_without_printing_material() {
    let f = fixtures();
    for name in ["primary", "other"] {
        let expected = f["keys"][name]["file"].as_str().unwrap().as_bytes();
        let decoded = Key::decode(expected, id(&f["profile"]), id(&f["generation"])).unwrap();
        assert_eq!(decoded.id(), id(&f["keys"][name]["id"]));
        assert_eq!(
            u64::from(decoded.sequence()),
            f["keys"][name]["sequence"].as_u64().unwrap()
        );
        assert_eq!(decoded.encode().unwrap(), expected);
        assert_eq!(key(&f, name).encode().unwrap(), expected);
        let printed = format!("{decoded:?}");
        assert!(!printed.contains("AAECAwQF") && !printed.contains("ICEiIyQl"));
    }
}

#[test]
fn vault_endpoint_digest_is_the_documented_canonical_text() {
    let f = fixtures();
    let c = &f["connection"];
    let connection = Connection {
        id: id(&json!("50000000-0000-4000-8000-000000000001")),
        email: "cloud@example.test".into(),
        protocol: Protocol::Imap,
        host: c["host"].as_str().unwrap().into(),
        port: c["port"].as_u64().unwrap() as u16,
        username: c["username"].as_str().unwrap().into(),
        incoming_security: Security::Tls,
        incoming_auth: IncomingAuth::Password,
        smtp_host: c["smtp_host"].as_str().unwrap().into(),
        smtp_port: c["smtp_port"].as_u64().unwrap() as u16,
        smtp_username: c["smtp_username"].as_str().unwrap().into(),
        smtp_security: Security::StartTls,
        smtp_auth: SmtpAuth::Login,
        smtp_separate_password: true,
        sent_copy: SentCopy::Automatic,
        sent_folder: String::new(),
        extra: Default::default(),
    };
    for (name, field) in [("incoming", Field::Incoming), ("smtp", Field::Smtp)] {
        let expected = f["endpoints"][name].as_str().unwrap();
        let text = f["endpoints"][format!("{name}_text")].as_str().unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(text.as_bytes())), expected);
        assert_eq!(vault::endpoint(&connection, field), expected);
    }
    let pop = Connection {
        protocol: Protocol::Pop3,
        ..connection.clone()
    };
    assert_ne!(
        vault::endpoint(&pop, Field::Incoming),
        vault::endpoint(&connection, Field::Incoming)
    );
}

#[test]
fn vault_seals_match_golden_envelopes_and_round_trip() {
    let f = fixtures();
    for case in f["seals"].as_array().unwrap() {
        let key = key(&f, case["key"].as_str().unwrap());
        let endpoint = case["endpoint"].as_str().unwrap();
        let slot = Slot {
            account: id(&case["account"]),
            field: field(&case["field"]),
            revision: case["revision"].as_u64().unwrap(),
            endpoint,
        };
        assert_eq!(
            vault::associated_data(&key, &slot),
            case["aad"].as_str().unwrap().as_bytes(),
            "{}",
            case["name"]
        );
        let secret = case["secret"].as_str().unwrap().as_bytes();
        let nonce: [u8; 12] = hex(&case["nonce_hex"]).try_into().unwrap();
        let sealed = vault::seal(&key, &slot, secret, nonce).unwrap();
        assert_eq!(sealed, case["sealed"].as_str().unwrap(), "{}", case["name"]);
        assert_eq!(&vault::open(&key, &slot, &sealed).unwrap()[..], secret);
    }
}

#[test]
fn vault_rejects_tamper_wrong_key_changed_context_and_unknown_versions() {
    let f = fixtures();
    for case in f["rejections"].as_array().unwrap() {
        let base = seal_case(&f, case["seal"].as_str().unwrap());
        let profile = case
            .get("profile")
            .map(id)
            .unwrap_or_else(|| id(&f["profile"]));
        let name = case
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_else(|| base["key"].as_str().unwrap());
        let key = if case.get("profile").is_some() {
            key_for(&f, name, profile)
        } else {
            key(&f, name)
        };
        let endpoint = case
            .get("endpoint")
            .unwrap_or(&base["endpoint"])
            .as_str()
            .unwrap();
        let slot = Slot {
            account: id(case.get("account").unwrap_or(&base["account"])),
            field: field(case.get("field").unwrap_or(&base["field"])),
            revision: case
                .get("revision")
                .unwrap_or(&base["revision"])
                .as_u64()
                .unwrap(),
            endpoint,
        };
        let sealed = case
            .get("sealed")
            .unwrap_or(&base["sealed"])
            .as_str()
            .unwrap();
        assert_eq!(
            vault::open(&key, &slot, sealed).unwrap_err(),
            error(&case["error"]),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn vault_files_round_trip_and_reject_invalid_foreign_or_newer_files() {
    let f = fixtures();
    let (profile, generation) = (id(&f["profile"]), id(&f["generation"]));
    let expected = f["vault"]["file"].as_str().unwrap().as_bytes();
    let decoded = Vault::decode(expected, profile, generation).unwrap();
    assert_eq!(decoded.key, id(&f["keys"]["primary"]["id"]));
    assert_eq!((decoded.revision, decoded.minor), (4, 0));
    assert_eq!(decoded.encode().unwrap(), expected);
    let key = key(&f, "primary");
    let mut opened = Vec::new();
    for entry in &decoded.entries {
        if let vault::Value::Sealed { sealed, .. } = &entry.value {
            let secret = vault::open(&key, &entry.slot().unwrap(), sealed).unwrap();
            opened.push(String::from_utf8(secret.to_vec()).unwrap());
        } else {
            assert!(entry.slot().is_none());
        }
    }
    assert_eq!(
        opened,
        ["fixture incoming password", "fixture smtp pässwörd ✓"]
    );
    // Entry order in the input does not change the exact output bytes.
    let mut reversed = decoded.clone();
    reversed.entries.reverse();
    assert_eq!(reversed.encode().unwrap(), expected);
    for case in f["file_rejections"].as_array().unwrap() {
        let bytes = case["file"].as_str().unwrap().as_bytes();
        let actual = match case["kind"].as_str().unwrap() {
            "vault" => Vault::decode(bytes, profile, generation).map(|_| ()),
            _ => Key::decode(bytes, profile, generation).map(|_| ()),
        };
        assert_eq!(
            actual.err(),
            Some(error(&case["error"])),
            "{}",
            case["name"]
        );
    }
    let native = &f["native_fixture"];
    let key = Key::decode(
        native["key"].as_str().unwrap().as_bytes(),
        profile,
        generation,
    )
    .unwrap();
    let vault = Vault::decode(
        native["vault"].as_str().unwrap().as_bytes(),
        profile,
        generation,
    )
    .unwrap();
    for (entry, secret) in vault.entries.iter().zip(["incoming", "smtp"]) {
        let vault::Value::Sealed { sealed, .. } = &entry.value else {
            panic!("native fixture entries are sealed")
        };
        let opened = vault::open(&key, &entry.slot().unwrap(), sealed).unwrap();
        assert_eq!(&opened[..], native[secret].as_str().unwrap().as_bytes());
    }
}

#[test]
fn vault_merge_and_canonical_key_are_deterministic_in_any_order() {
    let f = fixtures();
    let (profile, generation) = (id(&f["profile"]), id(&f["generation"]));
    let vaults: Vec<Vault> = f["merge"]["vaults"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            let file = json!({"format":"so.shep.credential-vault","major":1,"minor":0,
                "profile":profile,"generation":generation,"key":f["keys"][v["key"].as_str().unwrap()]["id"],
                "revision":1,"entries":v["entries"]});
            Vault::decode(file.to_string().as_bytes(), profile, generation).unwrap()
        })
        .collect();
    let forward = vault::merge(&vaults);
    let backward = vault::merge(vaults.iter().rev());
    assert_eq!(forward, backward);
    let expected = f["merge"]["expected"].as_array().unwrap();
    assert_eq!(forward.len(), expected.len());
    for case in expected {
        let (entry, key) = &forward[&(id(&case["account"]), field(&case["field"]))];
        assert_eq!(*key, id(&f["keys"][case["key"].as_str().unwrap()]["id"]));
        assert_eq!(entry.device, id(&case["device"]));
        assert_eq!(
            entry.value == vault::Value::Removed,
            case["removed"].as_bool().unwrap()
        );
    }
    let keys: Vec<(Uuid, u32)> = f["canonical_key"]["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| (id(&k[0]), k[1].as_u64().unwrap() as u32))
        .collect();
    let expected = id(&f["canonical_key"]["expected"]);
    assert_eq!(vault::canonical(keys.clone()).unwrap().0, expected);
    assert_eq!(
        vault::canonical(keys.into_iter().rev()).unwrap().0,
        expected
    );
    assert!(vault::canonical(std::iter::empty()).is_none());
}

#[test]
fn vault_bounds_reject_empty_oversized_out_of_range_and_newer_writes() {
    let f = fixtures();
    let key = key(&f, "primary");
    let endpoint = f["endpoints"]["incoming"].as_str().unwrap();
    let slot = Slot {
        account: Uuid::from_u128(5),
        field: Field::Incoming,
        revision: 1,
        endpoint,
    };
    assert_eq!(
        vault::seal(&key, &slot, b"", [0; 12]).unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        vault::seal(&key, &slot, &[b'x'; vault::MAX_SECRET_BYTES + 1], [0; 12]).unwrap_err(),
        Error::TooLarge
    );
    for bad in [
        Slot {
            revision: 0,
            ..slot
        },
        Slot {
            revision: vault::MAX_REVISION + 1,
            ..slot
        },
        Slot {
            endpoint: "ABC",
            ..slot
        },
        Slot {
            account: Uuid::nil(),
            ..slot
        },
    ] {
        assert_eq!(
            vault::seal(&key, &bad, b"secret", [0; 12]).unwrap_err(),
            Error::Invalid
        );
    }
    assert_eq!(
        Key::new(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::nil(),
            1,
            [0; 32]
        )
        .unwrap_err(),
        Error::Invalid
    );
    assert_eq!(
        Key::new(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            0,
            [0; 32]
        )
        .unwrap_err(),
        Error::Invalid
    );
    let (profile, generation) = (id(&f["profile"]), id(&f["generation"]));
    let mut vault = Vault::decode(
        f["vault"]["file"].as_str().unwrap().as_bytes(),
        profile,
        generation,
    )
    .unwrap();
    // A newer minor is readable but this client cannot preserve it on rewrite.
    let newer = f["vault"]["file"]
        .as_str()
        .unwrap()
        .replace("\"minor\":0", "\"minor\":3");
    assert_eq!(
        Vault::decode(newer.as_bytes(), profile, generation)
            .unwrap()
            .minor,
        3
    );
    vault.minor = 3;
    assert_eq!(vault.encode().unwrap_err(), Error::Upgrade);
    vault.minor = 0;
    let template = vault.entries[2].clone();
    vault.entries = (0..=vault::MAX_ENTRIES as u128)
        .map(|n| vault::Entry {
            account: Uuid::from_u128(n + 1),
            ..template.clone()
        })
        .collect();
    assert_eq!(vault.encode().unwrap_err(), Error::TooLarge);
}
