use crate::{model::*, store::Store};

pub async fn seed_demo(store: &Store) -> anyhow::Result<()> {
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
    };
    store.save_account(account.clone()).await?;
    store
        .save_account(Account {
            id: "preview-personal".into(),
            name: "Personal".into(),
            email: "alex@example.com".into(),
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
    store.upsert(mails).await?;
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
    if std::env::args().any(|a| a == "--empty-calendars") {
        return Ok(());
    }
    let source = CalendarSource {
        id: "preview-calendar".into(),
        name: "Studio calendar".into(),
        kind: CalendarKind::Google,
        url: String::new(),
        username: String::new(),
    };
    store.save_source(source.clone()).await?;
    let home = CalendarSource {
        id: "preview-home-calendar".into(),
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
    Ok(())
}
