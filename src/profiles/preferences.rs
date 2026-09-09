use crate::model::{Appearance, ImagePolicy, Preferences, ReplyDisplay};
use anyhow::{Result, bail};
use serde_json::Value;
use shep_profile_core::SettingKey;
use std::collections::BTreeMap;

/// Explicitly mapped fields supported by the shared metadata format. Mobile-only
/// values remain in original history and never become guessed desktop settings.
pub const SUPPORTED: [SettingKey; 8] = [
    SettingKey::Appearance,
    SettingKey::ReplyDisplay,
    SettingKey::ImagePolicy,
    SettingKey::UnifiedInbox,
    SettingKey::CrossAccountMoves,
    SettingKey::GroupConversations,
    SettingKey::DesktopBadges,
    SettingKey::Tooltips,
];

pub fn export(prefs: &Preferences) -> Result<BTreeMap<SettingKey, Value>> {
    Ok(BTreeMap::from([
        (
            SettingKey::Appearance,
            serde_json::to_value(prefs.appearance)?,
        ),
        (
            SettingKey::ReplyDisplay,
            serde_json::to_value(prefs.reply_display)?,
        ),
        (
            SettingKey::ImagePolicy,
            serde_json::to_value(prefs.image_policy)?,
        ),
        (SettingKey::UnifiedInbox, prefs.unified_inbox.into()),
        (
            SettingKey::CrossAccountMoves,
            prefs.cross_account_moves.into(),
        ),
        (
            SettingKey::GroupConversations,
            prefs.group_conversations.into(),
        ),
        (SettingKey::DesktopBadges, prefs.unread_badge.into()),
        (SettingKey::Tooltips, prefs.tooltips.into()),
    ]))
}

/// Apply one reviewed field. A tombstone resets that field only; device grants,
/// draft ownership, layout and unrelated preferences cannot enter this mapping.
pub fn apply(prefs: &mut Preferences, key: SettingKey, value: Option<&Value>) -> Result<()> {
    fn parsed<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T> {
        serde_json::from_value(value.clone()).map_err(Into::into)
    }
    match key {
        SettingKey::Appearance => {
            prefs.appearance = value.map(parsed).transpose()?.unwrap_or(Appearance::System)
        }
        SettingKey::ReplyDisplay => {
            prefs.reply_display = value
                .map(parsed)
                .transpose()?
                .unwrap_or(ReplyDisplay::Collapsed)
        }
        SettingKey::ImagePolicy => {
            prefs.image_policy = value
                .map(parsed)
                .transpose()?
                .unwrap_or(ImagePolicy::BlockAll)
        }
        SettingKey::UnifiedInbox => {
            prefs.unified_inbox = value.map(parsed).transpose()?.unwrap_or(true)
        }
        SettingKey::CrossAccountMoves => {
            prefs.cross_account_moves = value.map(parsed).transpose()?.unwrap_or(false)
        }
        SettingKey::GroupConversations => {
            prefs.group_conversations = value.map(parsed).transpose()?.unwrap_or(true)
        }
        SettingKey::DesktopBadges => {
            prefs.unread_badge = value.map(parsed).transpose()?.unwrap_or(true)
        }
        SettingKey::Tooltips => prefs.tooltips = value.map(parsed).transpose()?.unwrap_or(true),
        _ => bail!(
            "This preference is not available on desktop yet. Its original value stays in the shared profile."
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn selected_fields_and_resets_keep_device_owned_and_unrelated_values() {
        let mut local = Preferences {
            appearance: Appearance::Dark,
            tooltips: false,
            google_client_id: "fixture-client".into(),
            google_client_secret: "fixture-application-secret".into(),
            google_connection_id: "drive:fixture-owner".into(),
            backup_folder: "/fixture/device-only".into(),
            reader_font_size: 22,
            reader_split: 0.5,
            contacts: vec!["local@example.test".into()],
            ..Default::default()
        };
        let before = local.clone();
        let exported = export(&local).unwrap();
        assert_eq!(exported.len(), SUPPORTED.len());
        let encoded = serde_json::to_string(&exported).unwrap();
        assert!(!encoded.contains("fixture") && !encoded.contains("example.test"));
        apply(&mut local, SettingKey::Appearance, None).unwrap();
        apply(&mut local, SettingKey::UnifiedInbox, Some(&json!(false))).unwrap();
        let expected = Preferences {
            appearance: Appearance::System,
            unified_inbox: false,
            ..before
        };
        assert_eq!(local, expected);
        assert!(apply(&mut local, SettingKey::LeftSwipe, Some(&json!("archive"))).is_err());
        assert!(apply(&mut local, SettingKey::Tooltips, Some(&json!("false"))).is_err());
        assert_eq!(local, expected);
    }
}
