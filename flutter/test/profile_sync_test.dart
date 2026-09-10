import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/profile_settings.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/profile_discovery.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'support/google_fixture.dart';
import 'support/profile_enrollment_fixture.dart';
import 'support/profile_sync_fixture.dart';

Future<(Workspace, ProfileDiscovery, GoogleConnection)> setup(
  FixtureProfileSync repository, {
  Preferences saved = const Preferences(
    appearance: ThemeMode.light,
    previewLines: 4,
  ),
}) async {
  final google = GoogleConnection(
    FixtureGoogleAuthorization(),
    MemoryGoogleStore(),
  );
  await google.load();
  google.choose(const GooglePermissions(drive: true));
  await google.connect();
  final d = ProfileDiscovery(google, repository, namespace: 'so.shep.fixture');
  final store = DeviceSettings(storage: EnrollmentPreferences());
  await store.write(saved);
  final workspace = Workspace(
    repository.mail,
    store,
    google: google,
    profileDiscovery: d,
  );
  addTearDown(workspace.dispose);
  await workspace.initialize();
  return (workspace, d, google);
}

Future<void> until(bool Function() check) async {
  for (var i = 0; i < 200; i++) {
    if (check()) return;
    await Future<void>.delayed(Duration.zero);
  }
  fail('The sync fixture did not reach the expected state.');
}

void main() {
  test(
    'a completed enrollment seeds a paused subscription that applies remote values through device receipts and publishes local intent',
    () async {
      final repository = FixtureProfileSync(EnrollmentMail(), accounts: 1);
      final (workspace, d, _) = await setup(repository);
      final device = workspace.profileApplication!;
      await d.discover();
      await d.prepareEnrollment(d.profiles.single, device);
      await d.approveEnrollment(device, accounts: true, settings: true);
      expect(d.enrollment!.complete, true);
      expect(repository.subscribes, 1);
      expect(d.sync, isNotNull);
      expect(d.sync!.enabled, false);
      expect(workspace.preferences.appearance, ThemeMode.dark);
      // Paused: the foreground tick runs nothing.
      await d.syncTick(device);
      expect(repository.cycles, 0);
      await d.configureSync(enabled: true);
      expect(d.sync!.enabled, true);
      // A remote change applies through the production preference store.
      repository.pendingRemote['appearance'] = 'System';
      repository.pendingRemote['tooltips'] = false;
      await d.syncTick(device);
      expect(repository.cycles, greaterThanOrEqualTo(2));
      expect(workspace.preferences.appearance, ThemeMode.system);
      expect(workspace.preferences.tooltips, false);
      expect(repository.application, isNull);
      expect(repository.confirms, 2);
      expect(repository.receipt!['applied'], ['tooltips']);
      final store = workspace.settings as ProfileSettingsStore;
      final snapshot = await store.profileSnapshot();
      expect(repository.receipt!['revisions'], snapshot.revisions);
      expect(d.sync!.last!.remaining, false);
      // A local edit is published; an unchanged snapshot publishes nothing.
      await workspace.savePreferences(
        workspace.preferences.copy(appearance: ThemeMode.dark),
      );
      await d.syncNow(device);
      expect(repository.published, [
        {'appearance': 'Dark'},
      ]);
      expect(repository.remote['appearance'], 'Dark');
      final cycles = repository.cycles;
      await d.syncNow(device);
      expect(repository.published.length, 1);
      expect(repository.cycles, cycles + 1);
      // A tick during a save waits for the saved snapshot, never a stale one.
      final pending = workspace.savePreferences(
        workspace.preferences.copy(tooltips: true),
      );
      await d.syncTick(device);
      await pending;
      expect(repository.published.last, {'tooltips': true});
      // A disabled field neither applies nor publishes.
      await d.configureSync(field: 'appearance', selected: false);
      repository.pendingRemote['appearance'] = 'Light';
      await workspace.savePreferences(
        workspace.preferences.copy(appearance: ThemeMode.light),
      );
      await d.syncNow(device);
      expect(workspace.preferences.appearance, ThemeMode.light);
      expect(repository.published.where((p) => p.containsKey('appearance')).length, 1);
      expect(d.sync!.reviews, 0);
    },
  );

  test(
    'a lost application receipt is retried with the same identity before new work, and failures keep status',
    () async {
      final repository = FixtureProfileSync(
        EnrollmentMail(),
        accounts: 0,
        completed: true,
      )..lostConfirmOnce = true;
      final (workspace, d, _) = await setup(
        repository,
        saved: const Preferences(),
      );
      final device = workspace.profileApplication!;
      await d.refreshSync();
      expect(d.syncChecked, true);
      expect(d.sync, isNull);
      expect(d.canSubscribe, true);
      await d.subscribeSync(device);
      expect(d.sync, isNotNull);
      await d.configureSync(enabled: true);
      repository.pendingRemote['appearance'] = 'Dark';
      await d.syncNow(device);
      expect(d.syncError, contains('reply was lost'));
      expect(workspace.preferences.appearance, ThemeMode.dark);
      expect(repository.application, isNotNull);
      final id = repository.application!['id'];
      final store = workspace.settings as ProfileSettingsStore;
      final before = (await store.profileSnapshot()).revisions;
      await d.syncNow(device);
      expect(d.syncError, isNull);
      expect(repository.application, isNull);
      expect(repository.receipt!['id'], id);
      expect(repository.receipt!['revisions'], before);
      expect(repository.confirms, 2);
      // A failed cycle reports its error and the next Sync now clears it.
      repository.failCycleOnce = true;
      await d.syncNow(device);
      expect(d.syncError, contains('Google Drive'));
      await d.syncNow(device);
      expect(d.syncError, isNull);
      expect(d.sync!.error, isNull);
    },
  );

  test(
    'concurrent edits open a paged review whose decisions keep mine or use the profile',
    () async {
      final repository = FixtureProfileSync(
        EnrollmentMail(),
        accounts: 0,
        completed: true,
      )..conflictVersions = 52;
      final (workspace, d, _) = await setup(
        repository,
        saved: const Preferences(),
      );
      final device = workspace.profileApplication!;
      await d.refreshSync();
      await d.subscribeSync(device);
      await d.configureSync(enabled: true);
      repository.pendingRemote['preview_lines'] = 1;
      await workspace.savePreferences(
        workspace.preferences.copy(previewLines: 3),
      );
      await d.syncNow(device);
      expect(d.sync!.reviews, 1);
      expect(d.syncReviews.single.field, 'preview_lines');
      expect(d.syncReviews.single.total, 52);
      expect(workspace.preferences.previewLines, 3);
      final review = d.syncReviews.single;
      await d.openSyncReview(review);
      expect(d.syncVersions.length, 50);
      expect(d.syncSeen, 50);
      // Every page must be seen before a decision.
      await d.decideSync(review, device);
      expect(d.syncError, contains('every version page'));
      expect(repository.reviews.length, 1);
      await d.syncVersionsPage(first: false);
      expect(d.syncVersions.length, 2);
      expect(d.syncSeen, 52);
      await d.syncVersionsPage(first: true);
      expect(d.syncSeen, 52);
      // Changing the preference after opening the review requires a refresh.
      await workspace.savePreferences(
        workspace.preferences.copy(previewLines: 2),
      );
      await d.decideSync(review, device);
      expect(d.syncError, contains('changed on the device'));
      await d.syncNow(device);
      final refreshed = d.syncReviews.single;
      await d.openSyncReview(refreshed);
      await d.syncVersionsPage(first: false);
      await d.decideSync(refreshed, device);
      expect(d.syncError, isNull);
      expect(d.syncReviews, isEmpty);
      expect(repository.published.last, {'preview_lines': 2});
      expect(workspace.preferences.previewLines, 2);
      // Use profile stages a device write with its own receipt.
      repository.pendingRemote['appearance'] = 'Dark';
      await workspace.savePreferences(
        workspace.preferences.copy(appearance: ThemeMode.light),
      );
      await d.syncNow(device);
      final conflict = d.syncReviews.single;
      await d.openSyncReview(conflict);
      await d.syncVersionsPage(first: false);
      await d.decideSync(
        conflict,
        device,
        shared: d.syncVersions.first.operation,
      );
      expect(d.syncError, isNull);
      expect(workspace.preferences.appearance, ThemeMode.dark);
      expect(repository.application, isNull);
      expect(repository.receipt!['applied'], ['appearance']);
    },
  );

  test(
    'disconnecting Google stops cycles locally and a held cycle cannot update a replaced grant',
    () async {
      final repository = FixtureProfileSync(
        EnrollmentMail(),
        accounts: 0,
        completed: true,
      );
      final (workspace, d, google) = await setup(
        repository,
        saved: const Preferences(),
      );
      final device = workspace.profileApplication!;
      await d.refreshSync();
      await d.subscribeSync(device);
      await d.configureSync(enabled: true);
      repository.holdCycle = Completer<void>();
      final running = d.syncNow(device);
      await until(() => repository.cycles == 1);
      expect(d.syncing, true);
      await google.disconnect();
      expect(d.connected, false);
      expect(d.sync, isNull);
      repository.pendingRemote['appearance'] = 'Dark';
      repository.holdCycle!.complete();
      await running;
      expect(d.sync, isNull);
      expect(d.syncing, false);
      expect(workspace.preferences.appearance, ThemeMode.system);
      expect(repository.closes, 1);
      await d.syncTick(device);
      expect(repository.cycles, 1);
      // Reconnecting the same account resumes the durable subscription.
      google.choose(const GooglePermissions(drive: true));
      await google.connect();
      await d.syncTick(device);
      expect(d.sync, isNotNull);
      expect(d.sync!.enabled, true);
      expect(workspace.preferences.appearance, ThemeMode.dark);
    },
  );
}
