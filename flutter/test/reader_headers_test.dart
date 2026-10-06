import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/ui/reader_headers.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/reader.dart';
import 'workspace_test.dart' show MemorySettings;
import 'support/reader_headers_scenario.dart';

void main() {
  String? copied;
  bool failCopy = false;
  Completer<void>? hold;
  setUp(() {
    failCopy = false;
    hold = null;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'Clipboard.setData') {
            if (failCopy) {
              throw PlatformException(
                code: 'clipboard-unavailable',
                message: 'private platform detail',
              );
            }
            copied = call.arguments['text'] as String;
            await hold?.future;
          }
          if (call.method == 'Clipboard.getData') return {'text': copied};
          return null;
        });
  });
  tearDown(
    () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, null),
  );
  for (final dark in [false, true]) {
    testWidgets(
      'accurate selectable/copyable headers survive loading/refresh/aliases (${dark ? 'dark' : 'light'})',
      (tester) async {
        await tester.binding.setSurfaceSize(const Size(360, 800));
        addTearDown(() => tester.binding.setSurfaceSize(null));
        await readerHeadersScenario(
          tester,
          dark: dark,
          clipboard: () async => copied,
        );
      },
    );
  }
  testWidgets(
    'empty headers do not copy invented placeholders or account identity',
    (tester) async {
      final mail = headerMail(subject: '', recipient: '', senderHeader: '');
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(body: ReaderHeaders(mail: mail)),
        ),
      );
      for (final key in ['subject', 'sender', 'recipient']) {
        expect(find.byKey(ValueKey('reader-$key-empty')), findsOneWidget);
        expect(
          tester
              .widget<IconButton>(find.byKey(ValueKey('reader-copy-$key')))
              .onPressed,
          isNull,
        );
      }
      expect(find.text('Account: Receiving account'), findsOneWidget);
      expect(find.textContaining('Bcc'), findsNothing);
    },
  );
  testWidgets(
    'subject text uses the native selection menu and copies the exact string',
    (tester) async {
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(body: ReaderHeaders(mail: headerMail())),
        ),
      );
      await tester.longPress(find.byKey(const ValueKey('reader-subject')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Select all'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Copy'));
      await tester.pumpAndSettle();
      expect(copied, headerSubject);
    },
  );
  testWidgets(
    'headers stay copyable through body failure and its real Load message retry',
    (tester) async {
      final repository = HeaderRepository();
      final workspace = Workspace(repository, MemorySettings());
      await workspace.loadPage();
      try {
        await tester.pumpWidget(
          MaterialApp(
            home: Reader(
              workspace: workspace,
              id: repository.metadata.id,
              act: (_, _) {},
            ),
          ),
        );
        final pending = workspace.loadBody(repository.metadata.id);
        await tester.pump();
        repository.body.completeError(StateError('Body unavailable.'));
        await pending;
        await tester.pump();
        expect(find.textContaining('Body unavailable.'), findsOneWidget);
        await tester.tap(find.byKey(const ValueKey('reader-copy-subject')));
        await tester.pump();
        expect(copied, headerSubject);
        final to = find.byKey(const ValueKey('reader-copy-recipient'));
        await tester.ensureVisible(to);
        await tester.tap(to);
        await tester.pump();
        expect(copied, headerRecipient);
        repository.body = Completer();
        final retry = find.widgetWithText(TextButton, 'Load message');
        await tester.ensureVisible(retry);
        await tester.tap(retry);
        await tester.pump();
        repository.body.complete(
          headerMail(body: 'Recovered source', loaded: true),
        );
        await tester.pump();
        await tester.pump();
        expect(workspace.bodyError(repository.metadata.id), isNull);
        expect(
          tester
              .widget<SelectableText>(
                find.byKey(const ValueKey('reader-recipient')),
              )
              .data,
          headerRecipient,
        );
      } finally {
        await tester.pumpWidget(const SizedBox.shrink());
        workspace.dispose();
      }
    },
  );
  testWidgets(
    'copy failure has visible recovery and an old reply cannot replace newer metadata',
    (tester) async {
      Widget screen(String recipient) => MaterialApp(
        home: Scaffold(
          body: ReaderHeaders(mail: headerMail(recipient: recipient)),
        ),
      );
      await tester.pumpWidget(screen(headerRecipient));
      failCopy = true;
      await tester.tap(find.byKey(const ValueKey('reader-copy-subject')));
      await tester.pump();
      expect(
        find.text('Could not copy subject. Try Copy again.'),
        findsOneWidget,
      );
      failCopy = false;
      hold = Completer<void>();
      await tester.tap(find.byKey(const ValueKey('reader-copy-recipient')));
      await tester.pump();
      await tester.pumpWidget(screen('New <new@example.test>'));
      hold!.complete();
      await tester.pump();
      expect(find.text('To copied.'), findsNothing);
      await tester.tap(find.byKey(const ValueKey('reader-copy-recipient')));
      await tester.pump();
      expect(copied, 'New <new@example.test>');
      expect(find.text('To copied.'), findsOneWidget);
    },
  );
}
