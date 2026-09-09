import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/data/profile_settings.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/profile_discovery.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'google_fixture.dart';
import 'profile_enrollment_fixture.dart';

Future<void> showEnrollmentControl(
  WidgetTester tester,
  Finder target, {
  String list = 'profile-enrollment-list',
  double delta = 300,
}) async {
  if (target.evaluate().isEmpty) {
    await tester.scrollUntilVisible(
      target,
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
  await tester.ensureVisible(target);
  await tester.pump(const Duration(milliseconds: 100));
}

Future<void> tapEnrollmentControl(
  WidgetTester tester,
  Finder target, {
  String list = 'profile-enrollment-list',
  double delta = 300,
  bool settle = true,
}) async {
  await showEnrollmentControl(tester, target, list: list, delta: delta);
  await tester.tap(target);
  if (settle) {
    await tester.pumpAndSettle();
  } else {
    await tester.pump();
  }
}

Future<(Workspace, ProfileDiscovery, VoidCallback)> mountEnrollment(
  WidgetTester tester,
  FixtureProfileEnrollment repository,
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
  await store.write(
    const Preferences(appearance: ThemeMode.light, previewLines: 4),
  );
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
  await tapEnrollmentControl(
    tester,
    find.text('Saved Google profiles'),
    list: 'preferences-list',
  );
  await tapEnrollmentControl(
    tester,
    find.text('Find profiles'),
    list: 'profile-discovery-list',
  );
  await tapEnrollmentControl(
    tester,
    find.text('Personal'),
    list: 'profile-discovery-list',
  );
  return (workspace, discovery, cleanup);
}

Future<void> profileEnrollmentReviewControls(
  WidgetTester tester, {
  Future<void> Function(String)? capture,
}) async {
  final fixture = FixtureProfileEnrollment(EnrollmentMail(), accounts: 52)
    ..lostAccountReplyOnce = true
    ..lostSettingsReplyOnce = true;
  final (workspace, d, cleanup) = await mountEnrollment(tester, fixture);
  try {
    expect(d.enrollment!.needsReview, true);
    expect(d.enrollmentRows.length, 50);
    await showEnrollmentControl(tester, find.text('Account connections'));
    await capture?.call('profile-enrollment-review-light');
    await tapEnrollmentControl(tester, find.text('Next review items'));
    expect(d.enrollmentRows.length, 4);
    await showEnrollmentControl(
      tester,
      find.text('Saved account 51'),
      delta: -300,
    );
    await capture?.call('profile-enrollment-second-page');
    await tapEnrollmentControl(tester, find.text('Connection details').first);
    expect(find.textContaining('mail.example.test:993'), findsOneWidget);
    await capture?.call('profile-enrollment-connection-details');
    await tapEnrollmentControl(tester, find.text('Close'));
    await tapEnrollmentControl(tester, find.text('Saved account 51'));
    expect(d.enrollmentRows.first.selected, false);
    await showEnrollmentControl(tester, find.text('Cancel review'));
    final footer = find.ancestor(
      of: find.text('Cancel review'),
      matching: find.byType(TextButton),
    );
    final safeBottom =
        (tester.view.physicalSize.height - tester.view.padding.bottom) /
        tester.view.devicePixelRatio;
    expect(tester.getRect(footer).bottom, lessThanOrEqualTo(safeBottom));
    await capture?.call('profile-enrollment-review-footer');
    await tapEnrollmentControl(tester, find.text('Apply selected changes'));
    expect(d.error, contains('reply was lost'));
    expect(fixture.mail.imported.length, 1);
    await showEnrollmentControl(tester, find.textContaining('reply was lost'));
    await capture?.call('profile-enrollment-retry');
    await tapEnrollmentControl(tester, find.text('Resume enrollment'));
    expect(d.error, contains('Preferences were saved'));
    final store = workspace.settings as ProfileSettingsStore;
    final original = await store.profileSnapshot();
    await showEnrollmentControl(
      tester,
      find.textContaining('Preferences were saved'),
    );
    await capture?.call('profile-enrollment-settings-retry-dark');
    await tapEnrollmentControl(tester, find.byTooltip('Back'));
    await tapEnrollmentControl(tester, find.byTooltip('Back'));
    await tapEnrollmentControl(
      tester,
      find.byType(DropdownButton<ThemeMode>),
      list: 'preferences-list',
      delta: -300,
    );
    await tapEnrollmentControl(
      tester,
      find.text('Light').last,
      list: 'preferences-list',
    );
    await tapEnrollmentControl(
      tester,
      find.text('Saved Google profiles'),
      list: 'preferences-list',
    );
    await tapEnrollmentControl(
      tester,
      find.text('Resume profile review and application'),
      list: 'profile-discovery-list',
    );
    await tapEnrollmentControl(tester, find.text('Resume enrollment'));
    expect(d.enrollment!.complete, true);
    expect(workspace.preferences.appearance, ThemeMode.light);
    expect(fixture.job!['settings_receipt']['revisions'], original.revisions);
    expect(
      (await store.profileSnapshot()).revisions['appearance'],
      greaterThan(original.revisions['appearance']!),
    );
    await showEnrollmentControl(
      tester,
      find.textContaining('Profile applied ·'),
    );
    await capture?.call('profile-enrollment-newer-preference-kept-light');
    await tapEnrollmentControl(tester, find.text('Done'));
    await tapEnrollmentControl(tester, find.byTooltip('Back'));
    await tapEnrollmentControl(
      tester,
      find.byType(DropdownButton<ThemeMode>),
      list: 'preferences-list',
      delta: -300,
    );
    await tapEnrollmentControl(
      tester,
      find.text('Dark').last,
      list: 'preferences-list',
    );
    await tapEnrollmentControl(
      tester,
      find.text('Saved Google profiles'),
      list: 'preferences-list',
    );
    await tapEnrollmentControl(
      tester,
      find.text('Profile applied on this device'),
      list: 'profile-discovery-list',
    );
    expect(fixture.mail.imported.length, 51);
    expect(fixture.mail.imported.map((a) => a.id).toSet().length, 51);
    expect(workspace.preferences.appearance, ThemeMode.dark);
    expect(workspace.preferences.previewLines, 2);
    await showEnrollmentControl(
      tester,
      find.textContaining('Profile applied ·'),
    );
    await capture?.call('profile-enrollment-complete-dark');
    await tapEnrollmentControl(tester, find.text('Done'));
    await tapEnrollmentControl(tester, find.byTooltip('Back'));
    await showEnrollmentControl(
      tester,
      find.textContaining('Reconnect required').first,
      list: 'preferences-list',
    );
    await capture?.call('profile-enrollment-reconnect');
    await showEnrollmentControl(
      tester,
      find.text('Sender pictures'),
      list: 'preferences-list',
      delta: -300,
    );
    // Two real control changes before the next repaint must merge current intent.
    await tester.tap(find.text('Sender pictures').hitTestable());
    await tester.tap(find.text('Unified inbox').hitTestable());
    expect(workspace.preferences.avatars, false);
    expect(workspace.preferences.unified, false);
    await tester.pumpAndSettle();
    final saved = await workspace.settings.read();
    expect(saved.avatars, false);
    expect(saved.unified, false);
    expect(saved.appearance, ThemeMode.dark);
    await capture?.call('profile-enrollment-rapid-preferences');
    await tapEnrollmentControl(
      tester,
      find.text('Mail'),
      list: 'preferences-list',
    );
    expect(find.text('A little room for good ideas'), findsOneWidget);
  } finally {
    if (fixture.holdStep case final held?) {
      if (!held.isCompleted) held.complete();
    }
    await tester.pumpWidget(const SizedBox.shrink());
    cleanup();
    await tester.pump();
  }
}

Future<void> profileEnrollmentPauseControls(
  WidgetTester tester, {
  Future<void> Function(String)? capture,
}) async {
  final fixture = FixtureProfileEnrollment(EnrollmentMail());
  final (workspace, d, cleanup) = await mountEnrollment(tester, fixture);
  try {
    fixture.holdStep = Completer<void>();
    await tapEnrollmentControl(
      tester,
      find.text('Apply selected changes'),
      settle: false,
    );
    for (var i = 0; i < 100 && fixture.steps < 4; i++) {
      await tester.pump(const Duration(milliseconds: 20));
    }
    expect(fixture.steps, 4);
    await tapEnrollmentControl(
      tester,
      find.text('Pause enrollment'),
      settle: false,
    );
    fixture.holdStep!.complete();
    await tester.pumpAndSettle();
    expect(d.paused, true);
    expect(fixture.mail.imported.length, 1);
    await showEnrollmentControl(tester, find.text('Resume enrollment'));
    await capture?.call('profile-enrollment-paused');
    await tapEnrollmentControl(tester, find.byTooltip('Back'));
    await tapEnrollmentControl(tester, find.byTooltip('Back'));
    await tapEnrollmentControl(
      tester,
      find.text('Mail'),
      list: 'preferences-list',
    );
    expect(find.text('A little room for good ideas'), findsOneWidget);
    fixture.holdStep = null;
    await tapEnrollmentControl(
      tester,
      find.text('Preferences'),
      list: 'preferences-list',
    );
    await tapEnrollmentControl(
      tester,
      find.text('Saved Google profiles'),
      list: 'preferences-list',
    );
    await tapEnrollmentControl(
      tester,
      find.text('Resume profile review and application'),
      list: 'profile-discovery-list',
    );
    await tapEnrollmentControl(tester, find.text('Resume enrollment'));
    expect(d.enrollment!.complete, true);
    expect(workspace.preferences.appearance, ThemeMode.dark);
  } finally {
    if (fixture.holdStep case final held?) {
      if (!held.isCompleted) held.complete();
    }
    await tester.pumpWidget(const SizedBox.shrink());
    cleanup();
    await tester.pump();
  }
}
