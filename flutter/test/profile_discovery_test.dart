import 'dart:async';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/profile_discovery.dart';
import 'support/google_fixture.dart';
import 'support/profile_discovery_fixture.dart';

const permissions = GooglePermissions(drive: true);
const account = GoogleConnectionRecord(
  FixtureGoogleAuthorization.subject,
  FixtureGoogleAuthorization.email,
  permissions,
  FixtureGoogleAuthorization.application,
);
Future<void> until(bool Function() condition) async {
  for (var i = 0; i < 200; i++) {
    if (condition()) return;
    await Future<void>.delayed(const Duration(milliseconds: 1));
  }
  fail('The expected fixture boundary was not reached.');
}

Future<
  (
    GoogleConnection,
    MemoryGoogleStore,
    ProfileDiscovery,
    FixtureProfileDiscovery,
  )
>
setup({int count = 2}) async {
  final store = MemoryGoogleStore(
    const GoogleConnectionState(requested: permissions, active: account),
  );
  final google = GoogleConnection(FixtureGoogleAuthorization(), store);
  await google.load();
  final repository = FixtureProfileDiscovery(count: count);
  final discovery = ProfileDiscovery(
    google,
    repository,
    namespace: 'so.shep.fixture',
  );
  addTearDown(() {
    discovery.dispose();
    google.dispose();
  });
  return (google, store, discovery, repository);
}

void main() {
  test(
    'discovery binds the saved principal, pages 50 summaries and refreshes with a fresh session',
    () async {
      final (google, store, d, r) = await setup(count: 52);
      await d.discover();
      expect(d.state!.complete, true);
      expect(d.profiles.length, 50);
      expect(
        store.value.active!.drivePrincipal,
        FixtureProfileDiscovery.principal,
      );
      expect(google.active!.drivePrincipal, FixtureProfileDiscovery.principal);
      final id = r.active;
      await d.page(first: false);
      expect(d.profiles.length, 2);
      expect(d.profiles.first.name, 'Profile 51');
      await d.page(first: true);
      expect(d.profiles.first.name, 'Personal');
      await d.discover();
      expect(r.active, isNot(id));
      expect(r.closes, 1);
      expect(r.opens, 2);
      expect(r.refreshes, 1);
      expect(d.state!.complete, true);
    },
  );
  test(
    'failed identity saves cannot read profile contents or advance discovery',
    () async {
      final (_, store, d, r) = await setup();
      store.failWrite = true;
      await d.discover();
      expect(d.error, contains('Could not save the Drive identity'));
      expect(d.state, isNull);
      expect(d.profiles, isEmpty);
      expect(r.advances, 0);
      expect(r.reads, 0);
      store.failWrite = false;
      await d.discover();
      expect(d.state!.complete, true);
      expect(r.closes, 1);
    },
  );
  test(
    'unconfirmed identity save pauses Google until its committed state is read again',
    () async {
      final (g, store, d, r) = await setup();
      store.unconfirmedWrite = true;
      await d.discover();
      expect(g.loaded, false);
      expect(r.advances, 0);
      expect(r.active, isNull);
      expect(d.state, isNull);
      store.unconfirmedWrite = false;
      await g.load();
      await d.discover();
      expect(d.state!.complete, true);
    },
  );
  test(
    'pause finishes one accepted step; retry resumes a saved failure',
    () async {
      final (_, _, d, r) = await setup();
      r.holdAdvance = Completer<void>();
      final pending = d.discover();
      await until(() => r.advances == 1);
      expect(identical(d.discover(), pending), true);
      d.pause();
      r.holdAdvance!.complete();
      await pending;
      expect(r.advances, 1);
      expect(d.paused, true);
      expect(d.state!.complete, false);
      r.failNext = true;
      await d.discover();
      expect(d.error, contains('could not be read'));
      expect(d.state!.error, isNotNull);
      await d.discover();
      expect(r.retries, 1);
      expect(d.error, isNull);
      expect(d.state!.complete, true);
    },
  );
  for (final fail in [false, true]) {
    test(
      'disconnect fences held ${fail ? 'errors' : 'data'} and stops subsequent requests',
      () async {
        final (g, _, d, r) = await setup();
        r.holdAdvance = Completer<void>();
        r.failNext = fail;
        final pending = d.discover();
        await until(() => r.advances == 1);
        await g.disconnect();
        expect(d.state, isNull);
        expect(d.profiles, isEmpty);
        r.holdAdvance!.complete();
        await pending;
        expect(r.advances, 1);
        expect(r.active, isNull);
        expect(d.state, isNull);
        expect(d.error, isNull);
      },
    );
  }
  test(
    'changed consent fences a held refresh as well as individual steps',
    () async {
      final (g, _, d, r) = await setup();
      await d.discover();
      r.holdRefresh = Completer<void>();
      final pending = d.discover();
      await until(() => r.refreshes == 1);
      g.choose(
        const GooglePermissions(calendar: GoogleCalendarPermission.read),
      );
      await g.connect();
      expect(g.active!.permissions.drive, false);
      expect(d.state, isNull);
      r.holdRefresh!.complete();
      await pending;
      expect(d.state, isNull);
      expect(d.profiles, isEmpty);
      expect(r.active, isNull);
    },
  );
  test(
    'disposing during open still owns the late session through cleanup',
    () async {
      final g = GoogleConnection(
        FixtureGoogleAuthorization(),
        MemoryGoogleStore(
          const GoogleConnectionState(requested: permissions, active: account),
        ),
      );
      await g.load();
      final r = FixtureProfileDiscovery()..holdOpen = Completer<void>();
      final d = ProfileDiscovery(g, r, namespace: 'so.shep.fixture');
      final pending = d.discover();
      await until(() => r.opens == 1);
      d.dispose();
      g.dispose();
      r.holdOpen!.complete();
      await pending;
      expect(r.active, isNull);
      expect(r.advances, 0);
    },
  );
  test(
    'wrong session and failed close cannot start another owner or display unbound contents',
    () async {
      final (_, _, d, r) = await setup();
      r.wrongSession = true;
      await d.discover();
      expect(d.error, contains('different session'));
      expect(r.reads, 0);
      r.wrongSession = false;
      r.failClose = true;
      await d.discover();
      expect(r.opens, 1);
      expect(d.error, contains('Could not close'));
      r.failClose = false;
      await d.discover();
      expect(d.state!.complete, true);
      expect(r.opens, 2);
    },
  );
}
