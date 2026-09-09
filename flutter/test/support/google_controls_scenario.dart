import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import '../workspace_test.dart' show MemorySettings;
import 'google_fixture.dart';
import 'preview_repository.dart';

Future<void> _show(WidgetTester tester, Finder finder) async {
  if (finder.evaluate().isEmpty) {
    await tester.scrollUntilVisible(
      finder,
      300,
      scrollable: find
          .descendant(
            of: find.byKey(const ValueKey('preferences-list')),
            matching: find.byType(Scrollable),
          )
          .first,
    );
  }
  await tester.ensureVisible(finder);
  await tester.pumpAndSettle();
}

Future<void> _tap(WidgetTester tester, Finder finder) async {
  await _show(tester, finder);
  await tester.tap(finder);
  await tester.pumpAndSettle();
}

Future<void> _calendar(
  WidgetTester tester,
  GoogleCalendarPermission current,
  String choice,
) async {
  await _tap(tester, find.byKey(ValueKey('google-calendar-${current.name}')));
  await _tap(tester, find.text(choice).last);
}

Future<void> googleConsentControls(
  WidgetTester tester, {
  Future<void> Function(String)? capture,
}) async {
  final store = MemoryGoogleStore();
  final sdk = FixtureGoogleAuthorization()..failEditingOnce = true;
  final c = GoogleConnection(sdk, store);
  final w = Workspace(PreviewRepository(), MemorySettings(), google: c);
  addTearDown(w.dispose);
  await w.initialize();
  await tester.pumpWidget(ShepApp(workspace: w));
  await tester.pumpAndSettle();
  await _tap(tester, find.text('Preferences'));
  await _tap(tester, find.byKey(const ValueKey('google-drive')));
  await _calendar(tester, GoogleCalendarPermission.off, 'Read calendars');
  await _tap(tester, find.text('Sign in with Google'));
  expect(find.text(FixtureGoogleAuthorization.email), findsOneWidget);
  expect(
    c.active!.permissions,
    const GooglePermissions(
      drive: true,
      calendar: GoogleCalendarPermission.read,
    ),
  );
  await capture?.call('google-mobile-consent-light');
  await _calendar(
    tester,
    GoogleCalendarPermission.read,
    'Read and edit calendars',
  );
  await _tap(tester, find.text('Reconnect Google'));
  expect(find.textContaining('was cancelled'), findsOneWidget);
  expect(c.active!.permissions.calendar, GoogleCalendarPermission.read);
  await _tap(tester, find.text('Reconnect Google'));
  expect(c.active!.permissions.calendar, GoogleCalendarPermission.edit);
  await _tap(tester, find.text('Mail'));
  await _tap(tester, find.text('Preferences'));
  await _tap(tester, find.text('System'));
  await _tap(tester, find.text('Dark').last);
  await _show(tester, find.text('Reconnect Google'));
  await tester.pumpAndSettle();
  await capture?.call('google-mobile-consent-dark');
  await _tap(tester, find.text('Disconnect…'));
  await _tap(tester, find.text('Cancel'));
  expect(c.active, isNotNull);
  sdk.failSignOut = true;
  await _tap(tester, find.text('Disconnect…'));
  await _tap(tester, find.text('Disconnect').last);
  expect(c.active, isNull);
  expect(c.cleanupPending, true);
  await _show(tester, find.text('Retry cleanup'));
  await tester.pumpAndSettle();
  await capture?.call('google-mobile-cleanup-dark');
  sdk.failSignOut = false;
  await _tap(tester, find.text('Retry cleanup'));
  expect(c.cleanupPending, false);
  expect(store.value.active, isNull);
  store.unconfirmedWrite = true;
  await _tap(tester, find.text('Sign in with Google'));
  expect(c.loaded, false);
  expect(find.textContaining('Could not confirm'), findsOneWidget);
  await _show(tester, find.text('Retry reading Google connection'));
  await capture?.call('google-mobile-unconfirmed-storage-dark');
  store.unconfirmedWrite = false;
  store.failRead = true;
  await _tap(tester, find.text('Retry reading Google connection'));
  expect(c.loaded, false);
  store.failRead = false;
  await _tap(tester, find.text('Retry reading Google connection'));
  expect(c.loaded, true);
  expect(c.active!.subject, FixtureGoogleAuthorization.subject);
  expect(tester.takeException(), isNull);
}

Future<void> googlePendingControls(
  WidgetTester tester, {
  Future<void> Function(String)? capture,
}) async {
  const read = GooglePermissions(calendar: GoogleCalendarPermission.read);
  final store = MemoryGoogleStore(
    const GoogleConnectionState(
      requested: read,
      active: GoogleConnectionRecord(
        FixtureGoogleAuthorization.subject,
        FixtureGoogleAuthorization.email,
        read,
        FixtureGoogleAuthorization.application,
      ),
    ),
  );
  final sdk = FixtureGoogleAuthorization()..hold = Completer<void>();
  final c = GoogleConnection(sdk, store);
  final w = Workspace(PreviewRepository(), MemorySettings(), google: c);
  addTearDown(w.dispose);
  await w.initialize();
  await tester.pumpWidget(ShepApp(workspace: w));
  await tester.pumpAndSettle();
  await _tap(tester, find.text('Preferences'));
  await _tap(tester, find.byKey(const ValueKey('google-drive')));
  await _tap(tester, find.text('Reconnect Google'));
  expect(c.busy, true);
  expect(sdk.connects, 1);
  await _tap(tester, find.text('Mail'));
  await _tap(tester, find.text('A little room for good ideas').first);
  if (find.text('Preferences').evaluate().isEmpty) {
    await _tap(tester, find.byTooltip('Back'));
  }
  await _tap(tester, find.text('Preferences'));
  await _calendar(
    tester,
    GoogleCalendarPermission.read,
    'Read and edit calendars',
  );
  expect(c.active!.permissions, read);
  sdk.hold!.complete();
  await tester.pumpAndSettle();
  expect(find.textContaining('choices changed'), findsOneWidget);
  expect(store.value.active!.permissions, read);
  await _show(tester, find.textContaining('choices changed'));
  await tester.pumpAndSettle();
  await capture?.call('google-mobile-changed-consent');
  expect(tester.takeException(), isNull);
}
