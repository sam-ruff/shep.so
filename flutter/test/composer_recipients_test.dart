import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/composer.dart';
import 'package:shep_mobile/ui/recipient_field.dart';
import 'package:shep_mobile/ui/theme.dart';

import 'composer_lifecycle_test.dart'
    show DraftLifecycleRepository, MemorySettings, loadPreviewFonts;

class SendingRepository extends DraftLifecycleRepository {
  final sent = <Draft>[];

  @override
  Future<void> send(Draft draft) async => sent.add(draft);
}

Future<(Workspace, SendingRepository)> openComposer(
  WidgetTester tester, {
  Draft draft = const Draft(id: 'draft'),
  Brightness brightness = Brightness.light,
}) async {
  await loadPreviewFonts();
  final repository = SendingRepository();
  final workspace = Workspace(repository, MemorySettings());
  addTearDown(workspace.dispose);
  await tester.pumpWidget(
    MaterialApp(
      theme: shepTheme(brightness),
      home: Builder(
        builder: (context) => TextButton(
          onPressed: () => Navigator.push(
            context,
            MaterialPageRoute<void>(
              builder: (_) => Composer(workspace: workspace, draft: draft),
            ),
          ),
          child: const Text('Open composer'),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open composer'));
  await tester.pumpAndSettle();
  return (workspace, repository);
}

Finder recipientField(String label) =>
    find.widgetWithText(RecipientField, label);

Finder entry(String label) => find.descendant(
  of: recipientField(label),
  matching: find.byType(TextField),
);

Finder labelled(String label) => find.byWidgetPredicate(
  (widget) => widget is TextField && widget.decoration?.labelText == label,
);

RecipientsController recipients(WidgetTester tester, String label) =>
    tester.widget<RecipientField>(recipientField(label)).controller;

List<String> chips(WidgetTester tester, String label) =>
    recipients(tester, label).chips.map((chip) => chip.text).toList();

String entryText(WidgetTester tester, String label) => tester
    .widget<TextField>(entry(label))
    .controller!
    .text
    .replaceAll(recipientSentinel, '');

Future<void> chord(
  WidgetTester tester,
  LogicalKeyboardKey key, {
  bool shift = false,
}) async {
  await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
  if (shift) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyEvent(key);
  if (shift) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
  await tester.pump();
}

Future<void> paste(WidgetTester tester, String label, String text) async {
  tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
    SystemChannels.platform,
    (call) async => switch (call.method) {
      'Clipboard.getData' => {'text': text},
      'Clipboard.hasStrings' => {'value': true},
      _ => null,
    },
  );
  addTearDown(
    () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      null,
    ),
  );
  await tester.tap(entry(label));
  await tester.pump();
  await chord(tester, LogicalKeyboardKey.keyV);
  await tester.pump();
}

Future<Draft> autosaved(
  WidgetTester tester,
  SendingRepository repository,
) async {
  await tester.pump(const Duration(milliseconds: 500));
  final draft = repository.saves.last;
  repository.gates.last.complete();
  await tester.pump();
  return draft;
}

void main() {
  testWidgets('typed separators make chips and autosave the comma string', (
    tester,
  ) async {
    final (_, repository) = await openComposer(tester);
    await tester.enterText(entry('To'), 'a@example.com;b@example.com');
    await tester.pump();
    expect(chips(tester, 'To'), ['a@example.com']);
    expect(entryText(tester, 'To'), 'b@example.com');
    expect(find.text('Unsaved changes'), findsOneWidget);
    final saved = await autosaved(tester, repository);
    expect(saved.to, 'a@example.com, b@example.com');
  });

  testWidgets('pasting a column or a semicolon list makes chips at once', (
    tester,
  ) async {
    final (_, repository) = await openComposer(tester);
    await paste(
      tester,
      'To',
      'a@example.com\nb@example.com\r\nc@example.com\n',
    );
    expect(chips(tester, 'To'), ['a@example.com', 'b@example.com']);
    expect(entryText(tester, 'To'), 'c@example.com');

    await tester.tap(find.text('Cc / Bcc'));
    await tester.pump();
    await paste(tester, 'Cc', 'd@example.com; Alex Doe <e@example.com>; ');
    expect(chips(tester, 'Cc'), ['d@example.com', 'Alex Doe <e@example.com>']);
    expect(entryText(tester, 'Cc'), '');
    final saved = await autosaved(tester, repository);
    expect(saved.to, 'a@example.com, b@example.com, c@example.com');
    expect(saved.cc, 'd@example.com, Alex Doe <e@example.com>');
  });

  testWidgets('chip x removes it and Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y replay', (
    tester,
  ) async {
    final (_, repository) = await openComposer(
      tester,
      draft: const Draft(id: 'draft', to: 'a@example.com, b@example.com'),
    );
    expect(chips(tester, 'To'), ['a@example.com', 'b@example.com']);
    final semantics = tester.ensureSemantics();
    await tester.pump();
    expect(find.bySemanticsLabel('Remove a@example.com'), findsOneWidget);
    semantics.dispose();
    expect(tester.getSize(find.byTooltip('Remove a@example.com')).height, 44);
    await tester.tap(find.byTooltip('Remove a@example.com'));
    await tester.pump();
    expect(chips(tester, 'To'), ['b@example.com']);

    await tester.tap(entry('To'));
    await tester.pump();
    await chord(tester, LogicalKeyboardKey.keyZ);
    expect(chips(tester, 'To'), ['a@example.com', 'b@example.com']);
    await chord(tester, LogicalKeyboardKey.keyZ, shift: true);
    expect(chips(tester, 'To'), ['b@example.com']);
    await chord(tester, LogicalKeyboardKey.keyZ);
    await chord(tester, LogicalKeyboardKey.keyY);
    expect(chips(tester, 'To'), ['b@example.com']);
    final saved = await autosaved(tester, repository);
    expect(saved.to, 'b@example.com');
    expect(saved.revision, greaterThanOrEqualTo(4));
  });

  testWidgets('Backspace in an empty entry reopens the last chip', (
    tester,
  ) async {
    final (_, repository) = await openComposer(
      tester,
      draft: const Draft(id: 'draft', to: 'a@example.com, b@example.com'),
    );
    await tester.tap(entry('To'));
    await tester.pump();
    await tester.sendKeyEvent(LogicalKeyboardKey.backspace);
    await tester.pump();
    expect(chips(tester, 'To'), ['a@example.com']);
    expect(entryText(tester, 'To'), 'b@example.com');

    // A soft keyboard deletes the leading sentinel without a key event.
    await tester.enterText(entry('To'), recipientSentinel);
    await tester.pump();
    expect(chips(tester, 'To'), ['a@example.com']);
    tester.testTextInput.updateEditingValue(TextEditingValue.empty);
    await tester.pump();
    expect(chips(tester, 'To'), isEmpty);
    expect(entryText(tester, 'To'), 'a@example.com');
    final saved = await autosaved(tester, repository);
    expect(saved.to, 'a@example.com');
  });

  testWidgets('focus loss and Done turn the entry into a chip', (tester) async {
    final (_, _) = await openComposer(tester);
    await tester.enterText(entry('To'), 'alex@example.com');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();
    expect(chips(tester, 'To'), ['alex@example.com']);
    expect(tester.widget<TextField>(entry('To')).focusNode!.hasFocus, isTrue);

    await tester.enterText(entry('To'), 'Alex Doe');
    await tester.tap(labelled('Subject'));
    await tester.pump();
    expect(chips(tester, 'To'), ['alex@example.com', 'Alex Doe']);
    final semantics = tester.ensureSemantics();
    await tester.pump();
    expect(
      find.bySemanticsLabel('Alex Doe, not a valid address'),
      findsOneWidget,
    );
    expect(find.bySemanticsLabel('alex@example.com'), findsOneWidget);
    semantics.dispose();
  });

  testWidgets('tapping empty field space focuses the entry', (tester) async {
    await openComposer(
      tester,
      draft: const Draft(id: 'draft', to: 'a@example.com'),
    );
    final field = tester.getRect(recipientField('To'));
    await tester.tapAt(field.centerRight - const Offset(8, 0));
    await tester.pump();
    expect(tester.widget<TextField>(entry('To')).focusNode!.hasFocus, isTrue);
  });

  testWidgets('Send receives the comma-separated recipients', (tester) async {
    final (_, repository) = await openComposer(tester);
    await tester.enterText(entry('To'), 'a@example.com; b@example.com, c@exa');
    await tester.enterText(labelled('Subject'), 'Hello');
    await tester.tap(find.byTooltip('Send'));
    await tester.pumpAndSettle();
    expect(repository.sent.single.to, 'a@example.com, b@example.com, c@exa');
  });

  testWidgets('Ctrl+Z undoes typing in the message body', (tester) async {
    await openComposer(tester);
    final body = labelled('Message');
    await tester.tap(body);
    await tester.enterText(body, 'Hello');
    await tester.pump(const Duration(seconds: 1));
    await tester.enterText(body, 'Hello world');
    await tester.pump(const Duration(seconds: 1));
    await chord(tester, LogicalKeyboardKey.keyZ);
    expect(tester.widget<TextField>(body).controller!.text, 'Hello');
    await chord(tester, LogicalKeyboardKey.keyZ, shift: true);
    expect(tester.widget<TextField>(body).controller!.text, 'Hello world');
  });

  for (final brightness in Brightness.values) {
    testWidgets('recipient chips in ${brightness.name}', (tester) async {
      tester.view.physicalSize = const Size(420, 640);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await openComposer(
        tester,
        brightness: brightness,
        draft: const Draft(
          id: 'draft',
          to: 'alex@example.com, Robin Doe <robin@example.com>, robin',
          cc: 'copy@example.com',
        ),
      );
      await tester.enterText(entry('To'), 'sam@exa');
      await tester.pump();
      await expectLater(
        find.byType(Scaffold),
        matchesGoldenFile('goldens/composer_recipients_${brightness.name}.png'),
      );
    });
  }
}
