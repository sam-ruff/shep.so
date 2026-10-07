import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/src/rust/frb_generated.dart';
import 'native_repository_test.dart' show FixtureCredentials;
import 'workspace_test.dart' show MemorySettings;

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
    'actual native drafts follow the shared mailto cases and survive reopening beside an existing draft',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-mailto-native-',
      );
      final path = '${directory.path}/mail.sqlite3';
      final credentials = FixtureCredentials()..unavailable = true;
      var repository = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      addTearDown(() async {
        repository.profile.dispose();
        await directory.delete(recursive: true);
      });
      await repository.initialize();
      const existing = Draft(
        id: 'existing-draft',
        to: 'kept@example.test',
        body: 'Unsent edit',
        revision: 2,
      );
      await repository.saveDraft(existing);
      final cases =
          (jsonDecode(File('../shared/mailto-cases.json').readAsStringSync())
                  as Map)['cases']
              as List;
      final created = <String, Map>{};
      for (final value in cases.cast<Map>()) {
        final id = newDraftIdentity();
        final message = value['mode'] == 'message';
        if (value.containsKey('rejected')) {
          await expectLater(
            repository.mailtoDraft(
              id,
              value['link'] as String,
              accountId: '',
              message: message,
            ),
            throwsA(isA<MailOperationFailure>()),
            reason: value['name'] as String,
          );
          continue;
        }
        final draft = await repository.mailtoDraft(
          id,
          value['link'] as String,
          accountId: '',
          message: message,
        );
        final expected = value['expected'] as Map;
        expect(
          [draft.to, draft.cc, draft.bcc, draft.subject, draft.body],
          [
            for (final key in ['to', 'cc', 'bcc', 'subject', 'body'])
              expected[key] ?? '',
          ],
          reason: value['name'] as String,
        );
        expect(draft.id, id);
        expect(draft.revision, 0);
        expect(draft.attachments, isEmpty);
        created[id] = expected;
      }
      final long = 'mailto:${'a' * 8192}@example.test';
      await expectLater(
        repository.mailtoDraft(
          newDraftIdentity(),
          long,
          accountId: '',
          message: false,
        ),
        throwsA(
          isA<MailOperationFailure>().having(
            (e) => e.message,
            'message',
            contains('too long'),
          ),
        ),
      );
      repository.profile.dispose();
      repository = await NativeRepository.open(path, credentials: credentials);
      await repository.initialize();
      final saved = {for (final d in repository.savedDrafts) d.id: d};
      expect(saved.keys.toSet(), {'existing-draft', ...created.keys});
      expect(saved['existing-draft']?.body, 'Unsent edit');
      expect(saved['existing-draft']?.revision, 2);
      for (final MapEntry(key: id, value: expected) in created.entries) {
        expect(saved[id]?.to, expected['to'] ?? '');
        expect(saved[id]?.subject, expected['subject'] ?? '');
      }
      expect(await repository.delivery(created.keys.first), isNull);
      expect(credentials.reads, 0, reason: 'opening a link needs no password');
    },
  );

  test(
    'a workspace without accounts keeps the native draft unassigned',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-mailto-workspace-',
      );
      final repository = await NativeRepository.open(
        '${directory.path}/mail.sqlite3',
        credentials: FixtureCredentials()..unavailable = true,
      );
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(() async {
        workspace.dispose();
        repository.profile.dispose();
        await directory.delete(recursive: true);
      });
      await workspace.initialize();
      final draft = await workspace.openMailto(
        'mailto:friend%40example.test?subject=Ignored&bcc=hidden@example.test',
        message: true,
      );
      expect(draft.accountId, '');
      expect(draft.to, 'friend@example.test');
      expect(draft.subject, '');
      expect(draft.bcc, '');
      expect(workspace.drafts[draft.id]?.to, 'friend@example.test');
      await expectLater(
        workspace.openMailto('mailto:a@example.test?subject=caf%E9'),
        throwsA(
          isA<MailOperationFailure>().having(
            (e) => e.message,
            'message',
            contains('invalid encoded text'),
          ),
        ),
      );
      expect(workspace.drafts, hasLength(1));
    },
  );
}
