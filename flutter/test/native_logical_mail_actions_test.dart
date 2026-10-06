import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/repository.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/src/rust/frb_generated.dart';
import 'package:shep_mobile/ui/mail_action_banner.dart';
import 'package:shep_mobile/ui/theme.dart';
import 'native_repository_test.dart' show FixtureCredentials;
import 'workspace_test.dart' show MemorySettings;

class _ResponseRepository extends NativeRepository {
  _ResponseRepository(super.profile, super.credentials);
  final requests = <Map<String, Object?>>[];
  Map<String, Object?> response = {
    'status': 'succeeded',
    'committed': true,
    'applied_fields': {'folder': 'INBOX', 'unread': null},
  };
  @override
  Future<dynamic> call(Map<String, Object?> request) async {
    requests.add(Map.of(request));
    return response;
  }
}

class _ImapPeer {
  _ImapPeer(this.server) {
    server.listen((socket) {
      sockets.add(socket);
      unawaited(_serve(socket));
    });
  }
  final SecureServerSocket server;
  final sockets = <SecureSocket>[];
  final commands = <String>[];
  final created = Completer<void>();
  final acknowledge = Completer<void>();
  bool archiveExists = false;

  static Future<_ImapPeer> open() async {
    final security = SecurityContext()
      ..useCertificateChain('../shared/mail-core/tests/fixtures/tls-cert.pem')
      ..usePrivateKey('../shared/mail-core/tests/fixtures/tls-private.pem');
    return _ImapPeer(
      await SecureServerSocket.bind(InternetAddress.loopbackIPv4, 0, security),
    );
  }

  Future<void> _serve(SecureSocket socket) async {
    socket.write('* OK synthetic IMAP ready\r\n');
    await for (final line
        in socket
            .cast<List<int>>()
            .transform(utf8.decoder)
            .transform(const LineSplitter())) {
      final split = line.indexOf(' ');
      final tag = line.substring(0, split);
      final command = line.substring(split + 1);
      commands.add(command.startsWith('LOGIN ') ? 'LOGIN' : command);
      if (command == 'CAPABILITY') {
        socket.write('* CAPABILITY IMAP4rev1 CREATE-SPECIAL-USE\r\n');
      } else if (command.startsWith('LIST ')) {
        if (command == 'LIST "" "*"') {
          socket.write('* LIST () "." "INBOX"\r\n');
          if (archiveExists) {
            socket.write('* LIST (\\Archive) "." "INBOX.Archive"\r\n');
          }
        } else if (command == 'LIST "" ""' || command.endsWith(' ""')) {
          socket.write('* LIST (\\Noselect) "." "INBOX."\r\n');
        } else if (archiveExists && command == 'LIST "" "INBOX.Archive"') {
          socket.write('* LIST (\\Archive) "." "INBOX.Archive"\r\n');
        }
      } else if (command.startsWith('CREATE ')) {
        expect(command, 'CREATE "INBOX.Archive" (USE (\\Archive))');
        created.complete();
        await acknowledge.future;
        archiveExists = true;
      } else if (command != 'LOGOUT' && !command.startsWith('LOGIN ')) {
        socket.write('$tag BAD unexpected command\r\n');
        continue;
      }
      socket.write('$tag OK completed\r\n');
      await socket.flush();
    }
  }

  Future<void> close() async {
    if (!acknowledge.isCompleted) acknowledge.complete();
    for (final socket in sockets) {
      socket.destroy();
    }
    await server.close();
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final certificate = File(
    '../shared/mail-core/tests/fixtures/tls-cert.pem',
  ).resolveSymbolicLinksSync();
  final enabled =
      Platform.isLinux && Platform.environment['SSL_CERT_FILE'] == certificate;
  setUpAll(() async {
    if (!enabled) return;
    await ShepNative.init(
      externalLibrary: ExternalLibrary.open(
        'build/native_assets/linux/libshep_mobile_native.so',
      ),
    );
  });
  test(
    'native logical response decoding and exact Resume keep physical evidence separate',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-logical-metadata-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final native = await NativeRepository.open(
        '${directory.path}/mail.sqlite',
        credentials: FixtureCredentials(),
      );
      final repository = _ResponseRepository(
        native.profile,
        native.credentials,
      );
      expect(
        await repository.executeLogicalMutation(
          'source',
          {'folder': 'Spam'},
          'same-attempt',
          'spam',
        ),
        {'folder': 'Inbox'},
      );
      repository.response = {
        'status': 'cancelled',
        'committed': false,
        'unchanged': true,
        'applied_fields': {'folder': 'INBOX'},
      };
      await expectLater(
        repository.executeLogicalMutation(
          'source',
          {'folder': 'Spam'},
          'same-attempt',
          'spam',
        ),
        throwsA(
          isA<MailOperationFailure>()
              .having((e) => e.unchanged, 'unchanged', true)
              .having((e) => e.appliedFields, 'display fields', {
                'folder': 'Inbox',
              }),
        ),
      );
      repository.response = {'status': 'succeeded', 'committed': true};
      final activity = MailActivity({
        'id': 'same-attempt',
        'mail': 'source',
        'account': 'fixture',
        'status': 'waiting',
        'fields': {'folder': 'INBOX.Junk Mail'},
        'logical_role': 'spam',
      });
      await repository.resumeMailAction(activity);
      expect(repository.requests.last['action_id'], 'same-attempt');
      expect(repository.requests.last['folder'], 'Spam');
      expect(repository.requests.last['logical_role'], 'spam');
      expect(activity.fields, {'folder': 'INBOX.Junk Mail'});
    },
    skip: !enabled,
  );
  testWidgets(
    'held CREATE Undo retains acknowledgement and sends no original or inverse MOVE',
    (tester) async {
      final peer = (await tester.runAsync(_ImapPeer.open))!;
      addTearDown(peer.close);
      final directory = (await tester.runAsync(
        () => Directory.systemTemp.createTemp('shep-logical-ffi-'),
      ))!;
      addTearDown(() => directory.delete(recursive: true));
      final seeded = (await tester.runAsync(
        () => Process.run('python3', [
          '../scripts/clients/provider_folders_fixture.py',
          '--prepare',
          '${directory.path}/mail.sqlite',
          '--port',
          '${peer.server.port}',
        ]),
      ))!;
      expect(seeded.exitCode, 0, reason: '${seeded.stderr}');
      final credentials = FixtureCredentials()
        ..values['fixture'] = ['synthetic-password', 'synthetic-smtp'];
      final repository = (await tester.runAsync(
        () => NativeRepository.open(
          '${directory.path}/mail.sqlite',
          credentials: credentials,
        ),
      ))!;
      final workspace = Workspace(repository, MemorySettings())
        ..setForeground(false);
      await tester.runAsync(workspace.initialize);
      final mail = workspace.visible.single;
      await tester.pumpWidget(
        MaterialApp(
          theme: shepTheme(Brightness.light),
          home: Scaffold(
            body: AnimatedBuilder(
              animation: workspace,
              builder: (_, _) => MailActionBanner(workspace: workspace),
            ),
          ),
        ),
      );
      await tester.runAsync(() async {
        unawaited(workspace.action(mail.id, MailAction.archive));
        await peer.created.future.timeout(const Duration(seconds: 15));
      });
      await tester.pump();
      final original = workspace.moves.records.single;
      expect(original.started, true);
      expect(original.committed, false);
      expect(find.text('Archiving 1 message'), findsOneWidget);
      await tester.tap(find.widgetWithText(TextButton, 'Undo'));
      await tester.pump();
      expect(workspace.mail(mail.id)!.folder, 'Inbox');
      await tester.runAsync(() async {
        for (var n = 0; (await repository.mailActions()).length != 2; n++) {
          if (n == 200) fail('newer inverse admission did not settle');
          await Future<void>.delayed(const Duration(milliseconds: 10));
        }
        peer.acknowledge.complete();
      });
      for (var n = 0; workspace.pending != 0; n++) {
        if (n == 200) fail('Undo did not settle');
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 10)),
        );
        await tester.pump();
      }
      await tester.pump();
      await tester.runAsync(() async {
        final actions = await repository.mailActions();
        final source = actions.singleWhere((a) => a.id == original.actionId);
        final inverse = actions.singleWhere((a) => a.id != original.actionId);
        expect(source.status, 'cancelled');
        expect(inverse.status, 'cancelled');
        expect(inverse.fields, {'folder': 'Inbox'});
        final folder = (await repository.folderCreations()).single;
        expect(folder.acknowledged, true);
        expect(folder.hasReceipt, true);
        expect(folder.data['receipt']['name'], 'INBOX.Archive');
        expect(peer.commands.where((c) => c.startsWith('CREATE ')).length, 1);
        expect(
          peer.commands.where(
            (c) => c.contains('MOVE') || c.startsWith('SELECT '),
          ),
          isEmpty,
        );
        expect((await repository.detail(mail.id)).folder, 'Inbox');
        expect(workspace.mail(mail.id)!.folder, 'Inbox');
        expect(workspace.error, isNull);
        debugPrint(
          'Original ${source.id}: ${source.status}; inverse ${inverse.id}: ${inverse.status}; CREATE ${folder.id}: ${folder.status}, acknowledged=${folder.acknowledged}.',
        );
        debugPrint('Loopback IMAP commands: ${peer.commands.join('; ')}');
      });
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
    },
    // The integration runner supplies process-scoped fixture TLS trust.
    skip: !enabled,
  );
}
