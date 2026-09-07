# Shep mobile

Android/iOS client in the Shep monorepo, with the same quiet violet theme and configurable K-9-style message swipes. This is a **development client**. A native Rust bridge provides mail account setup, device credentials, SQLite cache/drafts and mail transport. Full provider recovery, calendar, Google and backup parity remain in the [parity checklist](../docs/CLIENT_PARITY.md).

```sh
flutter pub get
flutter run --flavor preview --target test/preview_main.dart
flutter test
```

Android requires API 24 or later. iOS targets 14.0 so the system reader can display the shared WebP inline images; Apple simulator verification remains open.

Choose an isolated Android emulator. Preview data is fictional and sending is refused. Production uses `--flavor production` and `lib/main.dart`, starts empty, and contains no fixture import.

The separate desktop-style browser app is in `../web/`. A Flutter browser build exists for Playwright testing of the mobile controls:

```sh
flutter build web --target test/preview_main.dart --no-web-resources-cdn
```

See [client testing](../docs/CLIENT_TESTING.md) for Playwright, Appium and Apple simulator commands. Keep quality/release CI disabled.
