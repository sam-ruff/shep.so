//! Small, local settings index. Search navigates to the actual editable section.
use super::*;
use iced::{
    Alignment, Length,
    widget::{button, column, row, space, text},
};

pub(super) struct Setting {
    pub title: &'static str,
    pub tab: SettingsTab,
    keywords: &'static str,
}
const SETTINGS: &[Setting] = &[
    Setting {
        title: "Appearance",
        tab: SettingsTab::General,
        keywords: "theme light dark system colors colour",
    },
    Setting {
        title: "Reading and layout",
        tab: SettingsTab::General,
        keywords: "font text size scale zoom unified inbox cross account move conversations replies quotes collapse",
    },
    Setting {
        title: "Mail & performance",
        tab: SettingsTab::General,
        keywords: "sync interval minutes preload background speed",
    },
    Setting {
        title: "Tooltips",
        tab: SettingsTab::General,
        keywords: "tooltip hints primary keyboard shortcut disable icons",
    },
    Setting {
        title: "Your accounts",
        tab: SettingsTab::Accounts,
        keywords: "add account email imap pop3 smtp password server tls ssl authentication connection remove",
    },
    Setting {
        title: "Google connection",
        tab: SettingsTab::Accounts,
        keywords: "google login oauth reconnect disconnect permissions drive",
    },
    Setting {
        title: "Connected calendars",
        tab: SettingsTab::Calendars,
        keywords: "calendar caldav homeserver ical dav add connect remove",
    },
    Setting {
        title: "Backups",
        tab: SettingsTab::Backups,
        keywords: "backup drive destination folder rolling copies retention schedule passphrase password encryption",
    },
    Setting {
        title: "Restore a copy",
        tab: SettingsTab::Backups,
        keywords: "restore backup recovery import",
    },
    Setting {
        title: "Keyboard shortcuts",
        tab: SettingsTab::Shortcuts,
        keywords: "key keys keybind remap primary secondary hotkey archive delete backspace inbox",
    },
    Setting {
        title: "Privacy",
        tab: SettingsTab::Privacy,
        keywords: "images remote block allow contacts sender domain",
    },
    Setting {
        title: "Contacts",
        tab: SettingsTab::Contacts,
        keywords: "contact email address sender addressbook",
    },
];
fn matches(query: &str) -> Vec<&'static Setting> {
    let terms: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut entries: Vec<_> = SETTINGS
        .iter()
        .filter(|setting| {
            let text = format!("{} {}", setting.title, setting.keywords).to_lowercase();
            terms.iter().all(|term| text.contains(term))
        })
        .collect();
    entries.sort_by_key(|setting| {
        !setting
            .title
            .to_lowercase()
            .contains(&query.trim().to_lowercase())
    });
    entries
}
impl App {
    pub(super) fn settings_matches(&self) -> Vec<&'static Setting> {
        matches(&self.settings_search)
    }
    pub(super) fn settings_results(&self) -> Element<'_, Message> {
        let results = self.settings_matches();
        let mut content =
            column![text(format!("{} matching sections", results.len())).size(13)].spacing(10);
        for result in results {
            content = content.push(
                button(
                    row![
                        column![
                            text(result.title).size(14).font(BOLD),
                            muted(format!("{:?}", result.tab)).size(11)
                        ]
                        .spacing(5),
                        space().width(Length::Fill),
                        icon("chevron", 18.)
                    ]
                    .align_y(Alignment::Center),
                )
                .padding(16)
                .width(Length::Fill)
                .style(outline)
                .on_press(Message::FindSetting(result.tab, result.title)),
            );
        }
        content.into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_settings_by_title_and_common_control_names() {
        assert_eq!(matches("font")[0].title, "Reading and layout");
        assert_eq!(matches("TLS")[0].title, "Your accounts");
        assert_eq!(matches("tooltip")[0].title, "Tooltips");
        assert_eq!(matches("secondary")[0].title, "Keyboard shortcuts");
        assert!(matches("no-such-setting").is_empty());
    }
}
