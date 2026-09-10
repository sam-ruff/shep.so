import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'support/google_fixture.dart';

void main() {
  test(
    'a denied permission expansion retains usable Drive identity binding',
    () async {
      final sdk = FixtureGoogleAuthorization()..failEditingOnce = true;
      final g = GoogleConnection(sdk, MemoryGoogleStore());
      addTearDown(g.dispose);
      await g.load();
      g.choose(const GooglePermissions(drive: true));
      await g.connect();
      g.choose(
        const GooglePermissions(
          drive: true,
          calendar: GoogleCalendarPermission.edit,
        ),
      );
      await g.connect();
      expect(g.error, contains('cancelled'));
      expect(g.active!.permissions.calendar, GoogleCalendarPermission.off);
      await g.bindDrivePrincipal(g.grantGeneration, 'drive:verified-owner');
      expect(g.active!.drivePrincipal, 'drive:verified-owner');
      expect(g.error, contains('cancelled'));
    },
  );

  test(
    'verified Drive principal survives re-consent but cannot switch identity',
    () async {
      final g = GoogleConnection(
        FixtureGoogleAuthorization(),
        MemoryGoogleStore(),
      );
      addTearDown(g.dispose);
      await g.load();
      g.choose(const GooglePermissions(drive: true));
      await g.connect();
      final generation = g.grantGeneration;
      await g.bindDrivePrincipal(generation, 'drive:verified-owner');
      g.choose(
        const GooglePermissions(
          drive: true,
          calendar: GoogleCalendarPermission.read,
        ),
      );
      await g.connect();
      expect(g.active!.drivePrincipal, 'drive:verified-owner');
      await expectLater(
        g.bindDrivePrincipal(generation, 'drive:verified-owner'),
        throwsA(isA<GoogleConnectionFailure>()),
      );
      await expectLater(
        g.bindDrivePrincipal(g.grantGeneration, 'drive:wrong-owner'),
        throwsA(isA<GoogleConnectionFailure>()),
      );
      expect(g.active!.drivePrincipal, 'drive:verified-owner');
    },
  );
  test(
    'saved principal validates exact bounded provider identity while old metadata remains readable',
    () {
      final record = const GoogleConnectionRecord(
        'fixture',
        'fixture@example.test',
        GooglePermissions(drive: true),
        'fixture-app',
      ).toJson();
      expect(GoogleConnectionRecord.fromJson(record).drivePrincipal, isNull);
      for (final invalid in [
        42,
        '',
        'drive:',
        'drive:bad owner',
        'drive:${'a' * 201}',
        'mail:owner',
      ]) {
        expect(
          () => GoogleConnectionRecord.fromJson({
            ...record,
            'drive_principal': invalid,
          }),
          throwsFormatException,
        );
      }
      expect(
        GoogleConnectionRecord.fromJson({
          ...record,
          'drive_principal': 'drive:owner_12-AB',
        }).drivePrincipal,
        'drive:owner_12-AB',
      );
    },
  );
}
