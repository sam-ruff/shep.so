import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/profile_discovery.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'google_fixture.dart';
import 'profile_enrollment_controls.dart';
import 'profile_enrollment_fixture.dart';
import 'profile_sync_fixture.dart';

Future<(Workspace, ProfileDiscovery, VoidCallback)> mountSync(
  WidgetTester tester,
  FixtureProfileSync repository,
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
  final store = DeviceSettings(storage: EnrollmentPreferences());
  await store.write(const Preferences());
  final workspace = Workspace(
    repository.mail,
    store,
    google: google,
    profileDiscovery: discovery,
  );
  var disposed = false;
  void cleanup() {
    if (disposed) return;
    disposed = true;
    workspace.dispose();
  }

  addTearDown(cleanup);
  await workspace.initialize();
  await tester.pumpWidget(ShepApp(workspace: workspace));
  await tester.pumpAndSettle();
  await tapEnrollmentControl(
    tester,
    find.text('Preferences'),
    list: 'preferences-list',
  );
  return (workspace, discovery, cleanup);
}

Future<void> _theme(WidgetTester tester, String value) async {
  await tapEnrollmentControl(
    tester,
    find.byType(DropdownButton<ThemeMode>),
    list: 'preferences-list',
    delta: -300,
  );
  await tapEnrollmentControl(
    tester,
    find.text(value).last,
    list: 'preferences-list',
  );
}

/// Real Preferences controls: seed sync from the applied profile, turn it on,
/// apply a remote value, publish a local edit, review a paged conflict with
/// Keep mine, then Use profile, then lose a device receipt and retry it.
Future<void> profileSyncControls(
  WidgetTester tester, {
  Future<void> Function(String)? capture,
}) async {
  final fixture = FixtureProfileSync(EnrollmentMail(), accounts: 0, completed: true)
    ..conflictVersions = 52;
  final (workspace, d, cleanup) = await mountSync(tester, fixture);
  try {
    await showEnrollmentControl(
      tester,
      find.text('Preference sync is not set up'),
      list: 'preferences-list',
    );
    await capture?.call('profile-sync-unset');
    await tapEnrollmentControl(
      tester,
      find.text('Keep in sync'),
      list: 'preferences-list',
    );
    expect(d.sync, isNotNull);
    expect(d.sync!.enabled, false);
    expect(find.textContaining('Paused on this device'), findsOneWidget);
    // Paused: the foreground tick does nothing.
    await d.syncTick(workspace.profileApplication);
    expect(fixture.cycles, 0);
    fixture.pendingRemote['appearance'] = 'Dark';
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-master')),
      list: 'preferences-list',
    );
    expect(d.sync!.enabled, true);
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-now')),
      list: 'preferences-list',
    );
    expect(workspace.preferences.appearance, ThemeMode.dark);
    expect(fixture.receipt!['applied'], ['appearance']);
    expect(find.textContaining('1 applied here, 0 published'), findsOneWidget);
    await capture?.call('profile-sync-applied-dark');
    // A per-field switch keeps a remote change out of this device.
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-field-tooltips')),
      list: 'preferences-list',
    );
    expect(d.sync!.fieldEnabled('tooltips'), false);
    fixture.pendingRemote['tooltips'] = false;
    // A real theme change is local intent that the next sync publishes.
    await _theme(tester, 'Light');
    fixture.pendingRemote['preview_lines'] = 1;
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-now')),
      list: 'preferences-list',
    );
    expect(fixture.published.last, {'appearance': 'Light'});
    expect(workspace.preferences.tooltips, true);
    expect(workspace.preferences.previewLines, 1);
    expect(d.sync!.reviews, 0);
    // Concurrent edits: the other device changes theme while we change it too.
    fixture.pendingRemote['appearance'] = 'System';
    await _theme(tester, 'Dark');
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-now')),
      list: 'preferences-list',
    );
    expect(d.sync!.reviews, 1);
    expect(workspace.preferences.appearance, ThemeMode.dark);
    await showEnrollmentControl(
      tester,
      find.text('Review 1 preference conflict'),
      list: 'preferences-list',
    );
    await capture?.call('profile-sync-conflict-dark');
    await tapEnrollmentControl(
      tester,
      find.text('Review 1 preference conflict'),
      list: 'preferences-list',
    );
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-review-appearance')),
      list: 'profile-sync-review-list',
    );
    expect(d.syncVersions.length, 50);
    expect(find.text('50 of 52 shared versions reviewed'), findsOneWidget);
    await showEnrollmentControl(
      tester,
      find.text('Keep mine'),
      list: 'profile-sync-review-list',
    );
    final keepMine = find.byKey(const ValueKey('profile-sync-keep-mine'));
    expect(tester.widget<OutlinedButton>(keepMine).onPressed, isNull);
    final safeBottom =
        (tester.view.physicalSize.height - tester.view.padding.bottom) /
        tester.view.devicePixelRatio;
    expect(tester.getRect(keepMine).bottom, lessThanOrEqualTo(safeBottom));
    await capture?.call('profile-sync-review-first-page');
    await tapEnrollmentControl(
      tester,
      find.text('Next versions'),
      list: 'profile-sync-review-list',
    );
    expect(find.text('52 of 52 shared versions reviewed'), findsOneWidget);
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-keep-mine')),
      list: 'profile-sync-review-list',
    );
    expect(d.syncReviews, isEmpty);
    expect(fixture.published.last, {'appearance': 'Dark'});
    expect(find.text('No preference conflicts are waiting.'), findsOneWidget);
    await capture?.call('profile-sync-review-resolved');
    await tapEnrollmentControl(tester, find.byTooltip('Back'));
    // Use profile: the shared value is written here with its own receipt,
    // and a lost receipt reply is retried with the same identity.
    fixture.pendingRemote['appearance'] = 'Light';
    await _theme(tester, 'System');
    fixture.lostConfirmOnce = true;
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-now')),
      list: 'preferences-list',
    );
    expect(d.sync!.reviews, 1);
    await tapEnrollmentControl(
      tester,
      find.text('Review 1 preference conflict'),
      list: 'preferences-list',
    );
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-review-appearance')),
      list: 'profile-sync-review-list',
    );
    await tapEnrollmentControl(
      tester,
      find.text('Next versions'),
      list: 'profile-sync-review-list',
    );
    await tapEnrollmentControl(
      tester,
      find.text('Use profile').first,
      list: 'profile-sync-review-list',
    );
    expect(d.syncError, contains('reply was lost'));
    expect(workspace.preferences.appearance, ThemeMode.light);
    final pending = fixture.application!['id'];
    await showEnrollmentControl(
      tester,
      find.textContaining('reply was lost'),
      list: 'profile-sync-review-list',
    );
    await capture?.call('profile-sync-lost-receipt');
    await tapEnrollmentControl(tester, find.byTooltip('Back'));
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-now')),
      list: 'preferences-list',
    );
    expect(d.syncError, isNull);
    expect(fixture.application, isNull);
    expect(fixture.receipt!['id'], pending);
    expect(workspace.preferences.appearance, ThemeMode.light);
    await showEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-status')),
      list: 'preferences-list',
    );
    await capture?.call('profile-sync-recovered-light');
    await tapEnrollmentControl(
      tester,
      find.text('Mail'),
      list: 'preferences-list',
    );
    expect(find.text('A little room for good ideas'), findsOneWidget);
  } finally {
    if (fixture.holdCycle case final held?) {
      if (!held.isCompleted) held.complete();
    }
    await tester.pumpWidget(const SizedBox.shrink());
    cleanup();
    await tester.pump();
  }
}

/// Google disconnect while a cycle is held: the cycle cannot publish into the
/// replaced grant, controls explain the paused state, and mail stays usable.
Future<void> profileSyncDisconnectControls(
  WidgetTester tester, {
  Future<void> Function(String)? capture,
}) async {
  final fixture = FixtureProfileSync(EnrollmentMail(), accounts: 0, completed: true);
  final (workspace, d, cleanup) = await mountSync(tester, fixture);
  try {
    await tapEnrollmentControl(
      tester,
      find.text('Keep in sync'),
      list: 'preferences-list',
    );
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-master')),
      list: 'preferences-list',
    );
    fixture.holdCycle = Completer<void>();
    await tapEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-now')),
      list: 'preferences-list',
      settle: false,
    );
    for (var i = 0; i < 100 && fixture.cycles < 1; i++) {
      await tester.pump(const Duration(milliseconds: 20));
    }
    expect(find.text('Syncing preferences…'), findsOneWidget);
    await workspace.google!.disconnect();
    fixture.pendingRemote['appearance'] = 'Dark';
    fixture.holdCycle!.complete();
    await tester.pumpAndSettle();
    expect(d.sync, isNull);
    expect(workspace.preferences.appearance, ThemeMode.system);
    await showEnrollmentControl(
      tester,
      find.byKey(const ValueKey('profile-sync-disconnected')),
      list: 'preferences-list',
    );
    await capture?.call('profile-sync-disconnected');
    await tapEnrollmentControl(
      tester,
      find.text('Mail'),
      list: 'preferences-list',
    );
    expect(find.text('A little room for good ideas'), findsOneWidget);
  } finally {
    if (fixture.holdCycle case final held?) {
      if (!held.isCompleted) held.complete();
    }
    await tester.pumpWidget(const SizedBox.shrink());
    cleanup();
    await tester.pump();
  }
}
