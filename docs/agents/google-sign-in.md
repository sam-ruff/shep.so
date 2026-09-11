# Google sign-in client

The desktop connects Google with a single **Sign in with Google** button in
Preferences → Accounts or Calendars. Shep ships its own installed-app ("Desktop
app") OAuth client, so users never create a Google Cloud project or paste a
client ID and secret. Google describes installed-app client secrets as
[not confidential](https://developers.google.com/identity/protocols/oauth2#installed),
so release builds embed both values.

## How sign-in works

Sign-in opens the system browser at Google's consent page. The redirect goes to a
temporary listener on `http://127.0.0.1:<random port>/callback`, following
Google's [loopback flow for desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app).
Each attempt uses a fresh PKCE S256 verifier and a random 256-bit state that the
callback must match exactly. Waiting for the browser is bounded to three minutes,
and **Cancel sign-in** (or quitting Shep) stops the wait and closes the listener.
Denied, cancelled, timed-out and expired attempts say so and leave the existing
connection unchanged. Shep asks only for the permissions chosen above the button:
`drive.appdata` for Drive backups and profiles, and Calendar event read or
read/write access with the calendar list.

Staged grants, activation, refresh-token rotation, disconnect and the token
owner are unchanged; see [Google and backup contracts](backups.md) and the
Google sections of [AGENTS.md](https://github.com/sam-ruff/shep.so/blob/main/AGENTS.md).

## Giving a build the client

The client is read at compile time in `src/providers/google/client.rs`:

```sh
SHEP_GOOGLE_CLIENT_ID=1234567890-example.apps.googleusercontent.com \
SHEP_GOOGLE_CLIENT_SECRET=GOCSPX-example \
cargo build --release
```

Cargo rebuilds when either variable changes. `scripts/install-linux.sh` and
`scripts/release.py` run `cargo build --release` in the caller's environment, so
export both variables before running them; each prints a warning when they are
missing. The Windows build workflow and the dormant release workflow pass the
repository variable `SHEP_GOOGLE_CLIENT_ID` and the repository secret
`SHEP_GOOGLE_CLIENT_SECRET`. Never commit real values.

A build without both values still works. Preferences shows the button disabled
with "Google sign-in is not configured in this build.", and an existing
connection keeps refreshing.

Debug and `test-support` builds also read the same variables at runtime, which
replaces the built-in client; an empty `SHEP_GOOGLE_CLIENT_ID=` simulates a build
without one. Release builds ignore runtime values. The native harness always
sets a fictional client (`google_client="fixture"`) or an empty one
(`google_client="none"`), never the caller's.

## Connections made with a self-configured client

Earlier builds asked users for their own Desktop OAuth client. Each grant records
the client that issued it, and `google_client_id`/`google_client_secret` stay in
local preferences, no longer shown or editable. Such a connection keeps
refreshing with its own client until the user signs in again; that sign-in uses
Shep's client. Drive app data belongs to a Google Cloud project, so backups and
profile files written through a self-configured project are not listed after
switching. Preferences explains this beside the connected state.

## Google Cloud setup

No real client exists yet. To create Shep's:

1. Create a Google Cloud project for Shep; keep a separate one for testing. The
   Android, iOS and browser clients belong in the same production project (see
   [the profile handover](PROFILE_SYNC_HANDOVER.md)).
2. Enable the **Google Drive API** and the **Google Calendar API**.
3. In Google Auth Platform, configure branding (app name, support email, logo,
   home page, privacy policy and authorised domain) and choose the **External**
   audience.
4. Under Data access, add `https://www.googleapis.com/auth/drive.appdata`,
   `https://www.googleapis.com/auth/calendar.events`,
   `https://www.googleapis.com/auth/calendar.events.readonly` and
   `https://www.googleapis.com/auth/calendar.calendarlist.readonly`.
5. While the app is in Testing, add each tester's Google address as a test user.
   Google expires Testing refresh tokens after seven days, so testers must sign
   in again weekly.
6. Under Clients, create an OAuth client of type **Desktop app**. Loopback
   redirects need no registration. Build with its client ID and secret as above,
   and store them as the repository variable and secret for CI.
7. Before release, publish the app. `drive.appdata` is non-sensitive; the
   Calendar event scopes are sensitive and need Google's app verification.
