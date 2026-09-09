use crate::{model::*, store::Store};
pub mod backups;
mod bulk_history;
#[path = "html_mail.rs"]
mod html_mail;
mod move_recovery;
mod reading_mail;
pub use move_recovery::recover_move;
pub mod workspace;

pub async fn seed_demo(store: &Store) -> anyhow::Result<()> {
    if store.get::<bool>("fixture_seeded").await? {
        return Ok(());
    }
    seed_demo_contents(store).await?;
    if std::env::args().any(|a| a == "--bulk-history") {
        bulk_history::seed(store).await?;
    }
    move_recovery::seed(store).await?;
    if std::env::args().any(|arg| arg == "--reading-mail") {
        reading_mail::seed(store).await?;
    }
    // These seeded accounts model a completed initial import. Only subsequent
    // fixture sync arrivals exercise notification delivery policy.
    for account in store.get::<Vec<Account>>("accounts").await? {
        let epoch = match account.protocol {
            Protocol::Imap => "imap:1",
            Protocol::Pop3 => "pop3",
        };
        store
            .begin_notification_sync(account.id.clone(), epoch.into())
            .await?;
        store
            .finish_notification_sync(account.id, epoch.into())
            .await?;
    }
    backups::seed(store).await?;
    store.put("fixture_seeded", true).await
}

async fn seed_demo_contents(store: &Store) -> anyhow::Result<()> {
    if std::env::args().any(|a| a == "--profile-empty-workspace")
        && std::env::args().any(|a| a.starts_with("--profile-drive-url="))
    {
        return seed_profile_google(store).await;
    }
    store
        .put(
            "preferences",
            Preferences {
                appearance: Appearance::Light,
                ..Default::default()
            },
        )
        .await?;
    let account = Account {
        id: "preview-work".into(),
        name: "Design studio".into(),
        email: "alex@studio.example".into(),
        protocol: Protocol::Imap,
        host: "imap.example".into(),
        port: 993,
        username: "alex@studio.example".into(),
        smtp_host: "smtp.example".into(),
        smtp_port: 465,
        incoming_security: ConnectionSecurity::Tls,
        incoming_auth: IncomingAuth::Password,
        smtp_security: None,
        smtp_auth: SmtpAuth::Automatic,
        smtp_username: String::new(),
        smtp_separate_password: false,
        sent_copy: Default::default(),
        sent_folder: String::new(),
    };
    store.save_account(account.clone()).await?;
    store
        .save_account(Account {
            id: "preview-personal".into(),
            name: "Personal".into(),
            email: "alex@example.com".into(),
            protocol: if std::env::args().any(|a| a == "--pop3-personal") {
                Protocol::Pop3
            } else {
                Protocol::Imap
            },
            ..account
        })
        .await?;
    let entries = [
        (
            "Maya Chen",
            "A little more room to think",
            "Hey Alex,\n\nI've been thinking about our conversation yesterday. The best tools give us a little more room to think — a little less noise, a little more clarity.\n\nI pulled together the first direction for the studio refresh. Warm neutrals, considered typography, and small details that make everyday things feel good.\n\nA few things I'd love your thoughts on:\n\n• The quieter palette and softer surfaces\n• Making the important actions feel obvious\n• Giving the content a bit more breathing room\n\nNo rush. Take a look when you have a quiet moment, and let's catch up over coffee next week.\n\nThanks,\nMaya\n\nMaya Chen\nDesign lead · Form Studio",
        ),
        (
            "Oliver at Linear",
            "Your weekly workspace digest",
            "A good week of making things. Here's what moved forward in your workspace, and what's coming up next.",
        ),
        (
            "Sophie Williams",
            "Coffee next Thursday?",
            "Found a lovely new spot near the studio. Thursday morning? I'd love to hear how the new project is coming along.",
        ),
        (
            "The Modern House",
            "Spaces for slower living",
            "This week: a light-filled retreat, thoughtful objects, and the people finding a different rhythm.",
        ),
        (
            "Daniel Park",
            "Re: A few thoughts on the prototype",
            "The new direction feels really good. Left a few small notes on spacing and the account switcher.",
        ),
        (
            "Figma",
            "Your files, all in one place",
            "Your September design roundup is here. A few updates to help your next idea take shape.",
        ),
        (
            "Emma Wilson",
            "Weekend plans",
            "The weather is looking lovely. Shall we take the dogs out somewhere new this weekend?",
        ),
        (
            "Read.cv",
            "Good work, from good people",
            "A selection of independent projects, thoughtful portfolios, and interesting people to follow.",
        ),
        (
            "Noah Bennett",
            "Studio invoice · September",
            "Hi Alex, I've attached the September invoice. Everything is looking on track for the next milestone.",
        ),
        (
            "Isabel Martín",
            "Some inspiration for Monday",
            "A handful of things I thought you'd appreciate. Have a restful weekend.",
        ),
    ];
    let mut mails = Vec::new();
    for (i, (sender, subject, body)) in entries.iter().enumerate() {
        let date = chrono::Utc::now() - chrono::Duration::minutes((i * 48) as i64);
        let raw=format!("From: {sender} <hello{}@example.com>\r\nTo: Alex Morgan <alex@studio.example>\r\nSubject: {subject}\r\nDate: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}",i,date.to_rfc2822()).into_bytes();
        mails.push(parse_mail(
            if i == 2 || i == 6 {
                "preview-personal"
            } else {
                "preview-work"
            },
            &format!("1.{i}"),
            "INBOX",
            raw,
            i < 4,
            i == 0 || i == 4,
        )?);
    }
    for i in 10..120 {
        let date = chrono::Utc::now() - chrono::Duration::hours(i as i64);
        let raw=format!("From: Studio archive <archive@example.com>\r\nTo: alex@studio.example\r\nSubject: Project notes {i}\r\nDate: {}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nA saved conversation from an earlier project.",date.to_rfc2822()).into_bytes();
        mails.push(parse_mail(
            "preview-work",
            &format!("1.{i}"),
            "INBOX",
            raw,
            false,
            false,
        )?);
    }
    // Exercise MIME attachments, quoted replies and blocked images through real reader controls.
    let old = &mails[4];
    let raw = format!(
        "From: Daniel Park <hello4@example.com>\r\nTo: alex@studio.example, colleague@example.com\r\nCc: copy@example.com\r\nReply-To: Daniel Park <team@example.com>\r\nMessage-ID: <prototype@example.com>\r\nReferences: <first@example.com>\r\nSubject: Re: A few thoughts on the prototype\r\nDate: {}\r\nContent-Type: multipart/mixed; boundary=parts\r\n\r\n--parts\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>The prototype is ready. Please check the attached notes.</p><p>On Monday, Alex wrote:</p><blockquote><p>Can you send the updated prototype?</p></blockquote><img src=\"https://images.example.com/prototype.webp\" alt=\"Prototype sketch\">\r\n",
        chrono::DateTime::from_timestamp(old.summary.timestamp, 0)
            .unwrap()
            .to_rfc2822()
    );
    let mut raw = raw;
    for name in [
        "prototype-notes.txt",
        "schedule.txt",
        "references.txt",
        "review-checklist.txt",
    ] {
        raw.push_str(&format!("--parts\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=\"{name}\"\r\n\r\nReview material.\r\n"));
    }
    raw.push_str("--parts--\r\n");
    mails[4] = parse_mail(
        "preview-work",
        "1.4",
        "INBOX",
        raw.into_bytes(),
        false,
        true,
    )?;
    if std::env::args().any(|a| a == "--pending-transfer") {
        store
            .put(
                &format!("transfer:{}", mails[0].summary.id),
                Some((
                    "preview-personal".to_owned(),
                    "INBOX".to_owned(),
                    "uploaded".to_owned(),
                )),
            )
            .await?;
    }
    store.upsert(mails).await?;
    if std::env::args().any(|arg| arg == "--html-mail") {
        html_mail::seed(store).await?;
    }
    if std::env::args().any(|arg| arg == "--search-mail") {
        let mut messages = Vec::new();
        for (index, subject, body) in [
            (0, "Quick note", "test".to_owned()),
            (
                1,
                "Project testing plan",
                "Here is the test plan. Other project notes. ".repeat(50),
            ),
            (2, "Camping equipment", "tent".to_owned()),
            (3, "Testing checklist", "testing".to_owned()),
        ] {
            let mut mail = parse_mail("preview-work", &format!("search-{index}"), "INBOX", format!("From: Morgan <morgan@example.test>\r\nTo: alex@studio.example\r\nSubject: {subject}\r\n\r\n{body}").into_bytes(), true, false)?;
            mail.summary.timestamp = chrono::Utc::now().timestamp() + 60 - (3 - index) * 3600;
            messages.push(mail);
        }
        store.upsert(messages).await?;
    }
    if std::env::args().any(|a| a == "--outgoing-mail") {
        seed_outgoing(store).await?;
    }
    if std::env::args().any(|arg| arg == "--conversation-mail") {
        seed_conversations(store).await?;
    }
    for account in ["preview-work", "preview-personal"] {
        store
            .save_folders(
                account.into(),
                vec![
                    "INBOX".into(),
                    "Archive".into(),
                    "Projects".into(),
                    "Sent".into(),
                    "Trash".into(),
                ],
            )
            .await?;
    }
    if std::env::args().any(|a| a == "--long-folders") {
        let folders = vec![
            "Projects".to_string(),
            "Mailspring/Snoozed/Worldwide correspondence and scheduled delivery".to_string(),
            "WWW MMM WWW MMM WWW MMM WWW MMM".to_string(),
            "家族のカレンダーと旅行の計画と写真".to_string(),
        ];
        for (index, folder) in folders.iter().enumerate() {
            let raw = format!("From: Fixture <fixture@example.test>\r\nTo: alex@studio.example\r\nSubject: Folder sample {index}\r\n\r\nSidebar fixture {index}").into_bytes();
            store
                .upsert(vec![parse_mail(
                    "preview-work",
                    &format!("folder-{index}"),
                    folder,
                    raw,
                    true,
                    false,
                )?])
                .await?;
        }
        store.save_folders("preview-work".into(), folders).await?;
    }
    if std::env::args().any(|arg| arg == "--search-mail") {
        store
            .save_folders(
                "preview-work".into(),
                [
                    "INBOX",
                    "Archive",
                    "Projects",
                    "Projects/Archive",
                    "Café",
                    "Sent",
                    "Trash",
                ]
                .map(str::to_owned)
                .to_vec(),
            )
            .await?;
    }
    if std::env::args().any(|arg| arg == "--nested-folders") {
        seed_nested_folders(store).await?;
    }
    if std::env::args().any(|a| a == "--empty-calendars") {
        return Ok(());
    }
    let source = CalendarSource {
        access: Default::default(),
        id: if backups::active() {
            "google:studio@example.test"
        } else {
            "preview-calendar"
        }
        .into(),
        name: "Studio calendar".into(),
        kind: CalendarKind::Google,
        url: if backups::active() {
            "studio@example.test".into()
        } else {
            String::new()
        },
        username: String::new(),
    };
    store.save_source(source.clone()).await?;
    let home = CalendarSource {
        access: if std::env::args().any(|a| a == "--readonly-calendars") {
            CalendarAccess::READ_ONLY
        } else {
            CalendarAccess::default()
        },
        id: if backups::active() {
            "google:home@example.test"
        } else {
            "preview-home-calendar"
        }
        .into(),
        url: if backups::active() {
            "home@example.test".into()
        } else {
            String::new()
        },
        name: "Home calendar".into(),
        ..source.clone()
    };
    store.save_source(home.clone()).await?;
    let day = chrono::Local::now().date_naive();
    let mut events = Vec::new();
    for (i, title) in [
        "A quiet start",
        "Studio catch-up",
        "Coffee with Sophie",
        "Design review",
        "A little time outside",
    ]
    .iter()
    .enumerate()
    {
        let start = (day + chrono::Duration::days(if i == 4 { 0 } else { i as i64 }))
            .and_hms_opt(9 + i as u32, 0, 0)
            .unwrap()
            .and_utc();
        events.push(CalendarEvent {
            // The same remote UID in different calendars must remain independent.
            id: format!("demo-{}", if i == 4 { 0 } else { i }),
            source_id: if i == 4 {
                home.id.clone()
            } else {
                source.id.clone()
            },
            title: (*title).into(),
            start,
            end: start + chrono::Duration::minutes(45),
            location: if i == 2 { "Sunday Coffee" } else { "Studio" }.into(),
            description: String::new(),
            all_day: false,
            etag: None,
            remote_url: None,
        });
    }
    let home_events = events
        .iter()
        .filter(|e| e.source_id == home.id)
        .cloned()
        .collect();
    events.retain(|e| e.source_id == source.id);
    store.replace_events(source.id, events).await?;
    store.replace_events(home.id, home_events).await?;
    let mode =
        std::env::args().find_map(|a| a.strip_prefix("--google-permissions=").map(str::to_owned));
    if let Some(mode) = mode {
        let prefs: Preferences = store.get("preferences").await?;
        let access = GoogleAccess {
            known: true,
            drive: mode == "drive",
            calendar_read: mode != "drive",
            calendar_write: mode == "calendar",
        };
        let sources = if access.calendar_read {
            store.get("calendars").await?
        } else {
            vec![]
        };
        store
            .activate_google(
                prefs.clone(),
                GoogleGrant {
                    id: "fixture-grant".into(),
                    client_id: prefs.google_client_id.clone(),
                    access,
                },
                access.drive.then(|| "drive:fixture".into()),
                sources,
            )
            .await?;
    }
    if std::env::args().any(|a| a == "--invalid-profile-enrollment") {
        store
            .put(
                crate::profile_sync::enrollment::STORAGE_KEY,
                serde_json::json!({"future_format":true}),
            )
            .await?;
    }
    if std::env::args().any(|a| a.starts_with("--profile-drive-url=")) {
        seed_profile_google(store).await?;
    }
    Ok(())
}

async fn seed_conversations(store: &Store) -> anyhow::Result<()> {
    let mut messages = Vec::new();
    for (i, folder, from, to, body) in [
        (
            0,
            "Archive",
            "Maya <maya@example.com>",
            "alex@studio.example",
            "Shall we launch on Monday?",
        ),
        (
            1,
            "Sent",
            "Alex <alex@studio.example>",
            "maya@example.com",
            "Monday works. Here is the schedule.",
        ),
        (
            2,
            "INBOX",
            "Maya <maya@example.com>",
            "alex@studio.example",
            "Confirmed. See you on Monday.",
        ),
    ] {
        let date = chrono::Utc::now() + chrono::Duration::minutes(i);
        let references = if i == 0 {
            String::new()
        } else {
            format!(
                "References: <launch-0@example.com>\r\nIn-Reply-To: <launch-{}@example.com>\r\n",
                i - 1
            )
        };
        let content = if i == 1 {
            format!(
                "Content-Type: multipart/mixed; boundary=launch\r\n\r\n--launch\r\nContent-Type: text/plain\r\n\r\n{body}\r\n--launch\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=\"schedule.txt\"\r\n\r\nMonday at nine.\r\n--launch--\r\n"
            )
        } else {
            format!("Content-Type: text/plain\r\n\r\n{body}")
        };
        let raw = format!(
            "From: {from}\r\nTo: {to}\r\nSubject: Re: Launch schedule\r\nMessage-ID: <launch-{i}@example.com>\r\n{references}Date: {}\r\n{content}",
            date.to_rfc2822()
        );
        messages.push(parse_mail(
            "preview-work",
            &format!("launch-{i}"),
            folder,
            raw.into_bytes(),
            i == 2,
            false,
        )?);
    }
    for i in 0..25 {
        let date = chrono::Utc::now() - chrono::Duration::hours(25 - i);
        let raw = format!(
            "From: Project team <team@example.com>\r\nTo: alex@studio.example\r\nSubject: Long project review\r\nMessage-ID: <long-{i}@example.com>\r\nReferences: <long-root@example.com>\r\nDate: {}\r\n\r\nReview update {i}. Each message stays separate.",
            date.to_rfc2822()
        );
        messages.push(parse_mail(
            "preview-work",
            &format!("long-{i}"),
            if i == 24 { "INBOX" } else { "Projects" },
            raw.into_bytes(),
            false,
            false,
        )?);
    }
    store.upsert(messages).await
}

pub fn discover_calendars(
    url: &str,
    username: &str,
    password: &str,
) -> anyhow::Result<Vec<crate::providers::calendar::discovery::DiscoveredCalendar>> {
    anyhow::ensure!(
        url == "https://calendar.example.test/"
            && username == "alex"
            && password == "fixture-password",
        "Preview connection failed. Use the fixture calendar server and credentials."
    );
    Ok(vec![
        crate::providers::calendar::discovery::DiscoveredCalendar {
            name: "Personal plans".into(),
            url: format!("{url}personal/"),
            access: CalendarAccess::default(),
        },
        crate::providers::calendar::discovery::DiscoveredCalendar {
            name: "Team holidays".into(),
            url: format!("{url}holidays/"),
            access: CalendarAccess::READ_ONLY,
        },
    ])
}

async fn seed_outgoing(store: &Store) -> anyhow::Result<()> {
    use crate::outgoing::{DeliveryState, SentState, Submission};
    let account = store
        .workspace()
        .await?
        .accounts
        .into_iter()
        .find(|a| a.id == "preview-work")
        .unwrap();
    for (index, subject) in ["Delivery needs review", "Sent copy needs review"]
        .into_iter()
        .enumerate()
    {
        let draft = Draft {
            id: format!("preview-outgoing-{index}"),
            account_id: account.id.clone(),
            to: "friend@example.test".into(),
            subject: subject.into(),
            body: "A saved message for the outgoing recovery flow.".into(),
            revision: 1,
            ..Default::default()
        };
        store.save_draft(draft.clone()).await?;
        let mut submission = Submission::new(
            account.clone(),
            &draft,
            crate::compose::build(&account, &draft, vec![])?,
        )?;
        submission.info.created = chrono::Utc::now().timestamp() + (1 - index) as i64;
        let info = store.begin_outgoing(submission, draft).await?;
        if index == 0 {
            store
                .record_delivery(
                    info.attempt,
                    DeliveryState::Uncertain,
                    Some("The connection closed before delivery was acknowledged.".into()),
                )
                .await?;
        } else {
            store
                .record_delivery(info.attempt.clone(), DeliveryState::Accepted, None)
                .await?;
            let saved = store.outgoing_submission(info.attempt.clone()).await?;
            let mail = parse_mail(
                &account.id,
                &info.local_remote_id(),
                "Sent",
                saved.raw,
                false,
                false,
            )?;
            store
                .outgoing_local_sent(info.attempt.clone(), mail)
                .await?;
            store
                .record_sent_copy(
                    info.attempt.clone(),
                    SentState::Appending,
                    Some("Sent".into()),
                    None,
                )
                .await?;
            store
                .record_sent_copy(
                    info.attempt,
                    SentState::Uncertain,
                    Some("Sent".into()),
                    Some(
                        "Delivery succeeded, but the server did not acknowledge the Sent copy."
                            .into(),
                    ),
                )
                .await?;
        }
    }
    Ok(())
}

/// Fault injection is confined to the isolated non-production fixture binary.
pub async fn mail_action_delay() -> anyhow::Result<()> {
    let mode =
        std::env::args().find_map(|arg| arg.strip_prefix("--mail-actions=").map(str::to_owned));
    if let Some(mode) = mode {
        tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
        anyhow::ensure!(mode != "fail", "Fixture server rejected this change.");
    }
    Ok(())
}

/// Forward preparation fails once in the isolated failure fixture, then retries.
pub async fn forward_delay(store: &Store) -> anyhow::Result<()> {
    let mode =
        std::env::args().find_map(|arg| arg.strip_prefix("--mail-actions=").map(str::to_owned));
    if let Some(mode) = mode {
        tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
        if mode == "fail" && !store.get::<bool>("preview-forward-failed").await? {
            store.put("preview-forward-failed", true).await?;
            anyhow::bail!("Fixture storage failure. Try Forward again.");
        }
    }
    Ok(())
}

/// Attachment storage fails once under the same isolated delayed-failure mode.
pub async fn attachment_delay(store: &Store) -> anyhow::Result<()> {
    if std::env::args().any(|arg| arg == "--mail-actions=fail")
        && !store.get::<bool>("preview-attachment-failed").await?
    {
        tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
        store.put("preview-attachment-failed", true).await?;
        anyhow::bail!("Fixture storage failure. Choose the attachment again.");
    }
    Ok(())
}

/// A new arrival proves that an automatic cycle reaches the ordinary cache/UI.
pub async fn sync_mail(
    store: &Store,
) -> anyhow::Result<(u64, Option<crate::notifications::Arrival>)> {
    let background = std::env::args().any(|arg| arg == "--background-sync");
    let fail_once = std::env::args().any(|arg| arg == "--sync-failure-once");
    let round = store.get::<u64>("preview-sync-round").await? + 1;
    store.put("preview-sync-round", round).await?;
    tokio::time::sleep(std::time::Duration::from_millis(if background {
        2500
    } else {
        1500
    }))
    .await;
    anyhow::ensure!(
        !fail_once || round != 1,
        "Fixture mail server is temporarily unavailable. Try Refresh again."
    );
    move_recovery::refresh(store).await?;
    let arrival = if background {
        store.sync_message(parse_mail("preview-work", "1.9000", "INBOX",
            b"From: Morgan <morgan@example.test>\r\nTo: alex@studio.example\r\nSubject: New mail from the background\r\n\r\nThis fictional message arrived through the automatic refresh.".to_vec(), true, false)?).await?
    } else {
        None
    };
    Ok((round, arrival))
}

/// Controlled print delay/retry uses only isolated fixture storage.
pub async fn print_delay(store: &Store) -> anyhow::Result<()> {
    let mode =
        std::env::args().find_map(|arg| arg.strip_prefix("--mail-actions=").map(str::to_owned));
    if let Some(mode) = mode {
        tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
        if mode == "fail" && !store.get::<bool>("preview-print-failed").await? {
            store.put("preview-print-failed", true).await?;
            anyhow::bail!("Fixture storage failure. Try Print again.");
        }
    }
    Ok(())
}

/// Controlled image delivery in isolated demo workspaces only.
pub async fn image_delay() {
    let delay = std::env::var("SHEP_TEST_IMAGE_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        .min(5000);
    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
}

async fn seed_nested_folders(store: &Store) -> anyhow::Result<()> {
    use crate::folders::{Mailbox, NameEncoding};
    for (account, delimiter, folders) in [
        (
            "preview-work",
            '/',
            vec![
                ("INBOX", true),
                ("Archive", true),
                ("Sent", true),
                ("Trash", true),
                ("Projects", true),
                ("Projects/Design", true),
                ("Projects/Design/&ZeVnLIqe-", true),
                ("Projects/Travel", true),
                ("Teams/", false),
                ("Teams/Remote", false),
                ("Teams/Remote/Meetings", true),
                ("Empty container", false),
            ],
        ),
        (
            "preview-personal",
            '.',
            vec![
                ("INBOX", true),
                ("Archive", true),
                ("Sent", true),
                ("Trash", true),
                ("Home", false),
                ("Home.Plans", true),
                ("Home.Plans.2026", true),
                ("Notes/flat.name", true),
            ],
        ),
    ] {
        let catalog = folders
            .into_iter()
            .map(|(name, selectable)| Mailbox {
                name: name.into(),
                delimiter: if name == "Notes/flat.name" {
                    None
                } else {
                    Some(delimiter)
                },
                selectable,
                encoding: NameEncoding::ImapUtf7,
                no_inferiors: false,
                non_existent: false,
            })
            .collect();
        store.save_folder_catalog(account.into(), catalog).await?;
    }
    for (index, (account, folder, subject)) in [
        ("preview-work", "Projects", "Project overview"),
        ("preview-work", "Projects/Design", "Design brief"),
        (
            "preview-work",
            "Projects/Design/&ZeVnLIqe-",
            "Japanese folder note",
        ),
        ("preview-work", "Projects/Travel", "Travel plans"),
        (
            "preview-work",
            "Teams/Remote/Meetings",
            "Remote team agenda",
        ),
        ("preview-personal", "Home.Plans", "Home plans"),
        ("preview-personal", "Home.Plans.2026", "Plans for 2026"),
        ("preview-personal", "Notes/flat.name", "A flat folder"),
    ]
    .into_iter()
    .enumerate()
    {
        store.upsert(vec![parse_mail(account,&format!("nested-{index}"),folder,
            format!("From: Folder fixture <folders@example.test>\r\nSubject: {subject}\r\n\r\nFictional nested folder contents.").into_bytes(),true,false)?]).await?;
    }
    Ok(())
}

/// Desktop failure/latency fixtures never contact a desktop service or audio device.
pub async fn notification_delivery(attempt: u64) -> anyhow::Result<()> {
    let mode = std::env::args().find_map(|arg| {
        arg.strip_prefix("--notification-delivery=")
            .map(str::to_owned)
    });
    if let Some(mode) = mode {
        tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
        anyhow::ensure!(
            mode != "fail-once" || attempt != 1,
            "Fixture notification service unavailable. Check desktop permissions, then try Test notification again."
        );
    }
    Ok(())
}

async fn seed_profile_google(store: &Store) -> anyhow::Result<()> {
    store
        .update_preferences(|p| {
            p.google_client_id = "fixture-profile-client".into();
            p.google_connection_id = "drive:fixture".into();
            p.google_grant = GoogleGrant {
                id: "fixture-profile-grant".into(),
                client_id: "fixture-profile-client".into(),
                access: GoogleAccess {
                    known: true,
                    drive: true,
                    calendar_read: true,
                    calendar_write: true,
                },
            };
        })
        .await?;
    Ok(())
}

/// Fictional host-key review controls; the native fixture never contacts SSH.
pub fn sftp_fingerprint(host: &str) -> anyhow::Result<String> {
    use base64::Engine as _;
    let bytes = match host {
        "backup.example.test" => [1; 32],
        "changed.example.test" => [2; 32],
        _ => anyhow::bail!("The fixture SFTP server is unavailable. Check the server and retry."),
    };
    Ok(format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes)
    ))
}
