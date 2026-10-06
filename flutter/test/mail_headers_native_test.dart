import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
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
    'actual native metadata keeps recipient aliases and exact From independently of account identity',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-headers-native-',
      );
      final path = '${directory.path}/mail.sqlite3';
      final seeded = await Process.run('python3', [
        '../scripts/clients/android_incoming_fixture.py',
        '--prepare',
        path,
      ]);
      expect(seeded.exitCode, 0, reason: '${seeded.stderr}');
      const sender = r' "Help <desk>" <Sender@Example.TEST> ';
      const recipient =
          'Alias <alias@example.test>, "Café Team" <team@example.test>';
      const subject = 'Café ガ 한글';
      final changed = await Process.run('python3', [
        '-c',
        'import json,sqlite3,sys\n'
            'with sqlite3.connect(sys.argv[1]) as db:\n'
            ' db.execute("UPDATE mail SET sender=?,recipient=?,subject=?",json.loads(sys.argv[2]))\n',
        path,
        jsonEncode([sender, recipient, subject]),
      ]);
      expect(changed.exitCode, 0, reason: '${changed.stderr}');
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
      final page = await repository.page(
        folder: 'Inbox',
        query: '',
        filter: '',
        oldest: false,
        offset: 0,
      );
      expect(page.mail, isNotEmpty);
      final metadata = page.mail.first;
      expect(metadata.recipient, recipient);
      expect(metadata.senderHeader, sender);
      expect(metadata.address, 'Sender@Example.TEST');
      expect(metadata.subject, subject);
      expect(metadata.recipient, isNot(metadata.account));
      expect(metadata.bodyLoaded, isFalse);
      final detail = await repository.detail(metadata.id);
      expect(detail.recipient, recipient);
      expect(detail.senderHeader, sender);
      expect(detail.address, 'Sender@Example.TEST');
      expect(metadata.withDetail(detail).recipient, recipient);
      expect(credentials.reads, 0);
    },
  );
}
