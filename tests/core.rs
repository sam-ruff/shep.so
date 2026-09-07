use secrecy::SecretString;
use shep::{
    backup::{self, BackupProvider, LocalBackup, Snapshot},
    model::*,
    shortcuts::{Action, Keymap},
    store::Store,
};

#[test]
fn refresh_secondary_default_migrates_once_without_replacing_user_intent() {
    use shep::shortcuts::Slot;
    let defaults = Keymap::default();
    assert_eq!(defaults.key(Action::Sync), "Mod+R");
    assert_eq!(defaults.binding(Action::Sync, Slot::Secondary), "F5");
    defaults.validate().unwrap();
    for (primary, secondary, expected) in [
        (
            serde_json::json!({"Sync":"Mod+R"}),
            serde_json::json!({}),
            "F5",
        ),
        (
            serde_json::json!({"Sync":"Alt+R"}),
            serde_json::json!({}),
            "F5",
        ),
        (serde_json::json!({"Sync":""}), serde_json::json!({}), ""),
        (
            serde_json::json!({"Sync":"Mod+R"}),
            serde_json::json!({"Sync":""}),
            "",
        ),
        (
            serde_json::json!({"Sync":"Mod+R"}),
            serde_json::json!({"Sync":"F6"}),
            "F6",
        ),
        (
            serde_json::json!({"Sync":"Mod+R", "Move":"F5"}),
            serde_json::json!({}),
            "",
        ),
        (
            serde_json::json!({"Sync":"Mod+R"}),
            serde_json::json!({"Move":"f5"}),
            "",
        ),
    ] {
        let mut keys: Keymap = serde_json::from_value(
            serde_json::json!({"version":2,"primary":primary,"secondary":secondary}),
        )
        .unwrap();
        assert_eq!(keys.binding(Action::Sync, Slot::Secondary), expected);
        keys.validate().unwrap();
        keys.remap_slot(Action::Sync, Slot::Secondary, String::new())
            .unwrap();
        let saved = serde_json::to_value(&keys).unwrap();
        assert_eq!(saved["version"], 2);
        let restored: Keymap = serde_json::from_value(saved).unwrap();
        assert_eq!(restored.binding(Action::Sync, Slot::Secondary), "");
    }
    let mut conflict: Keymap = serde_json::from_value(
        serde_json::json!({"version":2,"primary":{"Sync":"Mod+R", "Move":"F5"},"secondary":{}}),
    )
    .unwrap();
    conflict.remap(Action::Move, "Alt+M".into()).unwrap();
    let restored: Keymap = serde_json::from_value(serde_json::to_value(conflict).unwrap()).unwrap();
    assert_eq!(restored.resolve("F5"), None);
    let legacy: Keymap = serde_json::from_value(serde_json::json!({"Move":"Alt+M"})).unwrap();
    assert_eq!(legacy.binding(Action::Sync, Slot::Secondary), "F5");
    let legacy_disabled: Keymap = serde_json::from_value(serde_json::json!({"Sync":""})).unwrap();
    assert_eq!(legacy_disabled.binding(Action::Sync, Slot::Secondary), "");
    let unsupported = serde_json::json!({"version":4,"primary":{},"secondary":{}});
    assert!(serde_json::from_value::<Keymap>(unsupported).is_err());
}

fn mail(i: usize) -> StoredMail {
    let raw=format!("From: Ada <ada@example.com>\r\nTo: sam@example.com\r\nSubject: Planning café {i}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nA thoughtful message about architecture {i}.").into_bytes();
    let mut m = parse_mail(
        "account",
        &i.to_string(),
        "INBOX",
        raw,
        i.is_multiple_of(2),
        false,
    )
    .unwrap();
    m.summary.timestamp = i as i64;
    m
}

#[tokio::test]
async fn pagination_search_and_flags_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    store.upsert((0..123).map(mail).collect()).await.unwrap();
    let query = MailQuery {
        folder: "INBOX".into(),
        ..Default::default()
    };
    let first = store.query(query.clone()).await.unwrap();
    assert_eq!(first.total, 123);
    assert_eq!(first.rows.len(), PAGE_SIZE);
    assert_eq!(first.rows[0].remote_id, "122");
    let second = store
        .query(MailQuery {
            offset: 50,
            ..query.clone()
        })
        .await
        .unwrap();
    assert!(
        second
            .rows
            .iter()
            .all(|m| !first.rows.iter().any(|f| f.id == m.id))
    );
    let mut selected = first.rows[0].clone();
    selected.starred = true;
    selected.unread = false;
    store.flags(selected.clone()).await.unwrap();
    store
        .move_local(selected.id.clone(), "Projects".into())
        .await
        .unwrap();
    drop(store);
    let reopened = Store::open(path).unwrap();
    let detail = reopened.detail(selected.id).await.unwrap();
    assert!(detail.summary.starred);
    assert!(!detail.summary.unread);
    assert_eq!(detail.summary.folder, "Projects");
    let found = reopened
        .query(MailQuery {
            search: "café 122".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(found.total, 1);
    for query in [
        "\" OR *",
        "architecture\" NEAR(",
        "' ; DROP TABLE messages --",
    ] {
        assert!(
            reopened
                .query(MailQuery {
                    search: query.into(),
                    ..Default::default()
                })
                .await
                .is_ok()
        );
    }
    assert_eq!(
        reopened.query(MailQuery::default()).await.unwrap().total,
        123
    );
}

#[tokio::test]
async fn pop_local_moves_keep_stable_dedup_identity() {
    let store = Store::memory().unwrap();
    let m = mail(4);
    let id = m.summary.id.clone();
    store.upsert(vec![m.clone()]).await.unwrap();
    store
        .move_local(id.clone(), "Archive".into())
        .await
        .unwrap();
    assert!(store.known("account".into()).await.unwrap().contains(&id));
    store.upsert(vec![m]).await.unwrap();
    assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 1);
    assert_eq!(store.detail(id).await.unwrap().summary.folder, "Archive");
}

#[test]
fn remapping_is_persistent_and_conflicts_are_rejected() {
    let mut keys = Keymap::default();
    assert_eq!(keys.resolve("m"), Some(Action::Move));
    assert!(keys.remap(Action::Move, "C".into()).is_err());
    assert_eq!(keys.key(Action::Move), "M");
    keys.remap(Action::Move, "Alt+M".into()).unwrap();
    let keys: Keymap = serde_json::from_str(&serde_json::to_string(&keys).unwrap()).unwrap();
    assert_eq!(keys.resolve("Alt+M"), Some(Action::Move));
    assert_eq!(keys.resolve("M"), None);
    assert!(keys.clone().remap(Action::Move, "Escape".into()).is_err());
}

#[test]
fn mime_decoding_keeps_attachments_and_never_fetches_remote_content() {
    let raw=b"From: a@example.com\r\nSubject: =?UTF-8?B?Q2Fmw6k=?=\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nHello there\r\n--x\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=note.txt\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n--x--\r\n".to_vec();
    let parsed = parse_mail("a", "1", "INBOX", raw.clone(), true, false).unwrap();
    assert_eq!(parsed.summary.subject, "Café");
    assert_eq!(parsed.summary.attachment_count, 1);
    let parsed = mailparse::parse_mail(&raw).unwrap();
    let (body, attachments) = content(&parsed);
    assert!(body.contains("Hello there"));
    assert_eq!(attachments[0].bytes, b"hello");
}

#[test]
fn authenticated_encryption_detects_wrong_password_and_tampering() {
    let secret = SecretString::from("a very strong test passphrase");
    let snapshot = Snapshot {
        version: 1,
        created_at: 1,
        messages: vec![mail(0)],
        accounts: vec![
            serde_json::from_value(serde_json::json!({
                "id": "account", "name": "Test", "email": "sam@example.com", "protocol": "Imap",
                "host": "imap.example.com", "port": 993, "username": "sam@example.com",
                "smtp_host": "smtp.example.com", "smtp_port": 465
            }))
            .unwrap(),
        ],
        calendars: vec![],
        preferences: Preferences::default(),
        credentials: vec![("account".into(), "private-password".into())],
    };
    let bytes = backup::encrypt(&snapshot, &secret).unwrap();
    assert!(!bytes.windows(16).any(|b| b == b"private-password"));
    let restored = backup::decrypt(&bytes, &secret).unwrap();
    assert_eq!(restored.messages.len(), 1);
    assert_eq!(restored.credentials[0].1, "private-password");
    assert!(backup::decrypt(&bytes, &SecretString::from("wrong passphrase")).is_err());
    let mut tampered = bytes.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    assert!(backup::decrypt(&tampered, &secret).is_err());
    let again = backup::encrypt(&snapshot, &secret).unwrap();
    assert_ne!(bytes, again);
    assert!(backup::encrypt(&snapshot, &SecretString::from("short")).is_err());
}

#[tokio::test]
async fn rolling_retention_keeps_newest_and_leaves_unrelated_files() {
    let dir = tempfile::tempdir().unwrap();
    let provider = LocalBackup {
        directory: dir.path().into(),
    };
    tokio::fs::write(dir.path().join("important.txt"), "keep me")
        .await
        .unwrap();
    let name = |i| format!("shep-2026090{i}T120000Z-{}.shepbackup", uuid::Uuid::nil());
    for i in 1..=5 {
        provider.upload(&name(i), vec![i]).await.unwrap();
    }
    assert_eq!(backup::retain(&provider, 2, &name(5)).await.unwrap(), 3);
    let copies = provider.list().await.unwrap();
    assert_eq!(copies.len(), 2);
    assert!(copies[0].name.contains("0905"));
    assert!(dir.path().join("important.txt").exists());
    assert!(backup::retain(&provider, 0, &name(5)).await.is_err());
    assert!(provider.download("../important.txt").await.is_err());
    assert!(provider.delete("important.txt").await.is_err());
}

#[test]
fn caldav_parses_namespaces_escaped_text_timezone_and_recurrence() {
    let source = CalendarSource {
        access: Default::default(),
        id: "home".into(),
        name: "Home".into(),
        kind: CalendarKind::CalDav,
        url: "https://calendar.example/home/".into(),
        username: "sam".into(),
    };
    let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/home/event.ics</d:href><d:propstat><d:prop><d:getetag>"version1"</d:getetag><c:calendar-data>BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:meeting-1
DTSTART;TZID=Europe/London:20260905T100000
DTEND;TZID=Europe/London:20260905T110000
SUMMARY:Coffee\, then design
END:VEVENT
END:VCALENDAR
</c:calendar-data></d:prop></d:propstat></d:response></d:multistatus>"#;
    let events = shep::providers::calendar::parse_caldav(
        xml,
        &source,
        &url::Url::parse(&source.url).unwrap(),
    )
    .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].start.to_rfc3339(), "2026-09-05T09:00:00+00:00");
    assert_eq!(events[0].title, "Coffee, then design");
    assert_eq!(events[0].etag.as_deref(), Some("\"version1\""));
    let recurring = xml.replace(
        "UID:meeting-1",
        "UID:meeting-1\nRECURRENCE-ID:20260905T090000Z",
    );
    let recurring = shep::providers::calendar::parse_caldav(
        &recurring,
        &source,
        &url::Url::parse(&source.url).unwrap(),
    )
    .unwrap();
    assert!(recurring[0].remote_url.is_none());
    let hostile = xml.replace("/home/event.ics", "https://attacker.example/event.ics");
    assert!(
        shep::providers::calendar::parse_caldav(
            &hostile,
            &source,
            &url::Url::parse(&source.url).unwrap()
        )
        .is_err()
    );
}

#[test]
fn ical_folds_unicode_without_splitting_codepoints_or_injecting_properties() {
    let now = chrono::Utc::now();
    let event = CalendarEvent {
        id: "safe".into(),
        source_id: "home".into(),
        title: "é".repeat(120),
        start: now,
        end: now + chrono::Duration::hours(1),
        location: "home\r\nATTENDEE:evil".into(),
        description: "a,b;c\\d".into(),
        all_day: false,
        etag: None,
        remote_url: None,
    };
    let encoded = shep::providers::calendar::encode_ical(&event);
    assert!(encoded.split("\r\n").all(|line| line.len() <= 75));
    assert!(!encoded.contains("\r\nATTENDEE:"));
    let parsed = ical::IcalParser::new(std::io::BufReader::new(encoded.as_bytes()))
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(parsed.events.len(), 1);
}

#[tokio::test]
async fn drafts_and_preferences_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    let prefs = Preferences {
        appearance: Appearance::Dark,
        ..Default::default()
    };
    store.put("preferences", prefs).await.unwrap();
    store
        .save_draft(Draft {
            id: "draft".into(),
            to: "a@example.com".into(),
            body: "Unfinished thought".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    drop(store);
    let store = Store::open(path).unwrap();
    let workspace = store.workspace().await.unwrap();
    assert_eq!(workspace.preferences.appearance, Appearance::Dark);
    assert_eq!(workspace.drafts[0].body, "Unfinished thought");
}

#[tokio::test]
async fn filtering_sorting_and_server_flag_reconciliation() {
    let store = Store::memory().unwrap();
    let mut a = mail(1);
    a.summary.sender = "Zoe".into();
    a.summary.subject = "Alpha".into();
    a.summary.attachment_count = 1;
    let mut b = mail(2);
    b.summary.sender = "ada".into();
    b.summary.subject = "Beta".into();
    let mut sent = mail(3);
    sent.summary.id = "account:Sent:local-sent-1".into();
    sent.summary.folder = "Sent".into();
    store
        .upsert(vec![a.clone(), b.clone(), sent.clone()])
        .await
        .unwrap();
    let query = MailQuery {
        folder: "INBOX".into(),
        ..Default::default()
    };
    assert_eq!(
        store
            .query(MailQuery {
                read_only: true,
                ..query.clone()
            })
            .await
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        store
            .query(MailQuery {
                unread_only: true,
                ..query.clone()
            })
            .await
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        store
            .query(MailQuery {
                attachments_only: true,
                ..query.clone()
            })
            .await
            .unwrap()
            .rows[0]
            .id,
        a.summary.id
    );
    for (sort, first) in [
        (MailSort::Newest, &b),
        (MailSort::Oldest, &a),
        (MailSort::Sender, &b),
        (MailSort::Subject, &a),
    ] {
        assert_eq!(
            store
                .query(MailQuery {
                    sort,
                    ..query.clone()
                })
                .await
                .unwrap()
                .rows[0]
                .id,
            first.summary.id
        );
    }
    store
        .apply_sync(MailSyncItem::Flags(vec![(
            b.summary.id.clone(),
            false,
            true,
        )]))
        .await
        .unwrap();
    let flagged = store
        .query(MailQuery {
            starred_only: true,
            ..query.clone()
        })
        .await
        .unwrap();
    assert_eq!(flagged.total, 1);
    assert!(!flagged.rows[0].unread);
    store
        .apply_sync(MailSyncItem::Reconcile {
            account: "account".into(),
            folder: "INBOX".into(),
            live_ids: [a.summary.id.clone()].into(),
        })
        .await
        .unwrap();
    assert_eq!(store.query(query).await.unwrap().total, 1);
    store
        .apply_sync(MailSyncItem::Reconcile {
            account: "account".into(),
            folder: "Sent".into(),
            live_ids: Default::default(),
        })
        .await
        .unwrap();
    assert!(store.detail(sent.summary.id).await.is_ok());
}

#[tokio::test]
async fn large_readers_are_bounded_and_backup_bytes_are_base64() {
    let store = Store::memory().unwrap();
    let mut m = mail(1);
    m.raw.extend("é".repeat(50000).as_bytes());
    let encoded = serde_json::to_value(&m).unwrap();
    assert!(encoded["raw"].is_string());
    assert_eq!(
        serde_json::from_value::<StoredMail>(encoded).unwrap().raw,
        m.raw
    );
    store.upsert(vec![m.clone()]).await.unwrap();
    let detail = store.detail(m.summary.id).await.unwrap();
    assert!(detail.body_truncated);
    assert!(detail.body.chars().count() <= 32000);
}

#[test]
fn resize_and_sort_preferences_round_trip_with_valid_minimums() {
    let prefs = Preferences {
        reader_split: 0.47,
        mail_sort: MailSort::Oldest,
        ..Default::default()
    };
    let saved: Preferences = serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
    assert_eq!(saved.reader_split, 0.47);
    assert_eq!(saved.mail_sort, MailSort::Oldest);
    assert!(saved.validate().is_ok());
    for ratio in [f32::NAN, 0.0, 1.0] {
        assert!(
            Preferences {
                reader_split: ratio,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}

#[test]
fn fuzzy_folders_rank_exact_prefix_typo_and_nested_matches() {
    use shep::fuzzy::{distance, ranked};
    assert_eq!(distance("archvie", "archive"), 1);
    assert_eq!(
        ranked(
            "archvie",
            ["Work".into(), "Archive".into(), "Projects/Archived".into()]
        )[0],
        "Archive"
    );
    assert_eq!(
        ranked("proj", ["Old Projects".into(), "Projects".into()])[0],
        "Projects"
    );
    assert!(ranked("zxqwv", ["Archive".into()]).is_empty());
}

#[tokio::test]
async fn fuzzy_mail_search_handles_typos_and_treats_query_syntax_as_text() {
    let store = Store::memory().unwrap();
    store.upsert(vec![mail(1)]).await.unwrap();
    for search in ["architecutre", "thoughtfu", "planning café"] {
        let page = store
            .query(MailQuery {
                folder: "INBOX".into(),
                search: search.into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 1, "{search}");
    }
    for search in ["\" OR *", "'; DROP TABLE messages; --", "{evil}", "éèê😊"] {
        let _ = store
            .query(MailQuery {
                folder: "INBOX".into(),
                search: search.into(),
                ..Default::default()
            })
            .await
            .unwrap();
    }
    assert_eq!(
        store
            .query(MailQuery {
                folder: "INBOX".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        1
    );
}

#[test]
fn image_privacy_is_default_deny_and_exceptions_have_exact_scope() {
    use shep::remote_images::{allowed, public_ip};
    let message = mail(1).summary;
    let mut prefs = Preferences::default();
    assert!(!allowed(&prefs, &message));
    prefs.image_messages.push(message.id.clone());
    assert!(allowed(&prefs, &message));
    assert!(!allowed(&prefs, &mail(2).summary));
    prefs.image_messages.clear();
    prefs.image_policy = ImagePolicy::Contacts;
    prefs.contacts.push("ADA@EXAMPLE.COM".into());
    assert!(allowed(&prefs, &message));
    prefs.image_policy = ImagePolicy::BlockAll;
    assert!(!allowed(&prefs, &message));
    prefs.image_domains.push("example.com".into());
    assert!(allowed(&prefs, &message));
    let mut spoof = message.clone();
    spoof.sender = "Ada <ada@evil-example.com>".into();
    assert!(!allowed(&prefs, &spoof));
    for address in [
        "127.0.0.1",
        "10.0.0.1",
        "169.254.169.254",
        "100.64.0.1",
        "192.168.1.1",
        "::1",
        "fc00::1",
        "::ffff:127.0.0.1",
    ] {
        assert!(!public_ip(address.parse().unwrap()), "{address}");
    }
    assert!(public_ip("8.8.8.8".parse().unwrap()));
}

#[test]
fn quoted_history_is_separated_without_losing_content() {
    let (latest, replies) = shep::replies::split(
        "Thanks!\n\nOn Monday, Ada wrote:\n> First question\n> On Sunday, Sam wrote:\n>> Original note",
    );
    assert_eq!(latest, "Thanks!");
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0].body, "First question");
    assert_eq!(replies[1].body, "Original note");
}

#[test]
fn old_accounts_migrate_to_tls_and_correct_smtp_defaults() {
    let mut account: Account = serde_json::from_value(serde_json::json!({"id":"test", "name":"Test", "email":"test@example.com", "protocol":"Imap", "host":"localhost", "port":993, "username":"test", "smtp_host":"localhost", "smtp_port":465})).unwrap();
    assert_eq!(account.incoming_security, ConnectionSecurity::Tls);
    assert_eq!(account.smtp_security(), ConnectionSecurity::Tls);
    account.smtp_port = 587;
    assert_eq!(account.smtp_security(), ConnectionSecurity::StartTls);
    assert_eq!(account.smtp_username(), "test");
}

#[test]
fn full_reader_open_close_shortcuts_are_remappable_and_persist() {
    let mut map = Keymap::default();
    map.remap(Action::OpenMessage, "Alt+Enter".into()).unwrap();
    map.remap(Action::ClosePreview, "Alt+Escape".into())
        .unwrap();
    let restored: Keymap = serde_json::from_str(&serde_json::to_string(&map).unwrap()).unwrap();
    assert_eq!(restored.resolve("Alt+Enter"), Some(Action::OpenMessage));
    assert_eq!(restored.resolve("Alt+Escape"), Some(Action::ClosePreview));
    assert_eq!(restored.resolve("Escape"), None);
    assert!(restored.validate().is_ok());
}

#[tokio::test]
async fn combined_folder_queries_keep_account_scope_filters_and_empty_selection() {
    let store = Store::memory().unwrap();
    let mut messages = Vec::new();
    for (account, folder, count) in [
        ("a", "Projects", 3),
        ("a", "Archive", 2),
        ("b", "Projects", 4),
        ("b", "Sent", 1),
    ] {
        for i in 0..count {
            messages.push(parse_mail(account, &format!("{folder}-{i}"), folder, format!("From: Fixture <f@example.test>\r\nSubject: selection {i}\r\n\r\nCombined folders").into_bytes(), i == 0, i == 0).unwrap());
        }
    }
    store.upsert(messages).await.unwrap();
    let selection = |account: Option<&str>, folder: &str, sent_only| FolderSelection {
        account: account.map(str::to_string),
        folder: folder.into(),
        sent_only,
    };
    let mut query = MailQuery {
        folders: Some(vec![
            selection(Some("a"), "Projects", false),
            selection(Some("b"), "Sent", true),
        ]),
        ..Default::default()
    };
    let page = store.query(query.clone()).await.unwrap();
    assert_eq!((page.total, page.unread), (4, 2));
    assert!(
        !page
            .rows
            .iter()
            .any(|m| m.account_id == "b" && m.folder == "Projects")
    );
    query.starred_only = true;
    assert_eq!(store.query(query.clone()).await.unwrap().total, 2);
    query.starred_only = false;
    query
        .folders
        .as_mut()
        .unwrap()
        .push(selection(None, "Projects", false));
    assert_eq!(store.query(query.clone()).await.unwrap().total, 8); // overlapping scopes never duplicate mail
    query.search = "selection 2".into();
    assert_eq!(store.query(query.clone()).await.unwrap().total, 2);
    query.folders = Some(vec![]);
    assert_eq!(store.query(query).await.unwrap().total, 0);
}

#[test]
fn shortcut_slots_defaults_conflicts_and_legacy_customizations() {
    use shep::shortcuts::Slot;
    let mut keys = Keymap::default();
    assert_eq!(keys.resolve("Mod+D"), Some(Action::Delete));
    assert_eq!(keys.resolve("Backspace"), Some(Action::Archive));
    assert_eq!(keys.resolve("Delete"), Some(Action::Archive));
    assert_eq!(keys.binding(Action::Move, Slot::Secondary), "");
    let before = keys.clone();
    assert!(
        keys.remap_slot(Action::Move, Slot::Secondary, "delete".into())
            .is_err()
    );
    assert_eq!(keys, before);
    keys.remap_slot(Action::Move, Slot::Secondary, "Alt+M".into())
        .unwrap();
    assert!(keys.remap(Action::Reply, "Alt+M".into()).is_err());
    let restored: Keymap = serde_json::from_str(&serde_json::to_string(&keys).unwrap()).unwrap();
    assert_eq!(restored.resolve("Alt+M"), Some(Action::Move));
    assert_eq!(restored.resolve("M"), Some(Action::Move));
    keys.remap_slot(Action::Move, Slot::Secondary, String::new())
        .unwrap();
    assert_eq!(keys.resolve("Alt+M"), None);

    let migrated: Keymap = serde_json::from_str(r#"{"Archive":"E","Move":"Alt+M"}"#).unwrap();
    assert_eq!(migrated.resolve("Backspace"), Some(Action::Archive));
    assert_eq!(migrated.resolve("Delete"), Some(Action::Archive));
    assert_eq!(migrated.resolve("Alt+M"), Some(Action::Move));
    let custom: Keymap =
        serde_json::from_str(r#"{"Archive":"Alt+E","Move":"Delete","Reply":"Mod+D"}"#).unwrap();
    custom.validate().unwrap();
    assert_eq!(custom.resolve("Delete"), Some(Action::Move));
    assert_eq!(custom.resolve("Alt+E"), Some(Action::Archive));
    assert_eq!(custom.resolve("Mod+D"), Some(Action::Reply));
    assert_eq!(custom.key(Action::Delete), "");
    assert_eq!(custom.binding(Action::Archive, Slot::Secondary), "");
}

#[tokio::test]
async fn sidebar_unread_counts_ignore_search_and_folder_scope_and_follow_changes() {
    let store = Store::memory().unwrap();
    let mut originals = Vec::new();
    for (account, id, folder, unread) in [
        ("a", "1", "INBOX", true),
        ("a", "2", "INBOX", true),
        ("b", "3", "INBOX", true),
        ("a", "4", "Archive", true),
        ("a", "5", "INBOX", false),
    ] {
        originals.push(
            parse_mail(
                account,
                id,
                folder,
                b"From: fixture@example.com\r\nSubject: Fixture\r\n\r\nCached mail".to_vec(),
                unread,
                false,
            )
            .unwrap(),
        );
    }
    store.upsert(originals.clone()).await.unwrap();
    let page = store
        .query(MailQuery {
            folder: "Archive".into(),
            search: "no-such-message".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.total, 0);
    assert_eq!(page.inbox_unread.get("a"), Some(&2));
    assert_eq!(page.inbox_unread.get("b"), Some(&1));
    let mut read = originals[0].summary.clone();
    read.unread = false;
    store.flags(read).await.unwrap();
    store
        .move_local(originals[1].summary.id.clone(), "Archive".into())
        .await
        .unwrap();
    let updated = store.query(MailQuery::default()).await.unwrap();
    assert!(!updated.inbox_unread.contains_key("a"));
    assert_eq!(updated.inbox_unread.get("b"), Some(&1));
}

#[test]
fn every_shortcut_slot_can_be_cleared_and_stays_disabled_after_reload() {
    use shep::shortcuts::Slot;
    let mut keys = Keymap::default();
    for action in Action::ALL {
        for slot in [Slot::Primary, Slot::Secondary] {
            keys.remap_slot(action, slot, String::new()).unwrap();
            keys = serde_json::from_str(&serde_json::to_string(&keys).unwrap()).unwrap();
            assert!(keys.binding(action, slot).is_empty());
        }
    }
    assert!(keys.0.values().chain(keys.1.values()).all(String::is_empty));
    keys.remap_slot(Action::Move, Slot::Primary, "M".into())
        .unwrap();
    assert_eq!(keys.resolve("M"), Some(Action::Move));
}
