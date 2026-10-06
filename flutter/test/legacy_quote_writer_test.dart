import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:shep_mobile/src/rust/api.dart';
import 'package:shep_mobile/src/rust/frb_generated.dart';

void main() {
  const library = String.fromEnvironment('SHEP_LEGACY_QUOTE_LIBRARY');
  test(
    'actual schema25 writer loses unfenced context and refuses schema26 unchanged',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-old-quote-writer-',
      );
      addTearDown(() => directory.delete(recursive: true));
      Future<Map<String, dynamic>> fixture(String path, {int? version}) async {
        final result = await Process.run('python3', [
          '../scripts/clients/legacy_reply_fixture.py',
          path,
          if (version != null) ...['--prepare', '$version'],
        ]);
        expect(result.exitCode, 0, reason: '${result.stderr}');
        return jsonDecode(result.stdout as String) as Map<String, dynamic>;
      }

      await ShepNative.init(externalLibrary: ExternalLibrary.open(library));
      final unprotected = '${directory.path}/unprotected.sqlite3';
      final original = await fixture(unprotected, version: 25);
      final profile = await MobileProfile.open(path: unprotected);
      final legacyInput =
          jsonDecode(original['drafts']['modern-reply'] as String)
                as Map<String, dynamic>
            ..remove('reply_context')
            ..['revision'] = 2
            ..['body'] = 'Older writer edited the typed answer';
      final saved =
          jsonDecode(
                await profile.request(
                  json: jsonEncode({'op': 'save_draft', 'draft': legacyInput}),
                ),
              )
              as Map;
      expect(saved['error'], isNull);
      profile.dispose();
      final lost = await fixture(unprotected);
      expect(
        (jsonDecode(lost['drafts']['modern-reply'] as String)
            as Map)['reply_context'],
        isNull,
      );
      expect(lost['files'], original['files']);
      final protected = '${directory.path}/protected.sqlite3';
      final before = await fixture(protected, version: 26);
      await expectLater(
        MobileProfile.open(path: protected),
        throwsA(predicate((error) => '$error'.contains('newer Shep version'))),
      );
      expect(await fixture(protected), before);
    },
    skip: library.isEmpty
        ? 'Run the saved compatibility probe with a verified schema25 library.'
        : false,
  );
}
