import 'dart:io';
import 'dart:convert';
import 'dart:async';
import 'support/connection_repository.dart';
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
  bool unavailable = false,
      removeUnavailable = false,
      saveUnavailable = false,
      saveResponseLost = false;
  Completer<void>? saveGate, saveStarted;
  @override
  Future<String?> read(String account, bool smtp) async {
    reads++;
    if (unavailable) throw StateError('Synthetic locked credential store');
    return values[account]?[smtp ? 1 : 0];
  }

  @override
  Future<void> save(String account, String incoming, String smtp) async {
    saveStarted?.complete();
    if (saveGate != null) await saveGate!.future;
    if (saveUnavailable) throw StateError('Synthetic locked credential write');
    values[account] = [incoming, smtp];
    if (saveResponseLost) {
      throw StateError('Synthetic lost credential save acknowledgment');
    }
  }

  @override
  Future<void> remove(String account) async {
    if (removeUnavailable) throw StateError('Synthetic locked cleanup');
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
  Future<ConnectionFixtureRepository> connection(
    FixtureCredentials credentials,
  ) async {
    final directory = await Directory.systemTemp.createTemp(
      'shep-connection-host-',
    );
    addTearDown(() => directory.delete(recursive: true));
    final path = '${directory.path}/mail.sqlite';
    final seeded = await Process.run('python3', [
      '../scripts/clients/android_incoming_fixture.py',
      '--prepare',
      path,
    ]);
    expect(seeded.exitCode, 0, reason: '${seeded.stderr}');
    final native = await NativeRepository.open(path, credentials: credentials);
    final repository = ConnectionFixtureRepository(native.profile, credentials);
    await repository.initialize();
    credentials.values[repository.mailAccounts.single.id] = [
      'prior-incoming',
      'prior-smtp',
    ];
    return repository;
  }

  test(
    'native Rust search shares exact UTF-16 ranges without opening credentials',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-find-host-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final credentials = FixtureCredentials()..unavailable = true;
      final repository = await NativeRepository.open(
        '${directory.path}/mail.sqlite',
        credentials: credentials,
      );
      final cases =
          jsonDecode(await File('../shared/find-cases.json').readAsString())
              as List;
      for (final c in cases) {
        expect(
          (await repository.findText(
            List<String>.from(c['blocks']),
            c['query'],
            c['match_case'],
          )).map((h) => h.toJson()).toList(),
          c['hits'],
        );
      }
      expect(credentials.reads, 0);
    },
  );
  test(
    'SMTP without authentication can prepare a bound request while device credentials are locked',
    () async {
      final credentials = FixtureCredentials();
      final repository = await connection(credentials);
      final account = repository.mailAccounts.single;
      await repository.call({
        'op': 'save_account',
        'account': {
          ...account.toJson(),
          'smtp_auth': 'None',
          'sent_copy': 'LocalOnly',
        },
      });
      await repository.initialize();
      repository.captureSend = true;
      credentials.unavailable = true;
      final before = credentials.reads;
      await expectLater(
        repository.send(
          Draft(
            id: 'no-auth-draft',
            accountId: account.id,
            to: 'recipient@example.test',
            body: 'Synthetic no-auth draft',
            revision: 1,
          ),
        ),
        throwsStateError,
      );
      expect(credentials.reads, before);
      expect(repository.submitted!['password'], '');
      expect(repository.submitted!['credential_slot'], account.id);
      expect(repository.submitted!['incoming_password'], isNull);
      expect(await repository.delivery('no-auth-draft'), isNull);
      expect(repository.submitted!['id'], 'no-auth-draft');
    },
  );
  test(
    'legacy implicit STARTTLS retains its effective connection and saved credentials',
    () async {
      final credentials = FixtureCredentials();
      final repository = await connection(credentials);
      final account = repository.mailAccounts.single;
      await repository.call({
        'op': 'save_account',
        'account': {
          ...account.toJson(),
          'smtp_port': 587,
          'smtp_security': null,
        },
      });
      await repository.initialize();
      final legacy = repository.mailAccounts.single;
      expect(legacy.smtpSecurity, 'StartTls');
      expect(await repository.password(legacy, smtp: true), 'prior-smtp');
      await repository.connect(legacy, 'candidate-incoming', 'candidate-smtp');
      expect(await repository.password(legacy, smtp: true), 'candidate-smtp');
    },
  );
  test(
    'connection activation failure preserves the old pair and successful retry cleans only unused keys',
    () async {
      final credentials = FixtureCredentials();
      final repository = await connection(credentials);
      final account = repository.mailAccounts.single;
      credentials.saveUnavailable = true;
      await expectLater(
        repository.connect(account, 'candidate-incoming', 'candidate-smtp'),
        throwsA(predicate((e) => '$e'.contains('previous connection'))),
      );
      credentials.saveUnavailable = false;
      credentials.saveResponseLost = true;
      await expectLater(
        repository.connect(
          account,
          'unacknowledged-incoming',
          'unacknowledged-smtp',
        ),
        throwsA(predicate((e) => '$e'.contains('previous connection'))),
      );
      expect(credentials.values, {
        account.id: ['prior-incoming', 'prior-smtp'],
      });
      credentials.saveResponseLost = false;
      repository.refuseActivation = true;
      await expectLater(
        repository.connect(account, 'candidate-incoming', 'candidate-smtp'),
        throwsStateError,
      );
      expect(credentials.values, {
        account.id: ['prior-incoming', 'prior-smtp'],
      });
      expect(await repository.password(account), 'prior-incoming');
      repository.refuseActivation = false;
      credentials.removeUnavailable = true;
      await repository.connect(account, 'candidate-incoming', 'candidate-smtp');
      expect(await repository.password(account), 'candidate-incoming');
      expect(await repository.password(account, smtp: true), 'candidate-smtp');
      expect(repository.pendingCredentialCleanup, 1);
      final reopened = NativeRepository(repository.profile, credentials);
      await reopened.initialize();
      expect(await reopened.password(account), 'candidate-incoming');
      expect(reopened.pendingCredentialCleanup, 1);
      credentials.removeUnavailable = false;
      await reopened.cleanupCredentials();
      expect(credentials.values, hasLength(1));
      expect(credentials.values.containsKey(account.id), false);
      expect(await reopened.password(account), 'candidate-incoming');
    },
  );
  test(
    'lost activation acknowledgment preserves the new pair and queued cleanup cannot delete a staged write',
    () async {
      final credentials = FixtureCredentials();
      final repository = await connection(credentials);
      final account = repository.mailAccounts.single;
      credentials.saveGate = Completer<void>();
      credentials.saveStarted = Completer<void>();
      final connect = repository.connect(
        account,
        'candidate-incoming',
        'candidate-smtp',
      );
      await credentials.saveStarted!.future;
      final other = NativeRepository(repository.profile, credentials);
      var cleanupFinished = false;
      final cleanup = other.cleanupCredentials().then((_) {
        cleanupFinished = true;
      });
      expect(await repository.password(account), 'prior-incoming');
      expect(cleanupFinished, false);
      credentials.saveGate!.complete();
      await connect;
      await cleanup;
      expect(await repository.password(account), 'candidate-incoming');
      credentials.saveGate = null;
      credentials.saveStarted = null;
      repository.loseActivationResponse = true;
      await expectLater(
        repository.connect(account, 'latest-incoming', 'latest-smtp'),
        throwsStateError,
      );
      expect(credentials.values, hasLength(1));
      await other.initialize();
      expect(await other.password(account), 'latest-incoming');
      expect(await other.password(account, smtp: true), 'latest-smtp');
    },
  );
  test(
    'abandoned staged credentials clean up on reopening without changing the active connection',
    () async {
      final credentials = FixtureCredentials();
      final repository = await connection(credentials);
      final account = repository.mailAccounts.single;
      final pending = await repository.call({
        'op': 'prepare_account',
        'account': account.toJson(),
        'expected': account.toJson(),
      });
      await credentials.save(
        pending['slot'],
        'abandoned-incoming',
        'abandoned-smtp',
      );
      final reopened = NativeRepository(repository.profile, credentials);
      await reopened.initialize();
      expect(reopened.pendingCredentialCleanup, 1);
      expect(await reopened.password(account), 'prior-incoming');
      await reopened.cleanupCredentials();
      expect(credentials.values, {
        account.id: ['prior-incoming', 'prior-smtp'],
      });
      await expectLater(
        repository.call({'op': 'activate_account', 'slot': pending['slot']}),
        throwsA(predicate((e) => '$e'.contains('no longer available'))),
      );
    },
  );
  test(
    'reviewed account removal survives locked cleanup and stale reconnect',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-removal-host-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final path = '${directory.path}/mail.sqlite';
      final seeded = await Process.run('python3', [
        '../scripts/clients/android_incoming_fixture.py',
        '--prepare',
        path,
      ]);
      expect(seeded.exitCode, 0, reason: '${seeded.stderr}');
      final credentials = FixtureCredentials()..removeUnavailable = true;
      final repository = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      await repository.initialize();
      final original = repository.mailAccounts.single;
      credentials.values[original.id] = ['fixture-secret', 'fixture-secret'];
      final review = await repository.removalPreview(original.id);
      expect(review.count('messages'), 1);
      await repository.removeAccount(review, false);
      expect(repository.mailAccounts, isEmpty);
      expect(repository.pendingCredentialCleanup, 1);
      expect(credentials.values, contains(original.id));
      await expectLater(
        repository.connect(original, 'new', 'new'),
        throwsA(predicate((e) => '$e'.contains('removed'))),
      );
      expect(credentials.values[original.id], [
        'fixture-secret',
        'fixture-secret',
      ]);
      final reopened = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      await reopened.initialize();
      expect(reopened.pendingCredentialCleanup, 1);
      credentials.removeUnavailable = false;
      await reopened.cleanupCredentials();
      expect(credentials.values, isEmpty);
      expect(reopened.pendingCredentialCleanup, 0);
    },
  );
  test(
    'incoming attachment metadata and bytes need no credential access',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-incoming-host-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final path = '${directory.path}/mail.sqlite';
      final seeded = await Process.run('python3', [
        '../scripts/clients/android_incoming_fixture.py',
        '--prepare',
        path,
      ]);
      expect(seeded.exitCode, 0, reason: '${seeded.stderr}');
      final credentials = FixtureCredentials()..unavailable = true;
      final repository = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      await repository.initialize();
      final detail = await repository.detail('fixture:INBOX:files');
      expect(detail.files.map((f) => f.name), [
        'binary.bin',
        'résumé.txt',
        'binary.bin',
      ]);
      expect(await repository.attachment(detail.id, detail.files[0]), [
        0,
        255,
        1,
        13,
        10,
      ]);
      expect(await repository.attachment(detail.id, detail.files[2]), [
        0,
        1,
        2,
      ]);
      expect(credentials.reads, 0);
    },
  );
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
