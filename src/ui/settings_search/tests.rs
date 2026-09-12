use super::*;
use std::collections::BTreeSet;

#[test]
fn actual_labels_synonyms_and_typos_find_their_sections() {
    for (query, expected) in [
        ("dark mode", "Appearance"),
        ("apperance", "Appearance"),
        ("font", "Reading and layout"),
        ("reply history", "Reading and layout"),
        ("group related messages", "Reading and layout"),
        ("mail check interval", "Mail & performance"),
        ("tooltip", "Tooltips"),
        ("system tray", "System tray"),
        ("sound", "Notifications"),
        ("palette", "Colors"),
        ("synced password", "Profiles and sync"),
        ("sftp fingerprint", "Backups"),
        ("S3 region", "Backups"),
        ("FTP security", "Backups"),
        ("path style", "Backups"),
        ("retention", "Backups"),
        ("compression", "Backups"),
        ("encrypt with a passphrase", "Backups"),
        ("SMTP username", "Your accounts"),
        ("sent folder", "Your accounts"),
        ("calendar access", "Google connection"),
        ("caldav", "Connected calendars"),
        ("select all", "Keyboard shortcuts"),
    ] {
        assert_eq!(
            matches(query).first().map(|entry| entry.title),
            Some(expected),
            "{query}"
        );
    }
}

#[test]
fn visible_titles_and_controls_outrank_supporting_text_across_tabs() {
    assert_eq!(matches("appearance")[0].title, "Appearance");
    assert!(
        matches("appearance")
            .iter()
            .any(|entry| entry.title == "Profiles and sync")
    );
    assert_eq!(matches("backup interval")[0].title, "Backups");
    assert_eq!(matches("check for new mail")[0].title, "Mail & performance");
    for setting in SETTINGS {
        assert_eq!(matches(setting.title)[0].title, setting.title);
        for label in setting.labels.split('|') {
            assert!(
                matches(label)
                    .iter()
                    .any(|entry| entry.title == setting.title),
                "{}: {label}",
                setting.title
            );
        }
    }
}

#[test]
fn matching_requires_every_term_and_preserves_unicode_and_numbers() {
    for query in ["", "---", "qzxvjkwp", "appearance qzxvjkwp", "S4 region"] {
        assert!(matches(query).is_empty(), "{query}");
    }
    assert_eq!(matches("  APPEARÁNCE  ")[0].title, "Appearance");
    let mut matcher = crate::fuzzy::WordMatcher::new("ガ 123 100");
    assert_eq!(matcher.score_normalized("ガ", "カ"), None);
    assert_eq!(matcher.score_normalized("123", "1234"), None);
    assert_eq!(matcher.score_normalized("100", "100"), Some(0));
    let first: Vec<_> = matches("password")
        .iter()
        .map(|entry| entry.title)
        .collect();
    let second: Vec<_> = matches("password")
        .iter()
        .map(|entry| entry.title)
        .collect();
    assert_eq!(first, second);
}

#[test]
fn abbreviations_find_their_visible_sections_before_supporting_keywords() {
    for (query, section) in [
        ("prf", "Profiles"),
        ("ntfctns", "Notifications"),
        ("sfp fng", "Backups"),
    ] {
        assert_eq!(matches(query)[0].title, section, "{query}");
    }
}

#[test]
fn persisted_preferences_require_search_coverage_or_an_explicit_exclusion() {
    let Preferences {
        appearance: _,
        palettes: _,
        reader_split: _,
        sidebar_width: _,
        window_size: _,
        mail_sort: _,
        unified_inbox: _,
        collapsed_accounts: _,
        expanded_folders: _,
        collapsed_drafts: _,
        cross_account_moves: _,
        reader_font_size: _,
        interface_scale: _,
        tooltips: _,
        shortcut_tooltips: _,
        unread_badge: _,
        close_to_tray: _,
        notifications: _,
        image_policy: _,
        reply_display: _,
        group_conversations: _,
        contacts: _,
        image_senders: _,
        image_domains: _,
        image_messages: _,
        backup_destinations: _,
        backup_selected: _,
        backup_destination: _,
        backup_s3: _,
        backup_sftp: _,
        backup_ftp: _,
        backup_folder: _,
        backup_format: _,
        backup_copies: _,
        backup_hours: _,
        backup_accounts: _,
        auto_backup: _,
        last_backup: _,
        backup_ready: _,
        google_connection_id: _,
        google_grant: _,
        google_lifecycle: _,
        sync_minutes: _,
        mail_check_seconds: _,
        google_client_id: _,
        google_client_secret: _,
        google_services: _,
        shortcuts: _,
    } = Preferences::default();
    let coverage = [
        ("appearance", "Appearance"),
        ("palettes", "Colors"),
        ("unified_inbox", "Show a unified inbox"),
        ("cross_account_moves", "Allow moving mail between accounts"),
        ("reader_font_size", "Message text size"),
        ("interface_scale", "Interface size"),
        ("tooltips", "Tooltips"),
        ("shortcut_tooltips", "Show primary shortcut in tooltips"),
        ("close_to_tray", "System tray"),
        ("notifications", "Notifications"),
        ("image_policy", "Remote images"),
        ("reply_display", "Reply history"),
        (
            "group_conversations",
            "Group related messages in the reader",
        ),
        ("contacts", "Contacts"),
        ("image_senders", "Add sender"),
        ("image_domains", "Add domain"),
        ("backup_destinations", "Add destination"),
        ("backup_selected", "Backups"),
        ("backup_destination", "Save to"),
        ("backup_s3", "S3 endpoint"),
        ("backup_sftp", "SFTP host"),
        ("backup_ftp", "FTP host"),
        ("backup_folder", "Local backup folder"),
        ("backup_format", "Compress copies"),
        ("backup_copies", "Copies to keep"),
        ("backup_hours", "Backup interval"),
        ("backup_accounts", "Include account passwords"),
        ("auto_backup", "Back up automatically"),
        ("google_services", "Permissions for the next sign-in"),
        ("mail_check_seconds", "Check for new mail"),
        ("shortcuts", "Keyboard shortcuts"),
    ];
    let excluded = [
        ("reader_split", "Changed by dragging the reader divider"),
        ("sidebar_width", "Changed by dragging the sidebar divider"),
        ("window_size", "Saved native window geometry"),
        ("mail_sort", "Changed by the mailbox sort control"),
        ("collapsed_accounts", "Saved sidebar expansion state"),
        ("expanded_folders", "Saved folder expansion state"),
        ("collapsed_drafts", "Saved drafts expansion state"),
        (
            "image_messages",
            "Per-message image permission from the reader",
        ),
        ("last_backup", "Provider receipt timestamp"),
        ("backup_ready", "Provider setup receipt"),
        (
            "google_connection_id",
            "Credential slot identity, never a search value",
        ),
        ("google_grant", "Authenticated consent receipt"),
        ("google_lifecycle", "Disconnection and cleanup journal"),
        ("google_client_id", "Legacy OAuth compatibility data"),
        (
            "google_client_secret",
            "Legacy OAuth secret, never a search value",
        ),
        (
            "sync_minutes",
            "Legacy calendar/background interval without a Preferences control",
        ),
    ];
    for (field, query) in coverage {
        assert!(!matches(query).is_empty(), "{field}: {query}");
    }
    for (_, reason) in excluded {
        assert!(!reason.is_empty());
    }
    let mut reviewed: BTreeSet<_> = coverage
        .iter()
        .chain(excluded.iter())
        .map(|(field, _)| *field)
        .collect();
    reviewed.insert("unread_badge");
    if crate::desktop_badge::SUPPORTED {
        assert_eq!(matches("badge")[0].title, "Mail & performance");
    }
    let value = serde_json::to_value(Preferences::default()).expect("preferences fixture");
    let actual: BTreeSet<_> = value
        .as_object()
        .expect("preferences object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        actual, reviewed,
        "Review each new persisted field's searchable control or document its internal-state exclusion"
    );
}

fn keys(value: &impl serde::Serialize, expected: &[&str]) {
    let value = serde_json::to_value(value).expect("configuration fixture");
    let actual: BTreeSet<_> = value
        .as_object()
        .expect("configuration object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        actual,
        expected.iter().copied().collect(),
        "Audit new dynamic control captions in the catalogue"
    );
}

#[test]
fn nested_network_and_dynamic_configuration_fields_require_a_catalogue_review() {
    keys(
        &crate::backup::s3::Settings::default(),
        &["endpoint", "region", "bucket", "prefix", "path_style"],
    );
    keys(
        &crate::backup::sftp::Settings::default(),
        &["host", "port", "username", "directory", "fingerprint"],
    );
    keys(
        &crate::backup::ftp::Settings::default(),
        &["host", "port", "username", "directory", "security"],
    );
    keys(
        &crate::backup::format::Options::default(),
        &["compression", "protection"],
    );
    keys(
        &crate::notifications::Settings::default(),
        &["popups", "sound", "show_details"],
    );
    keys(&GoogleServices::default(), &["drive", "calendar"]);
    keys(
        &crate::profile_sync::enrollment::Options::default(),
        &[
            "enabled",
            "accounts",
            "settings",
            "discover_on_login",
            "passwords",
        ],
    );
    keys(
        &crate::backup::config::Destination::capture(
            &Preferences::default(),
            "fixture".into(),
            "Fixture".into(),
        ),
        &[
            "id",
            "name",
            "included",
            "destination",
            "folder",
            "s3",
            "sftp",
            "ftp",
            "format",
            "copies",
            "hours",
            "accounts",
            "automatic",
            "last_backup",
            "ready",
        ],
    );
    let account: Account = serde_json::from_value(serde_json::json!({
        "id":"fixture", "name":"Fixture", "email":"alex@example.test", "protocol":"Imap",
        "host":"mail.example.test", "port":993, "username":"alex", "smtp_host":"smtp.example.test", "smtp_port":465
    })).expect("account fixture");
    keys(
        &account,
        &[
            "id",
            "name",
            "email",
            "protocol",
            "host",
            "port",
            "username",
            "smtp_host",
            "smtp_port",
            "incoming_security",
            "incoming_auth",
            "smtp_security",
            "smtp_auth",
            "smtp_username",
            "smtp_separate_password",
            "sent_copy",
            "sent_folder",
        ],
    );
    let calendar: CalendarSource = serde_json::from_value(serde_json::json!({
        "id":"fixture", "name":"Fixture", "kind":"CalDav", "url":"https://calendar.example.test/", "username":"alex"
    })).expect("calendar fixture");
    keys(
        &calendar,
        &["id", "name", "kind", "url", "username", "access"],
    );
}
