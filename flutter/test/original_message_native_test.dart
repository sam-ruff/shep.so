import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/src/rust/frb_generated.dart';
import 'native_repository_test.dart' show FixtureCredentials;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    final name = Platform.isWindows
        ? 'shep_mobile_native.dll'
        : Platform.isMacOS
        ? 'libshep_mobile_native.dylib'
        : 'libshep_mobile_native.so';
    await ShepNative.init(
      externalLibrary: ExternalLibrary.open(
        'build/native_assets/${Platform.operatingSystem}/$name',
      ),
    );
  });

  test(
    'actual native Save original message returns the cached MIME byte for byte',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-original-native-',
      );
      final path = '${directory.path}/mail.sqlite3';
      final seeded = await Process.run('python3', [
        '../scripts/clients/android_incoming_fixture.py',
        '--prepare',
        path,
      ]);
      expect(seeded.exitCode, 0, reason: '${seeded.stderr}');
      final read = await Process.run('python3', [
        '-c',
        'import sqlite3,sys\n'
            'with sqlite3.connect(sys.argv[1]) as db:\n'
            ' db.execute("INSERT INTO mail_aliases VALUES(\'older-identity\',\'fixture:INBOX:files\')")\n'
            ' print(db.execute("SELECT hex(raw) FROM mail WHERE id=\'fixture:INBOX:files\'").fetchone()[0])\n',
        path,
      ]);
      expect(read.exitCode, 0, reason: '${read.stderr}');
      final hex = (read.stdout as String).trim();
      final expected = [
        for (var i = 0; i < hex.length; i += 2)
          int.parse(hex.substring(i, i + 2), radix: 16),
      ];
      expect(expected, contains(0x0D), reason: 'the fixture keeps CRLF');
      final credentials = FixtureCredentials()..unavailable = true;
      final repository = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      addTearDown(() async {
        repository.profile.dispose();
        await directory.delete(recursive: true);
      });
      await repository.initialize();
      expect(await repository.originalMessage('fixture:INBOX:files'), expected);
      expect(await repository.originalMessage('older-identity'), expected);
      await expectLater(
        repository.originalMessage('not-cached'),
        throwsA(
          isA<MailOperationFailure>().having(
            (e) => e.message,
            'message',
            contains('no longer cached'),
          ),
        ),
      );
      expect(credentials.reads, 0, reason: 'saving needs no password');
    },
  );
}
