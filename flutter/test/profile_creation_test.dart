import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/profile_discovery.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'support/google_fixture.dart';
import 'support/profile_creation_fixture.dart';

Future<(GoogleConnection, ProfileDiscovery)> setup(
  FixtureProfileCreation repository,
) async {
  final google = GoogleConnection(
    FixtureGoogleAuthorization(),
    MemoryGoogleStore(),
  );
  await google.load();
  google.choose(const GooglePermissions(drive: true));
  await google.connect();
  final d = ProfileDiscovery(google, repository, namespace: 'so.shep.fixture');
  addTearDown(d.dispose);
  addTearDown(google.dispose);
  await d.discover();
  return (google, d);
}

Future<void> waitFor(bool Function() check) async {
  for (var i = 0; i < 200; i++) {
    if (check()) return;
    await Future<void>.delayed(Duration.zero);
  }
  fail('Publication did not reach the held boundary.');
}

void main() {
  test(
    'a frozen 52-account review pages and retries publication with the same identity',
    () async {
      final repository = FixtureProfileCreation(accountCount: 52)
        ..failUploadOnce = true;
      final (_, d) = await setup(repository);
      final settings = const Preferences().profileSettings();
      await d.prepareCreation('Work', accounts: true, settings: settings);
      final id = d.creation!.id;
      expect(d.creation!.settings, 8);
      expect(d.creationAccounts.length, 50);
      await d.reviewCreationAccounts(first: false);
      expect(d.creationAccounts.length, 2);
      await d.reviewCreationAccounts(first: true);
      expect(d.creationAccounts.length, 50);
      await d.approveCreation(settings);
      expect(d.error, contains('could not be confirmed'));
      expect(d.creation!.phase, 'uploading');
      expect(d.creation!.uploaded, 0);
      expect(d.creation!.id, id);
      await d.resumeCreation();
      expect(d.error, isNull);
      expect(d.creation!.complete, true);
      expect(d.creation!.id, id);
      expect(repository.prepared, 1);
      expect(d.profiles.single.initialized, true);
    },
  );
  test(
    'lost approval retains the accepted publication and can resume instead of duplicating it',
    () async {
      final repository = FixtureProfileCreation()..lostApprovalOnce = true;
      final (_, d) = await setup(repository);
      final settings = const Preferences().profileSettings();
      await d.prepareCreation('Personal', accounts: false, settings: settings);
      await d.approveCreation(settings);
      expect(d.creation!.phase, 'staging');
      expect(d.error, contains('approval'));
      await d.resumeCreation();
      expect(d.creation!.complete, true);
      expect(d.creation!.accounts, 0);
      expect(repository.prepared, 1);
    },
  );
  test(
    'pause finishes only its accepted step and leaves normal discovery usable',
    () async {
      final repository = FixtureProfileCreation()..holdStep = Completer<void>();
      final (_, d) = await setup(repository);
      await d.prepareCreation('Pause', accounts: true, settings: {});
      final pending = d.approveCreation({});
      await waitFor(() => repository.steps == 1);
      d.pause();
      repository.holdStep!.complete();
      await pending;
      expect(repository.steps, 1);
      expect(d.creation!.staged, 1);
      expect(d.creation!.complete, false);
      expect(d.busy, false);
      repository.holdStep = null;
      await d.resumeCreation();
      expect(d.creation!.complete, true);
    },
  );
  test(
    'disconnect fences late publication state and sends no further step',
    () async {
      final repository = FixtureProfileCreation()..holdStep = Completer<void>();
      final (google, d) = await setup(repository);
      await d.prepareCreation('Disconnect', accounts: true, settings: {});
      final pending = d.approveCreation({});
      await waitFor(() => repository.steps == 1);
      await google.disconnect();
      expect(d.creation, isNull);
      expect(d.creationAccounts, isEmpty);
      repository.holdStep!.complete();
      await pending;
      expect(repository.steps, 1);
      expect(d.creation, isNull);
      expect(d.profiles, isEmpty);
      expect(repository.closes, 1);
    },
  );
  test(
    'changed settings reject approval and cancelling keeps the local choices',
    () async {
      final repository = FixtureProfileCreation();
      final (_, d) = await setup(repository);
      final settings = const Preferences().profileSettings();
      await d.prepareCreation('Review', accounts: true, settings: settings);
      await d.approveCreation({...settings, 'preview_lines': 4});
      expect(d.creation!.needsReview, true);
      expect(d.error, contains('Preferences changed'));
      await d.cancelCreation();
      expect(d.creation, isNull);
      expect(repository.steps, 0);
    },
  );
}
