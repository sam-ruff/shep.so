import 'dart:async';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'support/google_fixture.dart';

const read = GooglePermissions(calendar: GoogleCalendarPermission.read);
const both = GooglePermissions(
  drive: true,
  calendar: GoogleCalendarPermission.edit,
);
const old = GoogleConnectionRecord(
  FixtureGoogleAuthorization.subject,
  FixtureGoogleAuthorization.email,
  read,
  FixtureGoogleAuthorization.application,
);

void main() {
  test(
    'unconfirmed saves require re-reading and preserve later unsaved choices',
    () async {
      final store = MemoryGoogleStore(
        const GoogleConnectionState(requested: read, active: old),
      );
      final sdk = FixtureGoogleAuthorization();
      final c = GoogleConnection(sdk, store);
      addTearDown(c.dispose);
      await c.load();
      store.holdWrite = Completer<void>();
      store.unconfirmedWrite = true;
      c.choose(both);
      await Future<void>.delayed(Duration.zero);
      c.choose(const GooglePermissions(drive: true));
      store.holdWrite!.complete();
      await c.saveChoices();
      expect(c.loaded, false);
      expect(c.error, contains('Could not confirm'));
      await c.connect();
      await c.disconnect();
      expect(sdk.connects, 0);
      expect(sdk.signOuts, 0);
      await expectLater(
        c.accessToken(read.scopes),
        throwsA(isA<GoogleConnectionFailure>()),
      );
      store.unconfirmedWrite = false;
      await c.load();
      expect(c.requested, const GooglePermissions(drive: true));
      await c.saveChoices();
      expect(store.value.requested, const GooglePermissions(drive: true));
      await c.connect();
      expect(
        store.value.active!.permissions,
        const GooglePermissions(drive: true),
      );
      store.unconfirmedWrite = true;
      await c.disconnect();
      expect(c.loaded, false);
      expect(store.value.cleanupPending, true);
      expect(sdk.signOuts, 0);
      store.unconfirmedWrite = false;
      await c.load();
      expect(c.cleanupPending, true);
      expect(c.active, isNull);
      await c.disconnect();
      expect(c.cleanupPending, false);
      expect(sdk.signOuts, 1);
    },
  );
  test(
    'permission matrix is exact, stable and contains no broad Drive/mail scope',
    () {
      for (final drive in [false, true]) {
        for (final calendar in GoogleCalendarPermission.values) {
          final value = GooglePermissions(drive: drive, calendar: calendar);
          final scopes = value.scopes;
          expect(
            scopes.contains('https://www.googleapis.com/auth/drive.appdata'),
            drive,
          );
          expect(
            scopes.any((s) => s.endsWith('calendar.events')),
            calendar == GoogleCalendarPermission.edit,
          );
          expect(
            scopes.any((s) => s.endsWith('calendar.events.readonly')),
            calendar == GoogleCalendarPermission.read,
          );
          expect(
            scopes.any((s) => s.endsWith('calendar.calendarlist.readonly')),
            calendar != GoogleCalendarPermission.off,
          );
          expect(
            scopes.length,
            (drive ? 1 : 0) +
                (calendar == GoogleCalendarPermission.off ? 0 : 2),
          );
          expect(GooglePermissions.fromJson(value.toJson()), value);
        }
      }
    },
  );
  test(
    'device record contains no token and damaged/future state never resets it',
    () {
      const state = GoogleConnectionState(requested: both, active: old);
      final encoded = state.encode();
      expect(encoded, isNot(contains('token')));
      expect(
        GoogleConnectionState.decode(encoded).active!.subject,
        old.subject,
      );
      for (final source in [
        '{}',
        '[]',
        'not json',
        jsonEncode({'version': 2}),
        encoded.replaceFirst('"calendar":"edit"', '"calendar":"future"'),
        encoded.replaceFirst('"subject":"fixture-google-user"', '"subject":""'),
      ]) {
        expect(() => GoogleConnectionState.decode(source), throwsA(anything));
      }
    },
  );
  test(
    'locked startup forbids overwrite and retries the original saved connection',
    () async {
      final store = MemoryGoogleStore(const GoogleConnectionState(active: old))
        ..failRead = true;
      final sdk = FixtureGoogleAuthorization();
      final c = GoogleConnection(sdk, store);
      addTearDown(c.dispose);
      await c.load();
      c.choose(both);
      await c.connect();
      await c.disconnect();
      expect(c.loaded, false);
      expect(sdk.connects, 0);
      expect(store.writes, 0);
      store.failRead = false;
      await c.load();
      expect(c.active!.subject, old.subject);
    },
  );
  test(
    'held consent retains active access and newer choices reject its completion',
    () async {
      final store = MemoryGoogleStore(
        const GoogleConnectionState(requested: read, active: old),
      );
      final sdk = FixtureGoogleAuthorization()..hold = Completer<void>();
      final c = GoogleConnection(sdk, store);
      addTearDown(c.dispose);
      await c.load();
      c.choose(both);
      await c.saveChoices();
      final login = c.connect();
      await Future<void>.delayed(Duration.zero);
      expect(sdk.connects, 1);
      expect(c.active!.permissions, read);
      await c.connect();
      expect(sdk.connects, 1);
      c.choose(const GooglePermissions(drive: true));
      await c.saveChoices();
      sdk.hold!.complete();
      await login;
      expect(c.error, contains('changed'));
      expect(store.value.active!.permissions, read);
      expect(store.value.requested, const GooglePermissions(drive: true));
    },
  );
  test(
    'denial, wrong identity/application and failed commit preserve old connection',
    () async {
      final store = MemoryGoogleStore(
        const GoogleConnectionState(requested: both, active: old),
      );
      final sdk = FixtureGoogleAuthorization()..failure = 'Consent denied';
      final c = GoogleConnection(sdk, store);
      addTearDown(c.dispose);
      await c.load();
      await c.connect();
      expect(c.error, 'Consent denied');
      sdk.failure = null;
      sdk.returnedSubject = 'someone-else';
      await c.connect();
      expect(c.error, contains('different'));
      sdk.returnedSubject = old.subject;
      sdk.returnedApplication = 'other-project';
      await c.connect();
      expect(c.error, contains('different'));
      sdk.returnedApplication = old.application;
      store.failWrite = true;
      await c.connect();
      expect(c.active!.permissions, read);
      expect(store.value.active!.permissions, read);
      store.failWrite = false;
      await c.connect();
      expect(c.active!.permissions, both);
      final reopened = GoogleConnection(sdk, store);
      addTearDown(reopened.dispose);
      await reopened.load();
      expect(reopened.active!.permissions, both);
    },
  );
  test(
    'preference writer is bounded/coalesces and commit waits for saved choices',
    () async {
      final store = MemoryGoogleStore()..holdWrite = Completer<void>();
      final sdk = FixtureGoogleAuthorization();
      final c = GoogleConnection(sdk, store);
      addTearDown(c.dispose);
      await c.load();
      c.choose(read);
      await Future<void>.delayed(Duration.zero);
      for (var i = 0; i < 100; i++) {
        c.choose(i.isEven ? both : read);
      }
      c.choose(both);
      final login = c.connect();
      expect(sdk.connects, 0);
      store.holdWrite!.complete();
      await login;
      expect(store.maxWrites, 1);
      expect(store.writes, lessThanOrEqualTo(3));
      expect(c.active!.permissions, both);
    },
  );
  test(
    'disconnect commits before cleanup and restart retains retry without reconnecting',
    () async {
      final store = MemoryGoogleStore(
        const GoogleConnectionState(requested: read, active: old),
      );
      final sdk = FixtureGoogleAuthorization()..failSignOut = true;
      final c = GoogleConnection(sdk, store);
      addTearDown(c.dispose);
      await c.load();
      store.failWrite = true;
      await c.disconnect();
      expect(sdk.signOuts, 0);
      expect(c.active, isNotNull);
      store.failWrite = false;
      await c.disconnect();
      expect(c.active, isNull);
      expect(c.cleanupPending, true);
      final reopened = GoogleConnection(sdk, store);
      addTearDown(reopened.dispose);
      await reopened.load();
      await reopened.connect();
      expect(sdk.connects, 0);
      sdk.failSignOut = false;
      await reopened.disconnect();
      expect(reopened.cleanupPending, false);
      expect(store.value.active, isNull);
    },
  );
  test(
    'provider token is enabled-service scoped and cannot outlive controller ownership',
    () async {
      final store = MemoryGoogleStore(
        const GoogleConnectionState(requested: both, active: old),
      );
      final sdk = FixtureGoogleAuthorization()..hold = Completer<void>();
      final c = GoogleConnection(sdk, store);
      await c.load();
      await expectLater(
        c.accessToken(both.scopes),
        throwsA(isA<GoogleConnectionFailure>()),
      );
      expect(sdk.tokens, 0);
      final token = c.accessToken(read.scopes);
      final failed = expectLater(
        token,
        throwsA(isA<GoogleConnectionFailure>()),
      );
      await c.connect();
      expect(sdk.connects, 0);
      c.dispose();
      sdk.hold!.complete();
      await failed;
    },
  );
  test(
    'disposing held authorization cannot commit a connection after closing',
    () async {
      final store = MemoryGoogleStore();
      final sdk = FixtureGoogleAuthorization()..hold = Completer<void>();
      final c = GoogleConnection(sdk, store);
      await c.load();
      final login = c.connect();
      await Future<void>.delayed(Duration.zero);
      c.dispose();
      sdk.hold!.complete();
      await login;
      expect(store.value.active, isNull);
    },
  );
}
