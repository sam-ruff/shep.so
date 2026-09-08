//! Explicit account metadata mappings shared by desktop and Flutter. Callers
//! retain the operation envelope; these candidates never activate credentials.
use crate::model::*;
use anyhow::{Context, Result, ensure};
use codec::{Action, Change, account as wire};
pub use shep_profile_core as codec;
use uuid::Uuid;

/// `shared_id` must come from durable enrollment/legacy-duplicate review, not a
/// new random UUID on each export. Existing UUID account identities cannot change.
pub fn export_account(account: &Account, shared_id: Uuid) -> Result<Vec<Change>> {
    account.validate()?;
    ensure!(
        !shared_id.is_nil(),
        "Choose a valid shared account identity."
    );
    if let Ok(existing) = Uuid::parse_str(&account.id) {
        ensure!(
            existing == shared_id,
            "The shared account identity changed. Review enrollment again."
        );
    }
    let security = |s| match s {
        ConnectionSecurity::Tls => wire::Security::Tls,
        ConnectionSecurity::StartTls => wire::Security::StartTls,
    };
    let connection = wire::Connection {
        id: shared_id,
        email: account.email.clone(),
        protocol: match account.protocol {
            Protocol::Imap => wire::Protocol::Imap,
            Protocol::Pop3 => wire::Protocol::Pop3,
        },
        host: account.host.clone(),
        port: account.port,
        username: account.username.clone(),
        incoming_security: security(account.incoming_security),
        incoming_auth: match account.incoming_auth {
            IncomingAuth::Password => wire::IncomingAuth::Password,
            IncomingAuth::Plain => wire::IncomingAuth::Plain,
        },
        smtp_host: account.smtp_host.clone(),
        smtp_port: account.smtp_port,
        smtp_username: account.smtp_username().into(),
        smtp_security: security(account.smtp_security()),
        smtp_auth: match account.smtp_auth {
            SmtpAuth::Automatic => wire::SmtpAuth::Automatic,
            SmtpAuth::Plain => wire::SmtpAuth::Plain,
            SmtpAuth::Login => wire::SmtpAuth::Login,
            SmtpAuth::None => wire::SmtpAuth::None,
        },
        smtp_separate_password: account.smtp_separate_password,
        sent_copy: match account.sent_copy {
            SentCopyPolicy::Automatic => wire::SentCopy::Automatic,
            SentCopyPolicy::ServerManaged => wire::SentCopy::ServerManaged,
            SentCopyPolicy::LocalOnly => wire::SentCopy::LocalOnly,
        },
        sent_folder: account.sent_folder.clone(),
        extra: Default::default(),
    };
    connection.validate()?;
    Ok(vec![
        Change {
            action: Action::AccountConnection {
                account: connection,
            },
            extra: Default::default(),
        },
        Change {
            action: Action::AccountName {
                id: shared_id,
                name: account.name.clone(),
            },
            extra: Default::default(),
        },
    ])
}

/// Produce a review candidate only. Unknown connection extensions remain
/// read-only until a client knows how to apply them without a silent downgrade.
pub fn review_account(connection: &wire::Connection, name: &str) -> Result<Account> {
    connection.validate()?;
    ensure!(
        name.len() <= 256 && !name.chars().any(char::is_control),
        "The shared account name is invalid. Review it before applying."
    );
    ensure!(
        connection.extra.is_empty(),
        "This account uses additional connection fields. Update Shep before applying it."
    );
    let security = |s| match s {
        wire::Security::Tls => ConnectionSecurity::Tls,
        wire::Security::StartTls => ConnectionSecurity::StartTls,
    };
    let account = Account {
        id: connection.id.to_string(),
        name: name.into(),
        email: connection.email.clone(),
        protocol: match connection.protocol {
            wire::Protocol::Imap => Protocol::Imap,
            wire::Protocol::Pop3 => Protocol::Pop3,
        },
        host: connection.host.clone(),
        port: connection.port,
        username: connection.username.clone(),
        incoming_security: security(connection.incoming_security),
        incoming_auth: match connection.incoming_auth {
            wire::IncomingAuth::Password => IncomingAuth::Password,
            wire::IncomingAuth::Plain => IncomingAuth::Plain,
        },
        smtp_host: connection.smtp_host.clone(),
        smtp_port: connection.smtp_port,
        smtp_username: connection.smtp_username.clone(),
        smtp_security: Some(security(connection.smtp_security)),
        smtp_auth: match connection.smtp_auth {
            wire::SmtpAuth::Automatic => SmtpAuth::Automatic,
            wire::SmtpAuth::Plain => SmtpAuth::Plain,
            wire::SmtpAuth::Login => SmtpAuth::Login,
            wire::SmtpAuth::None => SmtpAuth::None,
        },
        smtp_separate_password: connection.smtp_separate_password,
        sent_copy: match connection.sent_copy {
            wire::SentCopy::Automatic => SentCopyPolicy::Automatic,
            wire::SentCopy::ServerManaged => SentCopyPolicy::ServerManaged,
            wire::SentCopy::LocalOnly => SentCopyPolicy::LocalOnly,
        },
        sent_folder: connection.sent_folder.clone(),
    };
    account
        .validate()
        .context("The synced account is not valid. Keep the local setup and review it.")?;
    Ok(account)
}
