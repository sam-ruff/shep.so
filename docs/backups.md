# Backups and Google

Google is optional. Mail, CalDAV, local, S3 and SFTP backups work without it.

## Make a backup

1. Open **Preferences → Backups**.
2. Choose **Local folder**, **Google Drive**, **S3-compatible storage**, **SFTP** or **FTP / FTPS**.
3. Set how many copies to keep. Compression and passphrase encryption start enabled; choose a passphrase of at least 12 characters.
4. Choose **Back up now**. After your first copy succeeds, enable automatic backups if wanted.

Each destination can disable compression or encryption independently. Unencrypted copies expose mail and account settings to anyone with file access; account passwords are excluded.

For encrypted copies, keep a separate copy of your passphrase. Shep saves it in the OS keychain for scheduled backups. Changing format options requires a successful first copy before automatic or combined backups resume.

Use **Add destination** to keep multiple copies in different places. Each destination
has its own name, schedule, passphrase and number of copies to keep. Select its
row to edit it. Removing a destination keeps its existing backup files.

Choose **Back up all** to save to every checked destination using its own options and saved
passphrase when encrypted. Each needs a successful first copy before joining. Progress and errors
appear separately; **Retry** repeats only that failed destination. **Include**
controls this action independently of automatic schedules.

For S3, enter the HTTPS endpoint, bucket, signing region and folder prefix, then
choose **Test and save connection** with your access key and secret key. Keys stay
in your OS keychain. The test checks read access; your first backup checks upload
permissions. A compatible service must support conditional object writes and
deletes. Existing snapshot limits also apply to S3; versioned buckets may retain
older object versions under their own lifecycle rules.


Backups include downloaded original mail, account and calendar settings, and preferences. Account passwords are optional; Google tokens are never included. Calendar events download again from their providers. Backups are limited to **256 MiB of original mail**.

For SFTP, enter the server, port, username and an existing absolute backup folder.
Choose **Check server fingerprint**, compare it with your server's trusted
settings, then check **I verified this fingerprint** and use it. You can also
paste a SHA256 fingerprint you already verified. Enter your password and choose
**Test and save connection**. A changed host key requires verification again;
previous credentials and automatic-backup readiness are not reused for it.
SFTP currently supports password authentication. Private keys and SSH agents
are not supported yet.

FTP / FTPS defaults to encrypted STARTTLS on port 21; implicit TLS uses port 990.
Enter the server, an existing absolute folder, username and password, then choose
**Test and save connection**. FTPS verifies the server certificate and encrypts
both login and data transfers. Plain FTP is a separate, clearly labelled choice
for servers that require it. The server must support machine-readable listings
(MLSD). Each rolling copy occupies its own folder containing the backup
archive and a small commit record; unrelated folders and files are preserved.

## Restore a copy

Choose a copy in **Backups**. Enter its original passphrase if encrypted; leave it blank for an unencrypted copy. Older Shep backups remain readable. Restore adds missing mail and connections while keeping your current messages, settings and drafts. It does not upload recovered mail to a server.

If password restore fails because the keychain is locked, unlock it and restore the same copy again.

## Move to another computer

1. On the original computer, open **Preferences → Backups → Database transfer**, choose **Export database…** and save the file. Shep saves pending settings/drafts before copying in the background.
2. Transfer the file to the other computer. In the same Preferences section, choose **Import database…**.
3. Review its accounts, mail and any unfinished actions, name the new profile, then import. Your existing profile is kept.
4. Open **Preferences → Accounts → Profiles**, choose **Use on next launch**, then close and reopen Shep.
5. Re-enter account/calendar passwords and reconnect Google in the imported profile.

The SQLite file includes cached original mail, attachments, drafts, accounts, calendars and settings. It is **unencrypted** and excludes OS-keychain passwords and Google sign-in. It has no additional 256 MiB backup limit. Use **Import database…** for this file; **Restore a copy** accepts `.shepbackup` archives.

You can keep reading mail or cancel while copying. Import checks the database before making it available and pauses unfinished provider changes for review. Check their status on the original device/server before retrying. Automatic backups start disabled in the imported profile.

Profiles can be renamed or selected for the next launch in Accounts. This is a one-time transfer; continuous Google account/profile sync is still being implemented.

## Connect Google (optional)

1. Open **Preferences → Accounts** or **Calendars** and find **Google connection**.
2. Choose **Drive backups**, Calendar access, or both.
3. Choose **Sign in with Google**, approve access in your browser, then return to Shep.

You do not need a Google Cloud project or any OAuth credentials. **Cancel sign-in** stops waiting for the browser. If the button says Google sign-in is not configured in this build, that copy of Shep was built without its Google client.

Google sign-in connects Calendar and Drive, not Gmail mail access. Drive copies live in private app storage and do not appear in My Drive.

If you connected with your own Google Cloud client in an earlier version, that connection keeps working. Signing in again switches to Shep's client, and Drive backups made with your own client are no longer listed.

In **Accounts → Profiles and sync**, review shared connection changes before using them. Keep this device’s connection, or add the shared setup and reconnect it. The previous account and its mail stay available.
