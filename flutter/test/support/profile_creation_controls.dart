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
import 'profile_creation_fixture.dart';

Future<void> _show(
  WidgetTester t,
  Finder finder, {
  String list = 'profile-creation-list',
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
      maxScrolls: 70,
    );
  }
  await t.ensureVisible(finder);
  await t.pump(const Duration(milliseconds: 100));
}

Future<void> _tap(
  WidgetTester t,
  Finder finder, {
  String list = 'profile-creation-list',
  bool settle = true,
  double delta = 300,
}) async {
  await _show(t, finder, list: list, delta: delta);
  await t.tap(finder);
  if (settle) {
    await t.pumpAndSettle();
  } else {
    await t.pump();
  }
}

Future<void> _wait(WidgetTester t, bool Function() check) async {
  for (var i = 0; i < 100; i++) {
    if (check()) return;
    await t.pump(const Duration(milliseconds: 20));
  }
  fail('Publication control did not reach the held boundary.');
}

Future<(Workspace, ProfileDiscovery)> _mount(
  WidgetTester t,
  FixtureProfileCreation repository,
) async {
  final google = GoogleConnection(
    FixtureGoogleAuthorization(),
    MemoryGoogleStore(),
  );
  await google.load();
  google.choose(const GooglePermissions(drive: true));
  await google.connect();
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
  await _tap(t, find.text('Preferences'), list: 'preferences-list');
  await _tap(
    t,
    find.byType(DropdownButton<ThemeMode>),
    list: 'preferences-list',
  );
  await _tap(t, find.text('Light').last, list: 'preferences-list');
  await _tap(t, find.text('Saved Google profiles'), list: 'preferences-list');
  await _tap(t, find.text('Find profiles'), list: 'profile-discovery-list');
  await _tap(t, find.text('Create profile'), list: 'profile-discovery-list');
  return (workspace, discovery);
}

Future<void> profileCreationReviewControls(
  WidgetTester t, {
  Future<void> Function(String)? capture,
}) async {
  final repository = FixtureProfileCreation(accountCount: 52)
    ..failUploadOnce = true;
  final (_, d) = await _mount(t, repository);
  await t.enterText(find.byKey(const ValueKey('profile-name')), 'Shared work');
  await _tap(t, find.text('Review profile'));
  final id = d.creation!.id;
  expect(d.creation!.settings, 8);
  expect(d.creation!.accounts, 52);
  await _show(t, find.text('Appearance'));
  await capture?.call('profile-creation-review-light');
  await _tap(t, find.text('Next accounts'));
  expect(d.creationAccounts.length, 2);
  expect(find.text('Saved account 51'), findsOneWidget);
  await _show(t, find.text('Saved account 51'));
  await capture?.call('profile-creation-accounts-page');
  await _tap(t, find.text('Saved account 51'));
  expect(find.textContaining('Incoming: IMAP'), findsOneWidget);
  expect(find.textContaining('Passwords are not included'), findsOneWidget);
  await _tap(t, find.text('Close'));
  await _tap(t, find.text('Publish profile'));
  expect(d.creation!.phase, 'uploading');
  expect(d.error, contains('could not be confirmed'));
  await _show(t, find.textContaining('could not be confirmed'));
  await capture?.call('profile-creation-error');
  await _tap(t, find.text('Resume publication'));
  expect(d.creation!.complete, true);
  expect(d.creation!.id, id);
  expect(repository.prepared, 1);
  await _show(t, find.text('Profile saved to Google'));
  await capture?.call('profile-creation-complete-light');
  await _tap(t, find.byTooltip('Back'));
  await _tap(t, find.byTooltip('Back'));
  await _tap(
    t,
    find.byType(DropdownButton<ThemeMode>),
    list: 'preferences-list',
    delta: -300,
  );
  await _tap(t, find.text('Dark').last, list: 'preferences-list');
  await _tap(t, find.text('Saved Google profiles'), list: 'preferences-list');
  // The publication summary and discovered profile intentionally share the name.
  await _tap(
    t,
    find.text('Profile saved to Google'),
    list: 'profile-discovery-list',
  );
  await _show(t, find.text('Profile saved to Google'));
  await capture?.call('profile-creation-complete-dark');
  expect(t.takeException(), isNull);
}

Future<void> profileCreationPauseControls(
  WidgetTester t, {
  Future<void> Function(String)? capture,
}) async {
  final gate = Completer<void>();
  final repository = FixtureProfileCreation()..holdStep = gate;
  final (_, d) = await _mount(t, repository);
  await t.enterText(
    find.byKey(const ValueKey('profile-name')),
    'Paused profile',
  );
  await _tap(t, find.text('Include app settings'));
  await _tap(t, find.text('Review profile'));
  expect(d.creation!.settings, 0);
  await _tap(t, find.text('Publish profile'), settle: false);
  await _wait(t, () => repository.steps == 1);
  await _tap(t, find.text('Pause publication'), settle: false);
  await _tap(t, find.byTooltip('Back'), settle: false);
  await _wait(
    t,
    () =>
        find.text('Profile publication').evaluate().isEmpty &&
        find.byTooltip('Back').evaluate().length == 1,
  );
  await _tap(t, find.byTooltip('Back'));
  await _tap(t, find.text('Mail'), list: 'preferences-list');
  await _tap(
    t,
    find.text('A little room for good ideas'),
    list: 'preferences-list',
  );
  expect(find.text('Reply'), findsWidgets);
  expect(d.busy, true);
  await capture?.call('profile-creation-browse-pending');
  gate.complete();
  await _wait(t, () => !d.busy);
  expect(repository.steps, 1);
  expect(d.creation!.staged, 1);
  await _tap(t, find.byTooltip('Back'));
  await _tap(t, find.text('Preferences'), list: 'preferences-list');
  await _tap(t, find.text('Saved Google profiles'), list: 'preferences-list');
  await _tap(
    t,
    find.text('Profile publication needs to finish'),
    list: 'profile-discovery-list',
  );
  await _show(t, find.text('Resume publication'));
  await capture?.call('profile-creation-paused');
  repository.holdStep = null;
  await _tap(t, find.text('Resume publication'));
  expect(d.creation!.complete, true);
  await capture?.call('profile-creation-resumed');
  expect(t.takeException(), isNull);
}
