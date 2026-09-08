use serde_json::json;
use shep_mail_core::{
    model::*,
    profiles::{
        self,
        codec::{Action, Operation},
    },
};

fn candidate() -> Account {
    let op = Operation::decode(include_bytes!("../../profile-operation.json")).unwrap();
    let Action::AccountConnection { account } = &op.changes[0].action else {
        panic!()
    };
    profiles::review_account(account, "Équipe").unwrap()
}

#[test]
fn profile_account_mapping_resolves_legacy_defaults_for_both_clients() {
    for protocol in [Protocol::Imap, Protocol::Pop3] {
        for port in [465, 587, 2525] {
            for auth in [
                SmtpAuth::Automatic,
                SmtpAuth::Plain,
                SmtpAuth::Login,
                SmtpAuth::None,
            ] {
                let mut account = candidate();
                account.protocol = protocol;
                account.smtp_port = port;
                account.smtp_security = None;
                account.smtp_username.clear();
                account.smtp_auth = auth;
                let changes =
                    profiles::export_account(&account, account.id.parse().unwrap()).unwrap();
                let Action::AccountConnection { account: wire } = &changes[0].action else {
                    panic!()
                };
                let returned = profiles::review_account(wire, &account.name).unwrap();
                assert_eq!(returned.smtp_username, account.username);
                assert_eq!(returned.smtp_security, Some(account.smtp_security()));
                account.smtp_security = Some(account.smtp_security());
                account.smtp_username = account.username.clone();
                assert_eq!(returned, account);
            }
        }
    }
}

#[test]
fn profile_identity_and_unknown_connection_options_require_explicit_review() {
    let account = candidate();
    assert!(profiles::export_account(&account, uuid::Uuid::new_v4()).is_err());
    let mut legacy = account.clone();
    legacy.id = "legacy-local-account".into();
    let changes = profiles::export_account(&legacy, account.id.parse().unwrap()).unwrap();
    let Action::AccountConnection { mut account } = changes[0].action.clone() else {
        panic!()
    };
    account
        .extra
        .insert("future_security_option".into(), json!({"required":true}));
    assert!(
        profiles::review_account(&account, "Name")
            .unwrap_err()
            .to_string()
            .contains("Update Shep")
    );
}

#[test]
fn profile_mapping_never_relies_on_serializing_local_account_state() {
    let account = candidate();
    let changes = profiles::export_account(&account, account.id.parse().unwrap()).unwrap();
    let Action::AccountConnection { account: wire } = &changes[0].action else {
        panic!()
    };
    assert!(wire.extra.is_empty());
    let encoded = serde_json::to_value(wire).unwrap();
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("../../profile-operation.json")).unwrap();
    assert_eq!(encoded, golden["changes"][0]["account"]);
    assert_eq!(encoded["smtp_username"], "outgoing@example.test");
    assert_eq!(encoded["smtp_separate_password"], true);
    assert_eq!(encoded["sent_folder"], "Sent/Équipe");
    assert!(encoded.get("name").is_none());
}
