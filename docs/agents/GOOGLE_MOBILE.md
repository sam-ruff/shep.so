# Mobile Google connection

Flutter uses the maintained Android/iOS Google Sign-In packages through their pinned platform interface. This avoids registering the combined plugin’s web SDK in the offline Flutter preview. The separate hosted browser client retains its own beta login. Preferences saves the next sign-in's
Drive/Calendar choices separately from committed access. The SDK owns tokens;
Shep stores only the application binding, subject, email, optional verified Drive principal, selected services and
cleanup state in device secure storage. No access/refresh token is entered in
Preferences or uploaded to the beta server.

[Profile discovery](PROFILE_MOBILE.md) now uses this saved connection and silent
token API, with verified principal persistence and lifecycle fencing. Calendar sync,
Drive backups, enrollment and continuous profiles remain unfinished. Those integrations, automatic session restoration, seamless
account switching and live Android/Apple verification remain in TODO.

## Build configuration

Register each platform client in the same intended Shep Google Cloud project.
Keep the production and preview application registrations separate. Android
registration needs the matching package name and signing certificate fingerprint;
the Flutter SDK also needs the project's Web OAuth client ID. No client secret
belongs in the mobile app. Follow the [Android SDK configuration](https://pub.dev/packages/google_sign_in_android).

```sh
flutter build apk --flavor production \
  --dart-define=SHEP_GOOGLE_SERVER_CLIENT_ID=YOUR_WEB_CLIENT_ID
```

For iOS, also provide `SHEP_GOOGLE_IOS_CLIENT_ID` through `--dart-define`. Put
`SHEP_GOOGLE_IOS_URL_SCHEME = YOUR_REVERSED_CLIENT_ID` in ignored
`flutter/ios/Flutter/Google.local.xcconfig`. Debug/Release configurations include
it and `Info.plist` registers the URL scheme. The tracked fallback is deliberately
unconfigured. See the [iOS SDK setup](https://pub.dev/packages/google_sign_in_ios).
Actual Xcode build/callback execution still requires a Mac and registered clients.

## Lifecycle and verification

`GoogleConnection` owns one SDK operation and one coalescing settings writer.
Changed choices reject held authorization; locked/invalid saved state cannot be
replaced with defaults. Definite failed commits retain the old connection. If a write and its verification read both fail, Google operations pause until the saved record is read again; newer unsaved choices survive that recovery. Disconnection
is durable before local SDK sign-out, with a persisted cleanup retry. It never
calls project-wide revocation. Reconnect authorizes the committed subject; it
does not sign that account out to open an account picker.

The SDK supports only one current account on some platforms. Until safe switching
is implemented, selecting another account requires explicit local disconnection.
Background token requests do not invoke potentially interactive lightweight
authentication; reconnect restores the session explicitly. These are current
limits, not completed continuous-sync behavior. See the [SDK authorization API](https://pub.dev/packages/google_sign_in).

Controller, SDK-boundary and secure-store lost-reply tests use synthetic data.
Shared widget/Android scenarios and Flutter Playwright/Appium controls exercise
saved permissions, cancellation, retry and local cleanup. Preview SDK fixtures
live only under `flutter/test/`; the production entry uses the native SDK and
refuses an unconfigured build. See [client testing](../CLIENT_TESTING.md) for the
commands and [the profile handover](PROFILE_SYNC_HANDOVER.md) for remaining work.
