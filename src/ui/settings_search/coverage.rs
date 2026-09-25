//! Renders every Preferences tab in representative states and checks that each
//! visible caption is searchable, and that each catalogue control can be revealed.
use super::*;
use iced::advanced::{
    Layout, Widget,
    layout::Limits,
    widget::{Id, Operation, Tree, operation::Scrollable},
};
use iced::{Rectangle, Renderer, Size, Vector};
use std::collections::BTreeSet;

/// Captions that carry data, counts or sample content rather than a setting.
/// `#` stands for a number and a trailing `*` for any suffix.
const DYNAMIC: &[(&str, &str)] = &[
    ("# saved contacts", "Saved contact count"),
    (
        "Image exceptions: # messages, # senders, # domains",
        "Image exception counts",
    ),
    ("Review unfinished moves (#)", "Pending move count"),
    ("#–# of #", "Profile page range"),
    ("Previous", "Profile page control"),
    ("Next", "Profile page control"),
    ("Shep #.#.#*", "Installed version number"),
    ("Press a key…", "Shortcut capture prompt"),
    ("Inbox", "Palette preview sample"),
    ("# unread", "Palette preview sample"),
    ("Your next adventure", "Palette preview sample"),
    (
        "A little inspiration for the weekend.",
        "Palette preview sample",
    ),
    ("New message", "Palette preview sample"),
    ("Selected", "Palette preview sample"),
    ("Flagged", "Palette preview sample"),
];

/// Catalogue controls laid out only in states these fixtures do not build.
const UNRENDERED: &[(&str, &str)] = &[
    (
        "Reconnect",
        "An imported account that still needs its password",
    ),
    (
        "Cancel sign-in",
        "While Google sign-in waits for the browser",
    ),
    ("Start a new sign-in", "A staged Google sign-in candidate"),
    ("Save name", "While renaming a profile"),
    ("Retry", "A failed backup run or activity entry"),
    ("I verified this fingerprint", "An SFTP host-key review"),
    ("Replace verified fingerprint", "An SFTP host-key review"),
    ("Use verified fingerprint", "An SFTP host-key review"),
];

/// Key names that appear as shortcut binding values.
const KEYS: &[&str] = &[
    "Ctrl",
    "Shift",
    "Alt",
    "Super",
    "⌘",
    "Backspace",
    "Delete",
    "Enter",
    "Escape",
    "Tab",
    "Space",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Up",
    "Down",
    "Left",
    "Right",
];

/// A shortcut binding value such as `Ctrl+Shift+R`, `F5` or `M`.
fn binding(caption: &str) -> bool {
    caption.split('+').all(|part| {
        part.chars().count() == 1
            || KEYS.contains(&part)
            || part
                .strip_prefix('F')
                .is_some_and(|number| number.parse::<u8>().is_ok())
    })
}

#[derive(Default)]
struct Captions {
    content: Option<Rectangle>,
    found: Vec<String>,
}
impl Operation for Captions {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _: Rectangle,
        content: Rectangle,
        _: Vector,
        _: &mut dyn Scrollable,
    ) {
        if id == Some(&Id::new(reveal::SCROLLER)) {
            self.content = Some(content);
        }
    }
    fn text(&mut self, _: Option<&Id>, bounds: Rectangle, text: &str) {
        let inside = self.content.is_some_and(|content| {
            bounds.y >= content.y && bounds.y + bounds.height <= content.y + content.height
        });
        if inside && !text.trim().is_empty() {
            self.found.push(text.trim().to_owned());
        }
    }
}

fn rendered_captions(app: &App) -> Vec<String> {
    let mut element = app.view();
    let renderer = Renderer::new(iced::Font::DEFAULT, iced::Pixels(16.));
    let widget = element.as_widget_mut();
    let mut tree = Tree::new(&*widget as &dyn Widget<Message, Theme, Renderer>);
    let node = widget.layout(
        &mut tree,
        &renderer,
        &Limits::new(Size::ZERO, Size::new(1440., 920.)),
    );
    let mut operation = Captions::default();
    widget.operate(&mut tree, Layout::new(&node), &renderer, &mut operation);
    assert!(
        operation.content.is_some(),
        "the Preferences scroller must keep its reveal id"
    );
    operation.found
}

/// Replaces each run of digits with `#`, so counts match a `DYNAMIC` template.
fn template(caption: &str) -> String {
    let mut output = String::new();
    for c in crate::fuzzy::normalized(caption).chars() {
        if !c.is_ascii_digit() {
            output.push(c);
        } else if !output.ends_with('#') {
            output.push('#');
        }
    }
    output
}

/// `DYNAMIC` templates use `#` for a number and a trailing `*` for any suffix.
fn dynamic(caption: &str) -> bool {
    let caption = template(caption);
    DYNAMIC.iter().any(|(pattern, _)| {
        let pattern = template(pattern);
        match pattern.strip_suffix('*') {
            Some(prefix) => caption.starts_with(prefix),
            None => caption == pattern,
        }
    })
}

/// Names and addresses echoed from this test's own fixtures are data.
fn fixture_data(caption: &str) -> bool {
    caption.contains("fixture") || caption.contains("example.test")
}

/// Whether `words` appear consecutively, as whole words, in `description`.
fn describes(description: &str, words: &[String]) -> bool {
    let description = split_words(description);
    !words.is_empty()
        && description
            .windows(words.len())
            .any(|window| window == words)
}

fn covered(caption: &str) -> bool {
    let caption_text = crate::fuzzy::normalized(caption);
    if fixture_data(&caption_text) || binding(caption) || dynamic(caption) {
        return true;
    }
    let words = split_words(caption);
    index().iter().any(|entry| {
        entry.title == caption_text
            || entry
                .labels
                .iter()
                .any(|label| label.normalized == caption_text)
            || describes(entry.setting.description, &words)
    })
}

fn account() -> Account {
    serde_json::from_value(serde_json::json!({
        "id":"fixture", "name":"Fixture", "email":"alex@example.test", "protocol":"Imap",
        "host":"mail.example.test", "port":993, "username":"alex",
        "smtp_host":"smtp.example.test", "smtp_port":465
    }))
    .expect("account fixture")
}

fn calendar() -> CalendarSource {
    serde_json::from_value(serde_json::json!({
        "id":"fixture", "name":"Fixture calendar", "kind":"CalDav",
        "url":"https://calendar.example.test/", "username":"alex"
    }))
    .expect("calendar fixture")
}

fn enrollment(selected: bool) -> Arc<crate::profile_sync::enrollment::Snapshot> {
    use crate::profile_sync::enrollment::{Enrollment, Options, Origin, Selection, Snapshot};
    Arc::new(Snapshot {
        enrollment: Enrollment {
            revision: 1,
            options: Options {
                enabled: true,
                ..Default::default()
            },
            selection: selected.then(|| Selection {
                binding: crate::profile_sync::vault::tests::binding(),
                name: "Fixture profile".into(),
                origin: Origin::Create,
                ready: true,
            }),
            last_success: None,
        },
        preferences_revision: 0,
        connections_revision: 0,
        google_revision: 0,
        google_identity: String::new(),
        available: true,
        accounts: 1,
        empty_workspace: false,
    })
}

fn profiles() -> Arc<crate::profiles::Snapshot> {
    use crate::profiles::{Id, Page, Profile, Snapshot};
    let current = Profile {
        id: Id::Legacy,
        name: "Fixture workspace".into(),
        ready: true,
    };
    Arc::new(Snapshot {
        page: Page {
            offset: 0,
            revision: 1,
            active: Id::Legacy,
            total: 51,
            rows: vec![
                current.clone(),
                Profile {
                    id: Id::Imported(uuid::Uuid::from_u128(1)),
                    name: "Fixture copy".into(),
                    ready: false,
                },
            ],
        },
        current,
        warning: None,
    })
}

fn connect(app: &mut App) {
    let mut workspace = (*app.workspace).clone();
    workspace.accounts.push(account());
    workspace.calendars.push(calendar());
    workspace.removed_google_calendars = 1;
    workspace.move_pending_total = 1;
    app.workspace = Arc::new(workspace);
    app.tray.available = true;
    app.google_connected = true;
    app.profiles.snapshot = Some(profiles());
    app.profile_sync.show_snapshot(enrollment(false));
}

fn destinations(app: &mut App) {
    for (id, name) in [("home", "Fixture home"), ("office", "Fixture office")] {
        let destination =
            crate::backup::config::Destination::capture(&app.preferences, id.into(), name.into());
        app.preferences.backup_destinations.push(destination);
    }
    app.backup_activity.open = true;
}

type State = (&'static str, fn(&mut App));

fn states() -> Vec<State> {
    vec![
        ("empty", |_| {}),
        ("connected", connect),
        ("enrolled", |app| {
            connect(app);
            app.profile_sync.show_snapshot(enrollment(true));
        }),
        ("disconnected", |app| {
            app.preferences.google_lifecycle.disconnected = true;
            app.preferences.google_lifecycle.cleanup_pending = true;
        }),
        ("destinations", destinations),
        ("unencrypted", |app| {
            app.preferences.backup_format.protection = crate::backup::format::Protection::None;
        }),
        ("drive", |app| {
            app.preferences.backup_destination = BackupDestination::GoogleDrive
        }),
        ("s3", |app| {
            app.preferences.backup_destination = BackupDestination::S3
        }),
        ("sftp", |app| {
            app.preferences.backup_destination = BackupDestination::Sftp
        }),
        ("ftp", |app| {
            app.preferences.backup_destination = BackupDestination::Ftp
        }),
        ("plain ftp", |app| {
            app.preferences.backup_destination = BackupDestination::Ftp;
            app.preferences.backup_ftp.security = crate::backup::ftp::Security::Plain;
        }),
    ]
}

const TABS: [SettingsTab; 7] = [
    SettingsTab::General,
    SettingsTab::Accounts,
    SettingsTab::Calendars,
    SettingsTab::Backups,
    SettingsTab::Shortcuts,
    SettingsTab::Privacy,
    SettingsTab::Contacts,
];

#[tokio::test]
async fn every_preferences_caption_is_searchable_and_every_control_can_be_revealed() {
    let mut rendered = BTreeSet::new();
    let mut uncovered = std::collections::BTreeMap::new();
    for (name, prepare) in states() {
        for tab in TABS {
            let (mut app, _) = App::new();
            app.tab = Tab::Preferences;
            prepare(&mut app);
            app.settings_tab = tab;
            app.fields.clear();
            app.settings_fields();
            for caption in rendered_captions(&app) {
                if !covered(&caption) {
                    uncovered
                        .entry(caption.clone())
                        .or_insert_with(|| format!("{name}/{tab:?}"));
                }
                rendered.insert(caption);
            }
        }
    }
    let unrendered: Vec<_> = SETTINGS
        .iter()
        .flat_map(captions)
        .filter(|label| !rendered.contains(*label))
        .filter(|label| UNRENDERED.iter().all(|(known, _)| known != label))
        .collect();
    let stale: Vec<_> = UNRENDERED
        .iter()
        .filter(|(label, _)| rendered.contains(*label))
        .collect();
    assert!(
        uncovered.is_empty() && unrendered.is_empty() && stale.is_empty(),
        "Add each new Preferences caption to the search catalogue.\n\
         Uncovered captions: {uncovered:#?}\n\
         Catalogue controls with no rendered caption: {unrendered:#?}\n\
         Unrendered exclusions that now render: {stale:#?}"
    );
}

#[test]
fn a_caption_without_a_catalogue_entry_is_reported() {
    for caption in [
        "Enable the imaginary feature",
        "Include imaginary copies",
        "Ctrl+Imaginary",
        "Shep beta",
    ] {
        assert!(!covered(caption), "{caption}");
    }
    for caption in [
        "Compress copies",
        "Ctrl+Shift+R",
        "F12",
        "M",
        "12 saved contacts",
        "Shep 1.20.3",
        "Current snapshot limit: 256 MiB of mail.",
    ] {
        assert!(covered(caption), "{caption}");
    }
}

#[test]
fn every_exclusion_explains_itself() {
    for (caption, reason) in DYNAMIC.iter().chain(UNRENDERED) {
        assert!(!caption.trim().is_empty() && !reason.trim().is_empty());
    }
}
