# Backups and Google

Google is optional. Mail, CalDAV and local backups work without it.

## Make a backup

1. Open **Preferences → Backups**.
2. Choose **Local folder** or **Google Drive**.
3. Set how many copies to keep and enter a passphrase of at least 12 characters.
4. Choose **Back up now**. After your first copy succeeds, enable automatic backups if wanted.

Keep a separate copy of your passphrase. Shep saves it in the OS keychain for scheduled backups.

Backups include downloaded original mail, account and calendar settings, and preferences. Account passwords are optional; Google tokens are never included. Calendar events download again from their providers. Backups are limited to **256 MiB of original mail**.

## Restore a copy

Choose a copy in **Backups** and enter its passphrase. Restore adds missing mail and connections while keeping your current messages, settings and drafts. It does not upload recovered mail to a server.

If password restore fails because the keychain is locked, unlock it and restore the same copy again.

## Move to another computer

1. On the original computer, open **Preferences → Backups → Database transfer**, choose **Export database…** and save the file. Shep saves pending settings/drafts before copying in the background.
2. Transfer the file to the other computer. In the same Preferences section, choose **Import database…**.
3. Review its accounts, mail and any unfinished actions, name the new profile, then import. Your existing profile is kept.
4. Open **Preferences → Accounts → Profiles**, choose **Use on next launch**, then close and reopen Shep.
5. Re-enter account/calendar passwords and reconnect Google in the imported profile.

The SQLite file includes cached original mail, attachments, drafts, accounts, calendars and settings. It is **unencrypted** and excludes OS-keychain passwords and Google sign-in. It has no additional 256 MiB backup limit. Use **Import database…** for this file; **Restore a copy** accepts encrypted backup archives.

You can keep reading mail or cancel while copying. Import checks the database before making it available and pauses unfinished provider changes for review. Check their status on the original device/server before retrying. Automatic backups start disabled in the imported profile.

Profiles can be renamed or selected for the next launch in Accounts. This is a one-time transfer; continuous Google account/profile sync is still being implemented.

## Connect Google (optional)

Google setup currently requires your own Google Cloud project:

1. Enable the **Drive API** and **Google Calendar API**.
2. Configure the OAuth consent screen; add yourself as a test user when required.
3. Create a **Desktop app** OAuth client.
4. Enter its client ID and secret in Preferences, save, and choose **Connect Google**.

Google sign-in connects Calendar and Drive, not Gmail mail access. Drive copies live in private app storage and do not appear in My Drive. Use the same OAuth application when restoring on another machine.

In **Accounts → Profiles and sync**, review shared connection changes before using them. Keep this device’s connection, or add the shared setup and reconnect it. The previous account and its mail stay available.
