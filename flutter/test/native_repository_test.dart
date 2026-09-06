import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:shep_mobile/src/rust/frb_generated.dart';
import 'package:shep_mobile/data/credentials.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/data/outgoing.dart';
import 'package:shep_mobile/model/mail.dart';

class FixtureCredentials implements CredentialStore {
  final values = <String, List<String>>{};
  int reads = 0;
  bool unavailable = false;
  @override
  Future<String?> read(String account, bool smtp) async {
    reads++;
    if (unavailable) throw StateError('Synthetic locked credential store');
    return values[account]?[smtp ? 1 : 0];
  }

  @override
  Future<void> save(String account, String incoming, String smtp) async {
    values[account] = [incoming, smtp];
  }

  @override
  Future<void> remove(String account) async {
    values.remove(account);
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    final name = Platform.isWindows
        ? 'shep_mobile_native.dll'
        : Platform.isMacOS
        ? 'libshep_mobile_native.dylib'
        : 'libshep_mobile_native.so';
    // FRB's default widget-test loader still looks in Cargo's legacy target/
    // folder. Use the code asset that Flutter's build hook actually bundled.
    await ShepNative.init(
      externalLibrary: ExternalLibrary.open(
        'build/native_assets/${Platform.operatingSystem}/$name',
      ),
    );
  });
  test(
    'IMAP local Sent actions never request credentials and persist',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-local-sent-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final path = '${directory.path}/mail.sqlite3';
      final fixture = await Process.run('python3', [
        '../scripts/clients/android_outbox_fixture.py',
        '--prepare',
        path,
      ]);
      expect(fixture.exitCode, 0, reason: '${fixture.stderr}');
      final credentials = FixtureCredentials()..unavailable = true;
      var repository = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      await repository.initialize();
      await repository.recoverOutgoing(
        'outbox-copy-saved',
        OutgoingAction.checkSent,
      );
      final reads = credentials.reads;
      const id = 'imap-fixture:Sent:local-sent-outbox-copy-saved';
      // The message has not even been displayed: routing comes from its current
      // database identity, not a frontend page cache or an ID string convention.
      await repository.mutate(id, {'starred': true});
      await repository.mutate(id, {'unread': true});
      await repository.mutate(id, {'folder': 'Archive'});
      expect(credentials.reads, reads);
      repository = await NativeRepository.open(path, credentials: credentials);
      await repository.initialize();
      final page = await repository.page(
        folder: 'Archive',
        query: '',
        filter: 'All',
        oldest: false,
        offset: 0,
      );
      expect(page.mail.single.id, id);
      expect(page.mail.single.starred, isTrue);
      expect(page.mail.single.unread, isTrue);
      await repository.mutate(id, {'folder': 'Sent'});
      expect(credentials.reads, reads);
      const remote = 'imap-fixture:Sent:local-sent-provider';
      await expectLater(
        repository.mutate(remote, {'starred': true}),
        throwsA(
          predicate((e) => '$e'.contains('credential store is unavailable')),
        ),
      );
      credentials.unavailable = false;
      await expectLater(
        repository.mutate(remote, {'folder': 'Archive'}),
        throwsA(predicate((e) => '$e'.contains('saved password is missing'))),
      );
      expect(credentials.reads, reads + 2);
      final inbox = await repository.page(
        folder: 'Inbox',
        query: '',
        filter: 'All',
        oldest: false,
        offset: 0,
      );
      expect(inbox.mail.single.id, remote);
      expect(inbox.mail.single.starred, isFalse);
    },
  );
  test(
    'native Sent receipt repair exposes a single grouped copy and both IDs',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-sent-alias-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final path = '${directory.path}/mail.sqlite3';
      final fixture = await Process.run('python3', [
        '../scripts/clients/android_outbox_fixture.py',
        '--prepare',
        path,
      ]);
      expect(fixture.exitCode, 0, reason: '${fixture.stderr}');
      final repository = await NativeRepository.open(
        path,
        credentials: FixtureCredentials(),
      );
      await repository.initialize();
      const provider = 'imap-fixture:Sent Mail:91.4';
      const local = 'imap-fixture:Sent:local-sent-outbox-copy-handover';
      expect((await repository.detail(provider)).id, provider);
      await repository.recoverOutgoing(
        'outbox-copy-handover',
        OutgoingAction.checkSent,
      );
      final page = await repository.page(
        folder: 'Sent',
        query: '',
        filter: 'All',
        oldest: false,
        offset: 0,
      );
      expect(page.mail.single.id, local);
      expect(page.mail.single.folder, 'Sent Mail');
      expect(page.aliases[provider], local);
      expect(page.folderMembership['imap-fixture'], {'Sent Mail'});
      expect((await repository.detail(provider)).id, local);
      expect(
        (await repository.detail(local)).body,
        'Original native copy-handover body.',
      );
      await expectLater(
        repository.mutate(provider, {'starred': true}),
        throwsA(predicate((e) => '$e'.contains('saved password is missing'))),
      );
      expect((await repository.detail(local)).starred, isFalse);
    },
  );
  test(
    'real native bridge keeps draft revisions and discard across profile reopen',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-bridge-test-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final path = '${directory.path}/mail.sqlite3';
      final repository = await NativeRepository.open(
        path,
        credentials: FixtureCredentials(),
      );
      await repository.initialize();
      expect(repository.mailAccounts, isEmpty);
      await repository.saveDraft(
        const Draft(
          id: 'ffi-draft',
          subject: 'Preserved draft',
          body: 'Newest text',
          revision: 4,
        ),
      );
      await repository.saveDraft(
        const Draft(id: 'ffi-draft', body: 'Stale text', revision: 2),
      );
      final reopened = await NativeRepository.open(
        path,
        credentials: FixtureCredentials(),
      );
      await reopened.initialize();
      expect(reopened.savedDrafts.single.body, 'Newest text');
      final page = await reopened.page(
        folder: 'Inbox',
        query: '',
        filter: 'All',
        oldest: false,
        offset: 0,
      );
      expect(page.mail, isEmpty);
      await reopened.discard('ffi-draft', 5);
      await expectLater(
        repository.saveDraft(
          const Draft(id: 'ffi-draft', body: 'Late save', revision: 10),
        ),
        throwsA(predicate((e) => '$e'.contains('discarded'))),
      );
      await repository.initialize();
      expect(repository.savedDrafts, isEmpty);
    },
  );
}
