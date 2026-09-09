import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/data/profile_settings.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/profile_discovery.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'support/google_fixture.dart';
import 'support/profile_enrollment_fixture.dart';

Future<(Workspace, ProfileDiscovery)> setup(
  FixtureProfileEnrollment repository,
) async {
  final google = GoogleConnection(
    FixtureGoogleAuthorization(),
    MemoryGoogleStore(),
  );
  await google.load();
  google.choose(const GooglePermissions(drive: true));
  await google.connect();
  final d = ProfileDiscovery(google, repository, namespace: 'so.shep.fixture');
  final workspace = Workspace(
    repository.mail,
    DeviceSettings(storage: EnrollmentPreferences()),
    google: google,
    profileDiscovery: d,
  );
  addTearDown(workspace.dispose);
  await d.discover();
  return (workspace, d);
}

Future<void> until(bool Function() check) async {
  for (var i = 0; i < 200; i++) {
    if (check()) return;
    await Future<void>.delayed(Duration.zero);
  }
  fail('Enrollment did not reach the held step.');
}

void main() {
  test(
    'review pages, separate account receipts and lost settings acknowledgments preserve newer local edits',
    () async {
      final repository =
          FixtureProfileEnrollment(EnrollmentMail(), accounts: 52)
            ..lostAccountReplyOnce = true
            ..lostSettingsReplyOnce = true;
      final (workspace, d) = await setup(repository);
      final device = workspace.profileApplication!;
      await d.prepareEnrollment(d.profiles.single, device);
      final id = d.enrollment!.id;
      expect(d.enrollmentRows.length, 50);
      await d.enrollmentPage(first: false);
      expect(d.enrollmentRows.length, 4);
      await d.chooseEnrollment(d.enrollmentRows.first, false);
      await d.approveEnrollment(device, accounts: true, settings: true);
      expect(d.error, contains('reply was lost'));
      expect(repository.mail.imported.length, 1);
      await d.resumeEnrollment(device);
      expect(d.error, contains('Preferences were saved'));
      expect(workspace.preferences.appearance, ThemeMode.dark);
      final store = workspace.settings as ProfileSettingsStore;
      final original = await store.profileSnapshot();
      await workspace.savePreferences(
        workspace.preferences.copy(
          appearance: ThemeMode.light,
          previewLines: 4,
        ),
      );
      await d.resumeEnrollment(device);
      expect(d.error, isNull);
      expect(d.enrollment!.complete, true);
      expect(d.enrollment!.id, id);
      expect(repository.mail.imported.length, 51);
      expect(repository.prepared, 1);
      expect(repository.settingsCalls, 2);
      expect(
        repository.job!['settings_receipt']['revisions'],
        original.revisions,
      );
      expect(
        (await store.profileSnapshot()).revisions['appearance'],
        greaterThan(original.revisions['appearance']!),
      );
      expect(workspace.preferences.appearance, ThemeMode.light);
      expect(workspace.preferences.previewLines, 4);
      expect(
        (await workspace.settings.read()).encode(),
        workspace.preferences.encode(),
      );
    },
  );
  test(
    'pause and Google disconnect retain accepted work and prevent later device application',
    () async {
      final repository = FixtureProfileEnrollment(EnrollmentMail())
        ..holdStep = Completer<void>();
      final (workspace, d) = await setup(repository);
      final pending = d.prepareEnrollment(
        d.profiles.single,
        workspace.profileApplication!,
      );
      await until(() => repository.steps == 1);
      d.pause();
      repository.holdStep!.complete();
      await pending;
      expect(repository.steps, 1);
      expect(d.enrollment!.copied, 1);
      repository.holdStep = null;
      await d.resumeEnrollment(workspace.profileApplication!);
      expect(d.enrollment!.needsReview, true);
      repository.holdStep = Completer<void>();
      final approved = d.approveEnrollment(
        workspace.profileApplication!,
        accounts: true,
        settings: true,
      );
      await until(() => repository.steps == 4);
      await workspace.google!.disconnect();
      repository.holdStep!.complete();
      await approved;
      expect(d.enrollment, isNull);
      expect(repository.steps, 4);
      expect(repository.settingsCalls, 0);
      expect(workspace.preferences.appearance, ThemeMode.system);
    },
  );
  test('category and row choices are applied only after approval', () async {
    final repository = FixtureProfileEnrollment(EnrollmentMail());
    final (workspace, d) = await setup(repository);
    await d.prepareEnrollment(d.profiles.single, workspace.profileApplication!);
    expect(repository.mail.imported, isEmpty);
    expect(workspace.preferences.appearance, ThemeMode.system);
    await d.approveEnrollment(
      workspace.profileApplication!,
      accounts: false,
      settings: false,
    );
    expect(d.enrollment!.complete, true);
    expect(repository.mail.imported, isEmpty);
    expect(workspace.preferences.appearance, ThemeMode.system);
  });
}
