use super::{Setting, SettingsTab};

pub(super) const SETTINGS: &[Setting] = &[
    Setting {
        title: "Appearance",
        tab: SettingsTab::General,
        labels: "Light|Dark|System",
        description: "A fresh, quiet workspace. Easy on the eyes. Follow your device.",
        synonyms: "theme dark mode night mode automatic",
    },
    Setting {
        title: "Colors",
        tab: SettingsTab::General,
        labels: "Light palette|Dark palette|Background|Surface|Border|Text|Muted text|Accent|Selection|Flag|Apply colors|Reset palette",
        description: "Choose the colours used throughout Shep and preview their contrast.",
        synonyms: "color colour primary secondary palette custom theme contrast violet",
    },
    Setting {
        title: "System tray",
        tab: SettingsTab::General,
        labels: "Keep Shep running in the system tray when closing the window|Quit Shep",
        description: "Open Shep or quit from the tray menu.",
        synonyms: "minimize minimise close exit menu bar background saving",
    },
    Setting {
        title: "Reading and layout",
        tab: SettingsTab::General,
        labels: "Message text size|Interface size (%)|Reply history|Show a unified inbox|Allow moving mail between accounts|Group related messages in the reader",
        description: "Adjust text and interface sizes, previous replies and conversation grouping.",
        synonyms: "font scale zoom cross account move conversations replies quotes collapse expanded newest oldest thread",
    },
    Setting {
        title: "Mail & performance",
        tab: SettingsTab::General,
        labels: if crate::desktop_badge::SUPPORTED {
            "Check for new mail|Show unread Inbox count on the dock icon"
        } else {
            "Check for new mail"
        },
        description: "Choose how often to check for new messages. Seconds between background checks.",
        synonyms: if crate::desktop_badge::SUPPORTED {
            "sync interval seconds minutes refresh preload background speed unread badge dock taskbar launcher"
        } else {
            "sync interval seconds minutes refresh preload background speed"
        },
    },
    Setting {
        title: "Notifications",
        tab: SettingsTab::General,
        labels: "Show new-mail popups|Play a sound for new mail|Show sender and subject in popups",
        description: "Choose how Shep alerts you when new mail arrives.",
        synonyms: "notification popup banner sound audio alert privacy details",
    },
    Setting {
        title: "Tooltips",
        tab: SettingsTab::General,
        labels: "Show tooltips on icons|Show primary shortcut in tooltips",
        description: "Show helpful hints beside icon controls.",
        synonyms: "tooltip hints keyboard disable shortcuts",
    },
    Setting {
        title: "Your accounts",
        tab: SettingsTab::Accounts,
        labels: "Add mail account|Edit account|Reconnect|Remove account|Review unfinished moves|Name|Email address|Incoming server|Port|Username|Password|Protocol|Connection security|Authentication|SMTP server|SMTP port|SMTP username|Use a different SMTP password|Sent folder|Sent copy|Test connection|Save account",
        description: "Use IMAP or POP3 with an app password. Add as many accounts as you need. Review connection settings and credential cleanup.",
        synonyms: "email imap pop3 smtp tls ssl starttls authentication login outgoing incoming host server separate password encrypted unencrypted sent automatic upload provider recovery local copy",
    },
    Setting {
        title: "Google connection",
        tab: SettingsTab::Accounts,
        labels: "Sign in with Google|Reconnect Google|Disconnect|Cancel sign-in|Permissions for the next sign-in|Drive backups · private app data|Calendar access|Read only|Read and write|Start a new sign-in|Retry Google cleanup",
        description: "Choose Calendar access, encrypted Drive backups, or both. These choices take effect after sign-in.",
        synonyms: "google login oauth reconnect disconnect permissions consent drive calendar read only revoke credential store",
    },
    Setting {
        title: "Profiles",
        tab: SettingsTab::Accounts,
        labels: "New profile|Rename profile|Switch profile|Open at launch|Import database",
        description: "Choose which saved workspace opens when you launch Shep. Each keeps its own accounts and settings.",
        synonyms: "profile workspace device computer switch rename startup launch local",
    },
    Setting {
        title: "Profiles and sync",
        tab: SettingsTab::Accounts,
        labels: "Account definitions|Appearance and mail preferences|Enable profile sync on this device|Check for shared profiles after Google sign-in|Sync account passwords through your Google account|Sync now|Review shared accounts|Review shared preferences|Keep this device|Add as a new account|Link to existing account",
        description: "Share profiles through your Google account. Review conflicting changes and choose what this device synchronises.",
        synonyms: "cloud shared profile google drive device settings accounts sync enrollment synced passwords credentials conflict merge automatic discovery",
    },
    Setting {
        title: "Connected calendars",
        tab: SettingsTab::Calendars,
        labels: "Add CalDAV calendar|Reconnect calendar|Remove calendar|Restore Google calendars|Server URL|Username|Password|Discover calendars|Save calendars",
        description: "Choose which calendars you use in Shep. Bring your home server calendar into Shep with CalDAV.",
        synonyms: "calendar caldav homeserver ical dav add connect remove url read only writable",
    },
    Setting {
        title: "Backups",
        tab: SettingsTab::Backups,
        labels: "Save to|Destination name|Add destination|Remove destination|Include in Back up all|Back up all|Local backup folder|Browse|Copies to keep (1–100)|Backup interval (hours)|Back up automatically while Shep is running|Compress copies|Encrypt with a passphrase|Include account passwords in the encrypted backup|Backup passphrase|Save backup preferences|Back up now|Recent activity|Retry|S3 endpoint|Bucket|Signing region|Folder prefix|Use path-style bucket addresses|Access key|Secret key|SFTP host|FTP host|Port|Username|Remote folder|Verified server fingerprint|Check server fingerprint|Copy fingerprint|I verified this fingerprint|Replace verified fingerprint|Use verified fingerprint|Password|Test and save connection|Connection security|FTPS · STARTTLS|FTPS · TLS|FTP · unencrypted connection",
        description: "Save rolling copies to a local folder, Google Drive, S3, SFTP or FTP. Test connections and verify SSH server fingerprints before sending passwords. Keep the passphrase for restoring.",
        synonyms: "backup compress compression encrypted unencrypted format all include multiple retry progress ssh host fingerprint ftp ftps tls signing bucket endpoint s3 region access key destination folder rolling copies retention schedule passphrase password encryption automatic interval",
    },
    Setting {
        title: "Restore a copy",
        tab: SettingsTab::Backups,
        labels: "Choose a copy|Restore & merge|Passphrase · encrypted copies only",
        description: "Restore saved mail and accounts while keeping existing connection settings. Leave the passphrase blank for an unencrypted copy.",
        synonyms: "restore backup recovery import decrypt",
    },
    Setting {
        title: "Database transfer",
        tab: SettingsTab::Backups,
        labels: "Export database|Import database|Review import|Import as a new profile|Use existing profile",
        description: "Move a database between computers, including cached messages, drafts, account definitions and preferences.",
        synonyms: "database sqlite import export migrate computer transfer all emails drafts accounts settings",
    },
    Setting {
        title: "Keyboard shortcuts",
        tab: SettingsTab::Shortcuts,
        labels: "Primary|Secondary|Disabled|Reset shortcuts|Archive|Delete|Inbox|Reply|Reply all|Forward|Find in message|Print|Move|Select all|Clear selection|New message|Preferences|Calendar|Search",
        description: "Click a binding, then press its replacement. Clear a binding to disable it.",
        synonyms: "key keys keybind remap hotkey backspace selection accelerator conflicts",
    },
    Setting {
        title: "Privacy",
        tab: SettingsTab::Privacy,
        labels: "Remote images|Block all|Allow contacts|Allow all|Email address|Domain|Add sender|Add domain|Remove exception",
        description: "Choose whether to load remote images and manage trusted sender and domain exceptions.",
        synonyms: "images remote block allow contacts sender domain tracking pictures privacy",
    },
    Setting {
        title: "Contacts",
        tab: SettingsTab::Contacts,
        labels: "Email address|Add contact|Remove contact",
        description: "Manage saved email addresses used by the remote-image policy.",
        synonyms: "contact email address sender addressbook trusted",
    },
    Setting {
        title: "About Shep",
        tab: SettingsTab::General,
        labels: "Shep version",
        description: "The installed application version.",
        synonyms: "about release build version",
    },
];
