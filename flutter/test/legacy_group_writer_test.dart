import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:shep_mobile/src/rust/api.dart';
import 'package:shep_mobile/src/rust/frb_generated.dart';

void main() {
  const library = String.fromEnvironment('SHEP_SCHEMA26_GROUP_LIBRARY');
  test(
    'actual schema26 writer opens26 and refuses group schema27 unchanged',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-old-group-writer-',
      );
      addTearDown(() => directory.delete(recursive: true));
      Future<Map<String, dynamic>> fixture(String path, {int? version}) async {
        final result = await Process.run('python3', [
          '../scripts/clients/legacy_reply_fixture.py',
          path,
          if (version != null) ...['--prepare', '$version'],
          if (version == 27) '--groups',
        ]);
        expect(result.exitCode, 0, reason: '${result.stderr}');
        return jsonDecode(result.stdout as String) as Map<String, dynamic>;
      }

      await ShepNative.init(externalLibrary: ExternalLibrary.open(library));
      final baseline = '${directory.path}/schema26.sqlite';
      final before = await fixture(baseline, version: 26);
      final profile = await MobileProfile.open(path: baseline);
      profile.dispose();
      expect(await fixture(baseline), before);
      final protected = '${directory.path}/schema27.sqlite';
      final exact = await fixture(protected, version: 27);
      await expectLater(
        MobileProfile.open(path: protected),
        throwsA(predicate((error) => '$error'.contains('newer Shep version'))),
      );
      expect(await fixture(protected), exact);
    },
    skip: library.isEmpty
        ? 'Run the saved compatibility probe with a verified schema26 library.'
        : false,
  );
}
