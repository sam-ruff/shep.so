import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/ui/message_find.dart';

/// Saved real reader controls shared by host and Android integration tests.
/// The caller opens the synthetic message before invoking this scenario.
Future<void> messageFindScenario(
  WidgetTester tester, {
  Future<void> Function(String)? screenshot,
}) async {
  Future<void> status(String text) async {
    for (var attempt = 0; find.text(text).evaluate().isEmpty; attempt++) {
      if (attempt >= 300) fail('Find did not show $text');
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 10)),
      );
      await tester.pump(const Duration(milliseconds: 100));
    }
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
  }

  final input = find.widgetWithText(TextField, 'Find in message');
  Future<void> query(String value, String expected) async {
    await tester.enterText(input, value);
    await status(expected);
  }

  void visibleHit() {
    final widgets = tester.widgetList<SearchableMessageText>(
      find.byType(SearchableMessageText),
    );
    final model = widgets.first.find, hit = model.hits[model.active];
    final body = find.descendant(
      of: find.byType(SearchableMessageText).at(hit.block),
      matching: find.byType(EditableText),
    );
    final render = tester.state<EditableTextState>(body).renderEditable;
    final caret = render
        .getLocalRectForCaret(TextPosition(offset: hit.start))
        .inflate(8);
    final scroll = tester.getRect(find.byType(SingleChildScrollView).first);
    expect(
      scroll.contains(render.localToGlobal(caret.topLeft)) &&
          scroll.contains(render.localToGlobal(caret.bottomRight)),
      isTrue,
      reason: 'The complete active match and margin must be inside $scroll',
    );
  }

  await tester.tap(find.byTooltip('Find in message'));
  await tester.pumpAndSettle();
  await query('alpha', '1 of 2');
  visibleHit();
  await tester.tap(find.byTooltip('Next match'));
  await status('2 of 2');
  visibleHit();
  if (screenshot != null) await screenshot('native-find-tail');
  await tester.tap(find.byTooltip('Next match'));
  await status('1 of 2');
  await tester.tap(find.byTooltip('Previous match'));
  await status('2 of 2');
  await tester.tap(find.byTooltip('Match case'));
  await status('No matches');
  await tester.tap(find.byTooltip('Match case'));
  await status('1 of 2');
  await query('Alpha wraps over lines.', '1 of 1');
  visibleHit();
  await query('CAFÉ', '1 of 3');
  await query('alpha', '1 of 2');
  await tester.ensureVisible(find.text('Quoted history'));
  await tester.tap(find.text('Quoted history'));
  await status('1 of 3');
  await tester.tap(find.byTooltip('Previous match'));
  await status('3 of 3');
  visibleHit();
  if (screenshot != null) await screenshot('native-find-quoted');
  await tester.tap(find.byTooltip('Close Find'));
  await tester.pumpAndSettle();
  expect(input, findsNothing);
  // Native word selection and Copy remain available in the highlighted reader.
  await tester.tap(find.byTooltip('Find in message'));
  await tester.pumpAndSettle();
  await status('1 of 3');
  final body = find.descendant(
    of: find.byType(SearchableMessageText).first,
    matching: find.byType(EditableText),
  );
  final state = tester.state<EditableTextState>(body);
  final position = state.renderEditable.localToGlobal(
    state.renderEditable
        .getLocalRectForCaret(const TextPosition(offset: 24))
        .center,
  );
  await tester.longPressAt(position);
  await tester.pumpAndSettle();
  expect(state.widget.controller.selection.isCollapsed, isFalse);
  expect(find.text('Copy'), findsOneWidget);
  await tester.tap(find.text('Copy'));
  await tester.pumpAndSettle();
  expect(
    (await tester.runAsync(
      () => Clipboard.getData(Clipboard.kTextPlain),
    ))?.text,
    'Alpha',
  );
  await tester.tap(find.byTooltip('Close Find'));
  await tester.pumpAndSettle();
}
