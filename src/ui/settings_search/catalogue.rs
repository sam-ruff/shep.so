//! Preferences search catalogue. `labels` are captions laid out as text in the
//! section, which a result can reveal. `description` holds the section's other
//! visible sentences plus form fields and options that are not revealable.
use super::{Setting, SettingsTab};

pub(super) const SETTINGS: &[Setting] = &[
    Setting {
        title: "Appearance",
        tab: SettingsTab::General,
        labels: "Light|Dark|System",
        description: "A fresh, quiet workspace. Easy on the eyes. Follow your device.",
        synonyms: "theme dark mode night mode light mode automatic",
    },
    Setting {
        title: "Colors",
        tab: SettingsTab::General,
        labels: "Light palette|Dark palette|Reset palette|Undo changes|Apply colors",
        description: "Choose the colours used throughout Shep and preview their contrast. Customize light and dark palettes separately. Preview your changes, then apply them to the app. Roles: background, surface, border, text, muted text, accent, selection and flag.",
        synonyms: "color colour scheme primary secondary palette custom theme contrast violet hex role",
    },
    Setting {
        title: "System tray",
        tab: SettingsTab::General,
        labels: "Keep Shep running in the system tray when closing the window|Quit Shep",
        description: "Open Shep or quit from the tray menu. A system tray is not currently available on this desktop.",
        synonyms: "minimize minimise to tray close exit menu bar background run saving",
    },
    Setting {
        title: "Reading and layout",
        tab: SettingsTab::General,
        labels: "Message text size|Interface size (%)|Reply history|Show a unified inbox|Allow moving mail between accounts|Group related messages in the reader|Search other accounts' folders when moving",
        description: "Adjust text and interface sizes, previous replies, conversation grouping and folder matches from other accounts when moving. Matches in other IMAP accounts show the account and ask before moving.",
        synonyms: "font scale zoom dpi cross account move conversations replies quotes quoted collapse expanded newest oldest thread threads threading foreign folders confirm",
    },
    Setting {
        title: "Mail & performance",
        tab: SettingsTab::General,
        labels: if crate::desktop_badge::SUPPORTED {
            "Check for new mail|Show unread Inbox count on the dock icon"
        } else {
            "Check for new mail"
        },
        description: "Choose how often to check for new messages. Seconds between background checks. Counts unread Inbox messages across all accounts.",
        synonyms: if crate::desktop_badge::SUPPORTED {
            "sync interval seconds minutes refresh preload background speed poll unread badge dock taskbar launcher"
        } else {
            "sync interval seconds minutes refresh preload background speed poll"
        },
    },
    Setting {
        title: "Notifications",
        tab: SettingsTab::General,
        labels: "Show new-mail popups|Play a sound for new mail|Show sender and subject in popups|Test notification",
        description: "Choose how Shep alerts you when new mail arrives. For new unread messages in your Inbox.",
        synonyms: "notification popup banner toast sound audio alert chime mute silent quiet privacy details",
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
        labels: "Add mail account|Edit account|Reconnect|Connection activity|Refresh connections",
        description: "Use IMAP or POP3 with an app password. Add as many accounts as you need. Review unfinished moves, connection settings and credential cleanup, or remove an account. The account form has name, email address, protocol, incoming server, port, username, password, connection security, authentication, SMTP server, SMTP port, SMTP username, use a different SMTP password, sent copy and sent folder, with test connection and save account.",
        synonyms: "email imap pop3 smtp tls ssl starttls authentication login outgoing incoming host server separate password encrypted unencrypted sent automatic upload provider recovery local copy mailbox credentials",
    },
    Setting {
        title: "Google connection",
        tab: SettingsTab::Accounts,
        labels: "Sign in with Google|Reconnect Google|Disconnect…|Cancel sign-in|Start a new sign-in|Retry Google cleanup|Permissions for the next sign-in|Drive backups · private app data",
        description: "Choose Calendar access, encrypted Drive backups, or both. These choices take effect after sign-in. Your current connection stays available until then. Choose Drive backup or Calendar access to sign in. Calendar access can be no Calendar access, read calendars, or read and edit calendars. Google sign-in is not configured in this build. Your current connection keeps working. Sign-in opens Google in your browser and asks only for the permissions chosen above. Finish signing in with Google in your browser. Connected. Calendar · read & write. Calendar · read only. Calendar · not granted. Drive backup · granted. Drive backup · not granted. Google is disconnected. Unlock your credential store to finish removing its saved login. Disconnected · cached calendars remain available to read.",
        synonyms: "google login oauth reconnect disconnect permissions consent drive calendar read only revoke credential store",
    },
    Setting {
        title: "Profiles",
        tab: SettingsTab::Accounts,
        labels: "Use on next launch|Rename|Save name|Refresh profiles|Import database…",
        description: "Choose which saved workspace opens when you launch Shep. Each keeps its own accounts and settings. Import a database to add another profile. Open now. Next launch. Needs recovery.",
        synonyms: "profile workspace device computer switch rename startup launch local open",
    },
    Setting {
        title: "Profiles and sync",
        tab: SettingsTab::Accounts,
        labels: "Account definitions|Appearance and mail preferences|Enable profile sync on this device|Account passwords|Sync account passwords through your Google account|Sync now|Review shared preferences|Review shared accounts|Discover profiles|Check for shared profiles after Google sign-in|Refresh status",
        description: "Share profiles through your Google account. Share account definitions and portable preferences through Google Drive. Review conflicting changes and choose what this device synchronises. Loading profile choices… Saved profile choices are unavailable. Update Shep or restore a working database. Ready to check for account and preference changes. Profile checked. Changes sync automatically in the background. Profile sync is paused on this device. Changes are saved on this device and waiting to upload. Some shared changes need review. Existing accounts and local changes have been kept. Connect Google with Drive permission to share a profile. Anyone with access to this Google account's Drive app data could read them. Off. Imported accounts ask for their password with Reconnect. Reviews offer keep this device, add as a new account or link to existing account.",
        synonyms: "cloud shared profile google drive device settings accounts sync enrollment synced passwords credentials conflict merge automatic discovery",
    },
    Setting {
        title: "Connected calendars",
        tab: SettingsTab::Calendars,
        labels: "Add CalDAV calendar|Restore removed Google calendars",
        description: "Choose which calendars you use in Shep. Bring your home server calendar into Shep with CalDAV. Enter your server address to find calendars, or use a calendar collection URL. CalDAV · home server. CalDAV · read only. Google Calendar · read only. Google Calendar · offline archive. Remove calendar. The CalDAV form asks for the server URL, username and password, then discovers calendars to save.",
        synonyms: "calendar caldav webdav homeserver ical dav add connect remove url read only writable",
    },
    Setting {
        title: "Backups",
        tab: SettingsTab::Backups,
        labels: "Save to|Include|Add destination|Remove destination|Back up all|Destination name|Local backup folder|Browse…|Connect Google / approve Drive access|Copies to keep (1–100)|Backup interval (hours)|Back up automatically while Shep is running|Compress copies|Encrypt with a passphrase|Include account passwords in the encrypted backup|Backup passphrase|Save backup preferences|Back up now|Recent activity|Hide activity|Refresh activity|Retry|S3 endpoint|Bucket|Signing region|Folder prefix|Use path-style bucket addresses|Access key|Secret key|SFTP server|FTP server|Port|Username|Remote folder|Connection security|Password|Test and save connection|Verified server fingerprint|Check server fingerprint|I verified this fingerprint|Replace verified fingerprint|Use verified fingerprint",
        description: "Save rolling copies to a local folder, Google Drive, S3, SFTP or FTP. Choose where to keep copies. Include selects destinations for Back up all. Manual. Automatic. Needs setup. Working… Test connections and verify SSH server fingerprints before sending passwords. Copy fingerprint. Keep the passphrase for restoring. A successful copy stores it in your OS keychain for this destination. Google tokens are never included. These copies are not encrypted. Anyone with file access can read your mail and account settings. Account passwords are excluded. Finish setup with Back up now. Automatic backups start after a copy with these options is saved. Current snapshot limit: 256 MiB of mail. No successful backup at this destination yet. Enter keys once, then Test and save connection. The test checks read access; the first backup checks upload permissions. The test checks read access. Your first backup checks upload permissions. Passwords are saved in your OS keychain only after a successful test. FTPS · STARTTLS, FTPS · TLS or FTP · unencrypted connection. Plain FTP sends your login and transferred data without connection encryption. Choose FTPS when your server supports it. Backup activity. The latest 20 attempts for this destination, including previous sessions. Loading activity… No backup attempts recorded for this destination yet.",
        synonyms: "backup snapshot compress compression encrypted unencrypted format all include multiple retry progress ssh host fingerprint ftp ftps tls signing bucket endpoint s3 region access key destination folder rolling copies retention schedule passphrase password encryption automatic interval",
    },
    Setting {
        title: "Restore a copy",
        tab: SettingsTab::Backups,
        labels: "Saved copies|Refresh copies",
        description: "Restoring merges messages and accounts into this device. Existing mail is kept. Refresh to see copies at this destination. No saved copies found at this destination. Choose a copy to restore, then enter its passphrase for encrypted copies only and choose Restore & merge. Existing connection settings and passwords are kept; leave the passphrase blank for an unencrypted copy.",
        synonyms: "restore backup recovery recover import decrypt snapshot merge",
    },
    Setting {
        title: "Database transfer",
        tab: SettingsTab::Backups,
        labels: "Export database…|Import database…",
        description: "Move a complete workspace between computers. Includes all cached mail, attachments, drafts, accounts and settings. This file is not encrypted. Account passwords and Google sign-in are not included.",
        synonyms: "database sqlite import export migrate computer transfer all emails drafts accounts settings",
    },
    Setting {
        title: "Keyboard shortcuts",
        tab: SettingsTab::Shortcuts,
        labels: "Primary|Secondary|Disabled|Reset shortcuts",
        description: "Click a binding, then press its replacement. Each × clears the binding beside it. Click Disabled to assign a key.",
        synonyms: "key keys keybind keybinding keymap remap hotkey backspace delete trash new message compose preferences settings clear selection accelerator conflicts",
    },
    Setting {
        title: "Privacy",
        tab: SettingsTab::Privacy,
        labels: "Remote images|Manage contacts|Clear image exceptions",
        description: "Choose whether to load remote images: block all, contacts or allow all. Clear trusted message, sender and domain image exceptions. Loading an external image lets its server see the request. Sender identity in email is not verified by Shep.",
        synonyms: "images remote block allow contacts sender domain tracking pixels external content pictures privacy",
    },
    Setting {
        title: "Contacts",
        tab: SettingsTab::Contacts,
        labels: "Save contacts|Image preferences",
        description: "Add email addresses, separated by commas. These addresses are used by the Contacts image policy.",
        synonyms: "contact email address book addressbook sender trusted allowlist",
    },
    Setting {
        title: "About Shep",
        tab: SettingsTab::General,
        labels: "",
        description: "The installed application version.",
        synonyms: "about release build version",
    },
];

/// Captions owned elsewhere, indexed from their source so they stay in step.
pub(super) fn generated(title: &str) -> Vec<&'static str> {
    match title {
        "Keyboard shortcuts" => crate::shortcuts::Action::ALL
            .iter()
            .map(|action| action.label())
            .collect(),
        _ => Vec::new(),
    }
}
