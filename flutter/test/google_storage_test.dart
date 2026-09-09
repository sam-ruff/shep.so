import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/google_native.dart';
import 'package:shep_mobile/model/google_connection.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'secure storage reconciles a lost write reply and preserves old state on refused writes',
    () async {
      const channel = MethodChannel(
        'plugins.it_nomads.com/flutter_secure_storage',
      );
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      final values = <String, String>{};
      bool loseReply = false, refuseWrite = false, refuseRead = false;
      messenger.setMockMethodCallHandler(channel, (call) async {
        final args = call.arguments as Map;
        if (call.method == 'read') {
          if (refuseRead) throw PlatformException(code: 'locked');
          return values[args['key']];
        }
        if (call.method == 'write') {
          if (refuseWrite) throw PlatformException(code: 'locked');
          values[args['key'] as String] = args['value'] as String;
          if (loseReply) {
            loseReply = false;
            throw PlatformException(code: 'lost-reply');
          }
          return null;
        }
        throw StateError('Unexpected storage operation');
      });
      addTearDown(() => messenger.setMockMethodCallHandler(channel, null));
      final store = DeviceGoogleConnectionStore();
      expect((await store.read()).active, isNull);
      const permissions = GooglePermissions(drive: true);
      const saved = GoogleConnectionState(
        requested: permissions,
        active: GoogleConnectionRecord(
          'fixture-subject',
          'alex@example.test',
          permissions,
          'fixture-application',
        ),
      );
      loseReply = true;
      await store.write(saved);
      expect((await store.read()).active!.subject, 'fixture-subject');
      expect(values.values.single, isNot(contains('token')));
      refuseWrite = true;
      await expectLater(
        store.write(const GoogleConnectionState(cleanupPending: true)),
        throwsA(isA<PlatformException>()),
      );
      expect((await store.read()).active!.subject, 'fixture-subject');
      refuseWrite = false;
      await store.write(const GoogleConnectionState(cleanupPending: true));
      expect((await store.read()).cleanupPending, true);
      loseReply = true;
      refuseRead = true;
      await expectLater(
        store.write(saved),
        throwsA(isA<GoogleStorageUnconfirmed>()),
      );
      refuseRead = false;
      expect((await store.read()).active!.subject, 'fixture-subject');
    },
  );
}
