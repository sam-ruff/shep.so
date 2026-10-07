import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/attachments.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/reader.dart';
import 'package:shep_mobile/ui/theme.dart';
import 'composer_lifecycle_test.dart' show loadPreviewFonts;
import 'support/paged_repository.dart';
import 'workspace_test.dart' show MemorySettings;

/// CRLF and bare LF line ends, 8-bit bytes and no final line break.
final exactRaw = Uint8List.fromList([
  ...'From: Alex <alex@example.test>\r\nSubject: Exact\r\n\r\nCaf'.codeUnits,
  0xE9,
  0x0A,
  0xFF,
  0x0D,
  0x0A,
  ...'end  '.codeUnits,
]);

const file = ReceivedAttachment(
  id: 'file-1',
  name: 'notes.txt',
  mediaType: 'text/plain',
  size: 5,
);

Mail original({bool loaded = true}) => Mail(
  id: 'original-message',
  sender: 'Alex',
  address: 'alex@example.test',
  subject: 'Exact',
  preview: 'Exact bytes',
  body: 'Café',
  date: DateTime(2026, 10, 7, 9),
  account: 'Receiving account',
  accountId: 'account',
  unread: false,
  bodyLoaded: loaded,
  attachments: const ['notes.txt'],
  files: const [file],
);

class OriginalRepository extends PagedRepository
    implements OriginalMessageRepository, AttachmentRepository {
  final requests = <String>[];
  Object? failure;
  Completer<void>? hold;
  @override
  List<Mail> get cached => [original()];
  @override
  Future<Uint8List> originalMessage(String id) async {
    requests.add(id);
    await hold?.future;
    if (failure case final Object error) throw error;
    return exactRaw;
  }

  @override
  Future<Uint8List> attachment(String message, ReceivedAttachment file) async =>
      Uint8List.fromList('notes'.codeUnits);
}

void main() {
  final calls = <Map<Object?, Object?>>[];
  Object? Function() reply = () => true;
  setUp(() {
    calls.clear();
    reply = () => true;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(AttachmentSaver.channel, (call) async {
          calls.add(call.arguments as Map<Object?, Object?>);
          final value = reply();
          if (value is PlatformException) throw value;
          return value;
        });
  });
  tearDown(
    () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(AttachmentSaver.channel, null),
  );

  Future<Workspace> open(
    WidgetTester tester,
    PagedRepository repository, {
    bool dark = false,
  }) async {
    tester.view.physicalSize = const Size(360, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final workspace = Workspace(repository, MemorySettings());
    await workspace.loadPage();
    await tester.pumpWidget(
      MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: shepTheme(dark ? Brightness.dark : Brightness.light),
        home: Reader(
          workspace: workspace,
          id: 'original-message',
          act: (_, _) {},
        ),
      ),
    );
    await tester.pump();
    await workspace.loadBody('original-message');
    await tester.pump();
    await tester.ensureVisible(saveOriginal);
    await tester.pump();
    return workspace;
  }

  Future<void> close(WidgetTester tester, Workspace workspace) async {
    await tester.pumpWidget(const SizedBox());
    workspace.dispose();
  }

  String status(WidgetTester tester) => tester
      .widget<Text>(find.byKey(const ValueKey('reader-original-status')))
      .data!;
  Color? statusColour(WidgetTester tester) => tester
      .widget<Text>(find.byKey(const ValueKey('reader-original-status')))
      .style
      ?.color;

  testWidgets(
    'Save original message hands the exact cached bytes to the picker',
    (tester) async {
      final repository = OriginalRepository();
      final workspace = await open(tester, repository);
      await tester.tap(saveOriginal);
      await tester.pump();
      await tester.pump();
      expect(repository.requests, ['original-message']);
      final call = calls.single;
      expect(call['name'], 'message.eml');
      expect(call['type'], 'message/rfc822');
      expect(call['bytes'], exactRaw);
      expect(status(tester), 'Original message saved.');
      expect(find.textContaining('notes.txt saved'), findsNothing);
      await close(tester, workspace);
    },
  );

  testWidgets('a cancelled picker is reported as cancelled, not as an error', (
    tester,
  ) async {
    reply = () => false;
    final repository = OriginalRepository();
    final workspace = await open(tester, repository);
    await tester.tap(saveOriginal);
    await tester.pump();
    await tester.pump();
    expect(status(tester), 'Save cancelled.');
    final scheme = Theme.of(tester.element(saveOriginal)).colorScheme;
    expect(statusColour(tester), scheme.onSurfaceVariant);
    expect(workspace.error, isNull);
    expect(tester.widget<OutlinedButton>(saveOriginal).enabled, isTrue);
    await close(tester, workspace);
  });

  testWidgets('read and picker failures stay visible and Save can be retried', (
    tester,
  ) async {
    final repository = OriginalRepository()
      ..failure = const MailOperationFailure(
        'This message moved or is no longer cached. Refresh its folder.',
      );
    final workspace = await open(tester, repository);
    await tester.tap(saveOriginal);
    await tester.pump();
    await tester.pump();
    expect(status(tester), contains('no longer cached'));
    final scheme = Theme.of(tester.element(saveOriginal)).colorScheme;
    expect(statusColour(tester), scheme.error);
    expect(calls, isEmpty, reason: 'nothing reaches the picker');
    repository.failure = null;
    reply = () => PlatformException(code: 'write', message: 'private detail');
    await tester.tap(saveOriginal);
    await tester.pump();
    await tester.pump();
    expect(status(tester), startsWith('Could not save the original message.'));
    expect(find.textContaining('private detail'), findsNothing);
    reply = () => true;
    await tester.tap(saveOriginal);
    await tester.pump();
    await tester.pump();
    expect(status(tester), 'Original message saved.');
    expect(calls, hasLength(2));
    expect(calls.last['bytes'], exactRaw);
    await close(tester, workspace);
  });

  testWidgets(
    'a held original save disables attachment saves that share the picker',
    (tester) async {
      final repository = OriginalRepository()..hold = Completer<void>();
      final workspace = await open(tester, repository);
      await tester.tap(saveOriginal);
      await tester.pump();
      expect(tester.widget<OutlinedButton>(saveOriginal).enabled, isFalse);
      expect(tester.widget<OutlinedButton>(attachment).enabled, isFalse);
      await tester.tap(saveOriginal, warnIfMissed: false);
      repository.hold!.complete();
      await tester.pump();
      await tester.pump();
      expect(repository.requests, hasLength(1));
      expect(calls, hasLength(1));
      expect(tester.widget<OutlinedButton>(attachment).enabled, isTrue);
      await tester.ensureVisible(attachment);
      await tester.tap(attachment);
      await tester.pump();
      await tester.pump();
      expect(calls.last['name'], 'notes.txt');
      expect(status(tester), 'Original message saved.');
      expect(find.text('notes.txt saved.'), findsOneWidget);
      await close(tester, workspace);
    },
  );

  testWidgets('a client without the native cache explains the limit', (
    tester,
  ) async {
    final repository = PagedRepository(extra: [original()]);
    final workspace = await open(tester, repository);
    await tester.tap(saveOriginal);
    await tester.pump();
    await tester.pump();
    expect(status(tester), contains('installed Shep app'));
    expect(calls, isEmpty);
    await close(tester, workspace);
  });

  for (final dark in [false, true]) {
    testWidgets(
      'compact Save original message status ${dark ? 'dark' : 'light'}',
      (tester) async {
        await loadPreviewFonts();
        reply = () => false;
        final repository = OriginalRepository();
        final workspace = await open(tester, repository, dark: dark);
        await tester.tap(saveOriginal);
        await tester.pump();
        await tester.pump();
        await tester.ensureVisible(
          find.byKey(const ValueKey('reader-original-status')),
        );
        await tester.pump();
        await expectLater(
          find.byType(MaterialApp),
          matchesGoldenFile(
            'goldens/original_message_compact_${dark ? 'dark' : 'light'}.png',
          ),
        );
        await close(tester, workspace);
      },
    );
  }
}

final saveOriginal = find.byKey(const ValueKey('reader-save-original'));
final attachment = find.ancestor(
  of: find.text('Save notes.txt (5 bytes)'),
  matching: find.byWidgetPredicate((widget) => widget is OutlinedButton),
);
