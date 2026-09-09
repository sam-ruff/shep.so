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
        title: "Colors",
        tab: SettingsTab::General,
        keywords: "color colour palette primary secondary accent surface background border light dark theme contrast",
    },
    Setting {
        title: "System tray",
        tab: SettingsTab::General,
        keywords: "tray close minimize minimise quit exit menu bar background saving",
    },
    Setting {
        title: "Appearance",
        tab: SettingsTab::General,
        keywords: "theme light dark system",
    },
    Setting {
        title: "Reading and layout",
        tab: SettingsTab::General,
        keywords: "font text size scale zoom unified inbox cross account move conversations replies quotes collapse",
    },
    Setting {
        title: "Mail & performance",
        tab: SettingsTab::General,
        keywords: if crate::desktop_badge::SUPPORTED {
            "sync interval seconds minutes refresh preload background speed unread badge dock taskbar launcher"
        } else {
            "sync interval seconds minutes refresh preload background speed"
        },
    },
    Setting {
        title: "Notifications",
        tab: SettingsTab::General,
        keywords: "notification popup banner sound audio alert new mail sender subject privacy",
    },
    Setting {
        title: "Tooltips",
        tab: SettingsTab::General,
        keywords: "tooltip hints primary keyboard shortcut disable icons",
    },
    Setting {
        title: "Your accounts",
        tab: SettingsTab::Accounts,
        keywords: "add account email imap pop3 smtp password server tls ssl authentication connection remove unfinished moves recovery local copy",
    },
    Setting {
        title: "Google connection",
        tab: SettingsTab::Accounts,
        keywords: "google login oauth reconnect disconnect permissions drive",
    },
    Setting {
        title: "Profiles",
        tab: SettingsTab::Accounts,
        keywords: "profile workspace database import device computer switch rename launch",
    },
    Setting {
        title: "Profiles and sync",
        tab: SettingsTab::Accounts,
        keywords: "cloud shared profile google drive device settings accounts sync enrollment",
    },
    Setting {
        title: "Connected calendars",
        tab: SettingsTab::Calendars,
        keywords: "calendar caldav homeserver ical dav add connect remove",
    },
    Setting {
        title: "Backups",
        tab: SettingsTab::Backups,
        keywords: "backup drive s3 bucket endpoint region access key destination folder rolling copies retention schedule passphrase password encryption",
    },
    Setting {
        title: "Restore a copy",
        tab: SettingsTab::Backups,
        keywords: "restore backup recovery import",
    },
    Setting {
        title: "Database transfer",
        tab: SettingsTab::Backups,
        keywords: "database sqlite import export migrate computer transfer all emails drafts accounts settings",
    },
    Setting {
        title: "Keyboard shortcuts",
        tab: SettingsTab::Shortcuts,
        keywords: "key keys keybind remap primary secondary hotkey archive delete backspace inbox select all selection",
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
        assert!(
            matches("secondary")
                .iter()
                .any(|setting| setting.title == "Keyboard shortcuts")
        );
        assert_eq!(matches("palette")[0].title, "Colors");
        assert_eq!(matches("select all")[0].title, "Keyboard shortcuts");
        assert!(matches("no-such-setting").is_empty());
    }
}
