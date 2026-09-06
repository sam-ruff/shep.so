import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/attachments.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'support/paged_repository.dart';
import 'workspace_test.dart' show MemorySettings;

class IncomingRepository extends PagedRepository
    implements AttachmentRepository {
  final value = Mail(
    id: 'incoming',
    accountId: 'fixture',
    sender: 'Files',
    address: 'files@example.test',
    subject: 'Incoming fixture',
    preview: 'Cached body',
    body: 'Cached body',
    date: DateTime(2026, 9, 6),
    unread: false,
    attachments: const ['binary.bin'],
    files: const [
      ReceivedAttachment(
        id: 'file',
        name: 'binary.bin',
        mediaType: 'application/octet-stream',
        size: 3,
      ),
    ],
  );
  bool omitted = false;
  @override
  List<Mail> get cached => [value];
  @override
  Future<MailPage> page({
    required String folder,
    String? account,
    required String query,
    required String filter,
    required bool oldest,
    required int offset,
  }) async =>
      MailPage(omitted ? [] : [value.withoutBody()], omitted ? 0 : 1, 0);
  @override
  Future<Uint8List> attachment(String message, ReceivedAttachment file) async {
    expect(message, value.id);
    expect(file.id, 'file');
    return Uint8List.fromList([0, 255, 1]);
  }
}

void main() {
  test(
    'removed account reader cannot be restored by a late mutation failure',
    () async {
      final repo = IncomingRepository();
      final w = Workspace(repo, MemorySettings());
      addTearDown(w.dispose);
      await w.initialize();
      w.setForeground(false);
      w.retainReader('incoming');
      final action = w.action('incoming', MailAction.star);
      await Future<void>.delayed(Duration.zero);
      repo.omitted = true;
      w.drafts['deleted-draft'] = Draft(
        id: 'deleted-draft',
        accountId: 'fixture',
      );
      await w.accountRemoved('fixture');
      expect(w.drafts, isEmpty);
      expect(w.mail('incoming'), isNull);
      repo.jobs.single.completeError(StateError('Late provider failure'));
      await action;
      expect(w.mail('incoming'), isNull);
      expect(w.pending, 0);
      expect(w.visible, isEmpty);
    },
  );
  test(
    'reader keeps loaded detail and metadata after page exclusion, then releases it',
    () async {
      final repository = IncomingRepository();
      final w = Workspace(repository, MemorySettings());
      addTearDown(w.dispose);
      await w.initialize();
      w.setForeground(false);
      w.retainReader('incoming');
      await w.loadBody('incoming');
      repository.omitted = true;
      await w.loadPage();
      expect(w.visible, isEmpty);
      expect(w.mail('incoming')!.body, 'Cached body');
      expect(w.mail('incoming')!.files.single.id, 'file');
      final flag = w.action('incoming', MailAction.star);
      await Future<void>.delayed(Duration.zero);
      expect(w.mail('incoming')!.starred, isTrue);
      repository.jobs.single.complete();
      await flag;
      expect(w.mail('incoming')!.starred, isTrue);
      w.releaseReader('incoming');
      expect(w.mail('incoming'), isNull);
    },
  );
  testWidgets(
    'attachment save distinguishes cancellation, write failure and completed save',
    (tester) async {
      tester.view.physicalSize = const Size(412, 892);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final repo = IncomingRepository();
      final w = Workspace(repo, MemorySettings());
      await w.initialize();
      w.setForeground(false);
      var attempt = 0;
      final held = Completer<void>();
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(AttachmentSaver.channel, (call) async {
            expect(call.method, 'save');
            expect(call.arguments['bytes'], Uint8List.fromList([0, 255, 1]));
            expect(call.arguments['name'], 'binary.bin');
            attempt++;
            if (attempt == 1) {
              await held.future;
              return false;
            }
            if (attempt == 2) throw PlatformException(code: 'write');
            return true;
          });
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(AttachmentSaver.channel, null),
      );
      await tester.pumpWidget(ShepApp(workspace: w));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Incoming fixture'));
      await tester.pumpAndSettle();
      final save = find.text('Save binary.bin (3 bytes)');
      await tester.ensureVisible(save);
      await tester.tap(save);
      await tester.pump();
      await tester.pump();
      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      held.complete();
      await tester.pumpAndSettle();
      expect(find.text('Save cancelled.'), findsOneWidget);
      await tester.tap(save);
      await tester.pumpAndSettle();
      expect(find.textContaining('Could not save binary.bin'), findsOneWidget);
      expect(find.text('binary.bin saved.'), findsNothing);
      await tester.tap(save);
      await tester.pumpAndSettle();
      expect(find.text('binary.bin saved.'), findsOneWidget);
      expect(attempt, 3);
      await tester.pumpWidget(const SizedBox());
      w.dispose();
    },
  );
}
