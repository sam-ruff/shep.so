import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/profile_discovery.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import '../workspace_test.dart' show MemorySettings;
import 'google_fixture.dart';
import 'preview_repository.dart';
import 'profile_discovery_fixture.dart';

Future<void> _show(
  WidgetTester t,
  Finder finder, {
  String list = 'preferences-list',
  double delta = 300,
}) async {
  if (finder.evaluate().isEmpty) {
    await t.scrollUntilVisible(
      finder,
      delta,
      scrollable: find
          .descendant(
            of: find.byKey(ValueKey(list)),
            matching: find.byType(Scrollable),
          )
          .first,
      maxScrolls: 50,
    );
  }
  await t.ensureVisible(finder);
  await t.pump(const Duration(milliseconds: 100));
}

Future<void> _tap(
  WidgetTester t,
  Finder finder, {
  String list = 'preferences-list',
  double delta = 300,
  bool settle = true,
}) async {
  await _show(t, finder, list: list, delta: delta);
  await t.tap(finder);
  if (settle) {
    await t.pumpAndSettle();
  } else {
    await t.pump();
  }
}

Future<void> _wait(WidgetTester t, bool Function() condition) async {
  for (var i = 0; i < 100; i++) {
    if (condition()) return;
    await t.pump(const Duration(milliseconds: 20));
  }
  fail('The discovery control did not reach its expected boundary.');
}

Future<(Workspace, ProfileDiscovery)> _mount(
  WidgetTester t,
  FixtureProfileDiscovery repository,
) async {
  const permissions = GooglePermissions(drive: true);
  final google = GoogleConnection(
    FixtureGoogleAuthorization(),
    MemoryGoogleStore(
      const GoogleConnectionState(
        requested: permissions,
        active: GoogleConnectionRecord(
          FixtureGoogleAuthorization.subject,
          FixtureGoogleAuthorization.email,
          permissions,
          FixtureGoogleAuthorization.application,
        ),
      ),
    ),
  );
  await google.load();
  final discovery = ProfileDiscovery(
    google,
    repository,
    namespace: 'so.shep.fixture',
  );
  final workspace = Workspace(
    PreviewRepository(),
    MemorySettings(),
    google: google,
    profileDiscovery: discovery,
  );
  addTearDown(workspace.dispose);
  await workspace.initialize();
  await t.pumpWidget(ShepApp(workspace: workspace));
  await t.pumpAndSettle();
  await _tap(t, find.text('Preferences'));
  await _tap(t, find.byType(DropdownButton<ThemeMode>));
  await _tap(t, find.text('Light').last);
  await _tap(t, find.text('Saved Google profiles'));
  return (workspace, discovery);
}

Future<void> profileDiscoveryControls(
  WidgetTester tester, {
  Future<void> Function(String)? capture,
}) async {
  final repository = FixtureProfileDiscovery(count: 52)..failNext = true;
  final (_, d) = await _mount(tester, repository);
  await _tap(
    tester,
    find.text('Find profiles'),
    list: 'profile-discovery-list',
  );
  expect(d.error, contains('could not be read'));
  expect(find.text('Profile discovery complete'), findsNothing);
  await _show(
    tester,
    find.textContaining('could not be read'),
    list: 'profile-discovery-list',
  );
  await capture?.call('profile-discovery-error-light');
  await _tap(
    tester,
    find.text('Retry discovery'),
    list: 'profile-discovery-list',
  );
  expect(d.state!.complete, true);
  expect(d.profiles.length, 50);
  await _show(tester, find.text('Personal'), list: 'profile-discovery-list');
  await capture?.call('profile-discovery-found-light');
  await _tap(
    tester,
    find.text('Next profiles'),
    list: 'profile-discovery-list',
  );
  expect(d.profiles.length, 2);
  expect(d.profiles.first.name, 'Profile 51');
  await _tap(tester, find.byTooltip('Back'));
  await _tap(tester, find.byType(DropdownButton<ThemeMode>), delta: -300);
  await _tap(tester, find.text('Dark').last);
  await _tap(tester, find.text('Saved Google profiles'));
  await _show(tester, find.text('Profile 51'), list: 'profile-discovery-list');
  await capture?.call('profile-discovery-page-dark');
  await _tap(tester, find.text('First page'), list: 'profile-discovery-list');
  expect(d.profiles.first.name, 'Personal');
  expect(tester.takeException(), isNull);
}

Future<void> profileDiscoveryPendingControls(
  WidgetTester tester, {
  Future<void> Function(String)? capture,
}) async {
  final repository = FixtureProfileDiscovery()..holdAdvance = Completer<void>();
  final (_, d) = await _mount(tester, repository);
  await _tap(
    tester,
    find.text('Find profiles'),
    list: 'profile-discovery-list',
    settle: false,
  );
  await _wait(tester, () => repository.advances == 1);
  await _tap(
    tester,
    find.text('Pause discovery'),
    list: 'profile-discovery-list',
    settle: false,
  );
  expect(d.paused, true);
  await _tap(tester, find.byTooltip('Back'));
  await _tap(tester, find.text('Mail'));
  await _tap(tester, find.text('A little room for good ideas').first);
  expect(d.busy, true);
  repository.holdAdvance!.complete();
  await _wait(tester, () => !d.busy);
  expect(repository.advances, 1);
  if (find.text('Preferences').evaluate().isEmpty) {
    await _tap(tester, find.byTooltip('Back'));
  }
  await _tap(tester, find.text('Preferences'));
  await _tap(tester, find.text('Saved Google profiles'));
  await _show(
    tester,
    find.text('Discovery paused'),
    list: 'profile-discovery-list',
  );
  await capture?.call('profile-discovery-paused');
  await _tap(
    tester,
    find.text('Resume discovery'),
    list: 'profile-discovery-list',
  );
  expect(d.state!.complete, true);
  expect(d.profiles.length, 2);
  await _show(tester, find.text('Personal'), list: 'profile-discovery-list');
  await capture?.call('profile-discovery-resumed');
  expect(tester.takeException(), isNull);
}
