//! Explicit account metadata mappings shared by desktop and Flutter. Callers
//! retain the operation envelope; these candidates never activate credentials.
//! Account adapter baseline: shared/mail-core at published client 9289f53.
use crate::model::*;
use anyhow::{Context, Result, ensure};
use codec::{Action, Change, account as wire};
pub use shep_profile_core as codec;
use uuid::Uuid;

pub const SETTINGS: &[codec::SettingKey] = &[
    codec::SettingKey::Appearance,
    codec::SettingKey::ReplyDisplay,
    codec::SettingKey::ImagePolicy,
    codec::SettingKey::UnifiedInbox,
    codec::SettingKey::CrossAccountMoves,
    codec::SettingKey::GroupConversations,
    codec::SettingKey::DesktopBadges,
    codec::SettingKey::Tooltips,
];

/// Export only fields the native client actually implements. In particular,
/// OAuth grants, backup destinations and local window geometry cannot leak into
/// metadata through a whole-Preferences serialization.
pub fn setting_value(
    key: codec::SettingKey,
    preferences: &Preferences,
) -> Option<serde_json::Value> {
    use codec::SettingKey::*;
    Some(match key {
        Appearance => serde_json::json!(preferences.appearance),
        ReplyDisplay => serde_json::json!(preferences.reply_display),
        ImagePolicy => serde_json::json!(preferences.image_policy),
        UnifiedInbox => serde_json::json!(preferences.unified_inbox),
        CrossAccountMoves => serde_json::json!(preferences.cross_account_moves),
        GroupConversations => serde_json::json!(preferences.group_conversations),
        DesktopBadges => serde_json::json!(preferences.unread_badge),
        Tooltips => serde_json::json!(preferences.tooltips),
        PreviewLines | LeftSwipe | RightSwipe | SenderPictures => return None,
    })
}

/// Apply one already-merged setting to a private candidate. The store validates
/// and commits the entire candidate atomically; failed batches never half-apply.
/// Optional extensions stay in shared history. Unsupported local display fields
/// have no local override and remain available to other clients unchanged.
pub fn apply_setting(preferences: &mut Preferences, change: &Change) -> Result<bool> {
    let (key, value) = match &change.action {
        Action::Setting { key, value } => (*key, value.clone()),
        Action::SettingRemoved { key } => {
            let Some(value) = setting_value(*key, &Preferences::default()) else {
                return Ok(false);
            };
            (*key, value)
        }
        _ => anyhow::bail!("The profile update is not a portable setting."),
    };
    let before = setting_value(key, preferences);
    use codec::SettingKey::*;
    match key {
        Appearance => preferences.appearance = serde_json::from_value(value)?,
        ReplyDisplay => preferences.reply_display = serde_json::from_value(value)?,
        ImagePolicy => preferences.image_policy = serde_json::from_value(value)?,
        UnifiedInbox => preferences.unified_inbox = serde_json::from_value(value)?,
        CrossAccountMoves => preferences.cross_account_moves = serde_json::from_value(value)?,
        GroupConversations => preferences.group_conversations = serde_json::from_value(value)?,
        DesktopBadges => preferences.unread_badge = serde_json::from_value(value)?,
        Tooltips => preferences.tooltips = serde_json::from_value(value)?,
        LeftSwipe | RightSwipe => {
            ensure!(
                value.as_str().is_some_and(|s| [
                    "none", "archive", "trash", "read", "star", "select", "move", "spam"
                ]
                .contains(&s)),
                "The shared swipe setting is invalid."
            );
            return Ok(false);
        }
        SenderPictures => {
            ensure!(
                value.is_boolean(),
                "The shared sender-picture setting is invalid."
            );
            return Ok(false);
        }
        PreviewLines => {
            ensure!(
                value.as_u64().is_some_and(|v| v <= 4),
                "The shared preview-line setting is invalid."
            );
            return Ok(false);
        }
    }
    Ok(before != setting_value(key, preferences))
}

/// `shared_id` must come from durable enrollment/legacy-duplicate review, not a
/// new random UUID on each export. Existing UUID account identities cannot change.
pub fn export_account(account: &Account, shared_id: Uuid) -> Result<Vec<Change>> {
    account.validate()?;
    ensure!(
        account.name.len() <= 256 && !account.name.chars().any(char::is_control),
        "Use an account name of at most 256 bytes without control characters before sharing it."
    );
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
