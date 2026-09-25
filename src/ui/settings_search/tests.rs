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
        ("other accounts", "Reading and layout"),
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
            matches(query).first().map(|entry| entry.setting.title),
            Some(expected),
            "{query}"
        );
    }
}

#[test]
fn visible_titles_and_controls_outrank_supporting_text_across_tabs() {
    assert_eq!(matches("appearance")[0].setting.title, "Appearance");
    assert!(
        matches("appearance")
            .iter()
            .any(|entry| entry.setting.title == "Profiles and sync")
    );
    assert_eq!(matches("backup interval")[0].setting.title, "Backups");
    assert_eq!(
        matches("check for new mail")[0].setting.title,
        "Mail & performance"
    );
    for setting in SETTINGS {
        assert_eq!(matches(setting.title)[0].setting.title, setting.title);
        for label in captions(setting) {
            assert!(
                matches(label)
                    .iter()
                    .any(|entry| entry.setting.title == setting.title),
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
    assert_eq!(matches("  APPEARÁNCE  ")[0].setting.title, "Appearance");
    let mut matcher = crate::fuzzy::WordMatcher::new("ガ 123 100");
    assert_eq!(matcher.score_normalized("ガ", "カ"), None);
    assert_eq!(matcher.score_normalized("123", "1234"), None);
    assert_eq!(matcher.score_normalized("100", "100"), Some(0));
    let first: Vec<_> = matches("password")
        .iter()
        .map(|entry| entry.setting.title)
        .collect();
    let second: Vec<_> = matches("password")
        .iter()
        .map(|entry| entry.setting.title)
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
        assert_eq!(matches(query)[0].setting.title, section, "{query}");
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
        foreign_move_folders: _,
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
        (
            "foreign_move_folders",
            "Search other accounts' folders when moving",
        ),
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
        ("image_senders", "Clear image exceptions"),
        ("image_domains", "Image exceptions"),
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
        assert_eq!(found("badge"), [("Mail & performance", None)]);
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

fn found(query: &str) -> Vec<(&'static str, Option<&'static str>)> {
    matches(query)
        .iter()
        .map(|entry| (entry.setting.title, entry.control))
        .collect()
}

#[test]
fn real_catalogue_words_are_not_read_as_typos_of_other_words() {
    assert_eq!(
        found("shared profile"),
        [(
            "Profiles and sync",
            Some("Check for shared profiles after Google sign-in")
        )]
    );
    assert_eq!(found("profile workspace"), [("Profiles", None)]);
    assert_eq!(found("database transfer"), [("Database transfer", None)]);
    assert!(
        found("shared")
            .iter()
            .all(|(title, _)| *title != "Profiles")
    );
    assert_eq!(found("apperance")[0].0, "Appearance");
    assert_eq!(found("notifcations")[0].0, "Notifications");
}

#[test]
fn results_name_the_control_that_explains_the_match() {
    for (query, section, control) in [
        ("copies to keep", "Backups", Some("Copies to keep (1–100)")),
        (
            "backup interval",
            "Backups",
            Some("Backup interval (hours)"),
        ),
        ("print", "Keyboard shortcuts", Some("Print message")),
        (
            "clear image exceptions",
            "Privacy",
            Some("Clear image exceptions"),
        ),
        ("smtp username", "Your accounts", None),
        (
            "include account passwords",
            "Backups",
            Some("Include account passwords in the encrypted backup"),
        ),
        (
            "back up auto",
            "Backups",
            Some("Back up automatically while Shep is running"),
        ),
        (
            "shared profile",
            "Profiles and sync",
            Some("Check for shared profiles after Google sign-in"),
        ),
        (
            "new mail interval",
            "Mail & performance",
            Some("Check for new mail"),
        ),
        (
            "account passwords backup",
            "Backups",
            Some("Include account passwords in the encrypted backup"),
        ),
        ("copies keep", "Backups", Some("Copies to keep (1–100)")),
        ("profile workspace", "Profiles", None),
        ("dark mode", "Appearance", Some("Dark")),
        (
            "refresh connections",
            "Your accounts",
            Some("Refresh connections"),
        ),
        ("backups", "Backups", None),
        ("backup", "Backups", None),
        // A typo finds the section but does not name a control.
        ("comprses", "Backups", None),
        ("retention", "Backups", None),
        ("system tray", "System tray", None),
    ] {
        assert_eq!(found(query)[0], (section, control), "{query}");
    }
    let appearance = found("appearance");
    assert_eq!(appearance[0], ("Appearance", None));
    assert!(appearance.contains(&("Profiles and sync", Some("Appearance and mail preferences"))));
}

#[test]
fn every_control_caption_names_itself_unless_its_section_title_does() {
    for setting in SETTINGS {
        for label in captions(setting) {
            let results = found(label);
            let Some((_, control)) = results.iter().find(|(found, _)| *found == setting.title)
            else {
                panic!("{}: {label}", setting.title);
            };
            if split_words(setting.title) != split_words(label) {
                assert_eq!(*control, Some(label), "{}", setting.title);
            }
        }
    }
}

#[test]
fn ranking_is_stable_for_every_caption_and_synonym() {
    let queries: BTreeSet<_> = SETTINGS
        .iter()
        .flat_map(|setting| {
            captions(setting)
                .into_iter()
                .chain(setting.synonyms.split_whitespace())
                .chain([setting.title])
        })
        .collect();
    for query in queries {
        let first = found(query);
        assert!(!first.is_empty(), "{query}");
        assert_eq!(first, found(query), "{query}");
        let titles: BTreeSet<_> = first.iter().map(|(title, _)| title).collect();
        assert_eq!(titles.len(), first.len(), "one result per section: {query}");
    }
}

#[test]
fn common_synonyms_find_their_sections() {
    for (query, expected) in [
        ("keymap", "Keyboard shortcuts"),
        ("hotkey", "Keyboard shortcuts"),
        ("zoom", "Reading and layout"),
        ("threads", "Reading and layout"),
        ("address book", "Contacts"),
        ("tracking pixels", "Privacy"),
        ("minimise to tray", "System tray"),
        ("light mode", "Appearance"),
        ("colour scheme", "Colors"),
        ("mute", "Notifications"),
        ("imap login", "Your accounts"),
        ("mailbox credentials", "Your accounts"),
        ("snapshot", "Backups"),
        ("oauth", "Google connection"),
        ("webdav calendar", "Connected calendars"),
        ("migrate computer", "Database transfer"),
        ("version", "About Shep"),
    ] {
        assert_eq!(
            found(query).first().map(|(title, _)| *title),
            Some(expected),
            "{query}"
        );
    }
}

#[tokio::test]
async fn reveal_results_follow_the_current_request_and_give_up_on_missing_captions() {
    use reveal::Found;
    let (mut app, _) = App::new();
    let _ = app.handle(Message::Tab(Tab::Preferences));
    let control = "Copies to keep (1–100)";
    let _ = app.handle(Message::RevealSetting(
        SettingsTab::Backups,
        "Backups",
        control,
    ));
    let first = app.settings_reveal.expect("pending reveal");
    assert_eq!(first.state, RevealState::Pending);
    assert_eq!(app.settings_group, Some("Backups"));
    assert_eq!(app.settings_tab, SettingsTab::Backups);
    let bounds = iced::Rectangle {
        x: 20.,
        y: 900.,
        width: 200.,
        height: 48.,
    };
    let outline = reveal::Outline {
        content: bounds,
        window: bounds,
    };
    let revealed = Found::Revealed {
        top: 300.,
        focused: true,
        outline,
    };
    let _ = app.handle(Message::SettingRevealed(first.generation + 1, 0, revealed));
    assert_eq!(
        app.settings_reveal.map(|r| r.state),
        Some(RevealState::Pending)
    );
    let _ = app.handle(Message::SettingRevealed(
        first.generation,
        0,
        Found::Missing,
    ));
    assert_eq!(
        app.settings_reveal.map(|r| r.state),
        Some(RevealState::Pending)
    );
    let last = REVEAL_ATTEMPTS - 1;
    let _ = app.handle(Message::SettingRevealed(
        first.generation,
        last,
        Found::Missing,
    ));
    assert_eq!(
        app.settings_reveal.map(|r| r.state),
        Some(RevealState::Missing)
    );

    let _ = app.handle(Message::RevealSetting(
        SettingsTab::Backups,
        "Backups",
        control,
    ));
    let second = app.settings_reveal.expect("second reveal");
    assert!(second.generation > first.generation);
    let _ = app.handle(Message::SettingRevealed(first.generation, 0, revealed));
    assert_eq!(
        app.settings_reveal.map(|r| r.state),
        Some(RevealState::Pending)
    );
    let _ = app.handle(Message::SettingRevealed(second.generation, 0, revealed));
    assert_eq!(
        app.settings_reveal.map(|r| r.state),
        Some(RevealState::Revealed {
            top: 300.,
            focused: true
        })
    );
    assert_eq!(app.settings_reveal.and_then(|r| r.outline), Some(outline));
    assert!(app.settings_outline().is_some());
    // An older timer or dismissal leaves the current outline alone.
    let _ = app.handle(Message::DismissSettingOutline(first.generation));
    assert!(app.settings_outline().is_some());
    let _ = app.handle(Message::DismissSettingOutline(second.generation));
    assert!(app.settings_outline().is_none());
    assert_eq!(
        app.settings_reveal.map(|r| r.state),
        Some(RevealState::Revealed {
            top: 300.,
            focused: true
        }),
        "dismissing the outline keeps the revealed state"
    );

    let _ = app.handle(Message::SettingsSearch("copies".into()));
    assert!(app.settings_reveal.is_none());
    let _ = app.handle(Message::SettingRevealed(second.generation, 0, revealed));
    assert!(app.settings_reveal.is_none());

    let _ = app.handle(Message::RevealSetting(
        SettingsTab::Backups,
        "Backups",
        control,
    ));
    let _ = app.handle(Message::SettingsTab(SettingsTab::General));
    assert!(app.settings_reveal.is_none());
    assert!(app.settings_group.is_none());
}
