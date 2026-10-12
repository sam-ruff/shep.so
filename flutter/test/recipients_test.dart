import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/recipients.dart';
import 'package:shep_mobile/ui/recipient_field.dart';

Future<List<Map<String, dynamic>>> cases(String section) async {
  final all =
      jsonDecode(await File('../shared/recipient-cases.json').readAsString())
          as Map<String, dynamic>;
  return List<Map<String, dynamic>>.from(all[section] as List);
}

List<String> texts(Recipients recipients) =>
    recipients.chips.map((chip) => chip.text).toList();

void typed(
  EditHistory<String> history,
  List<String> text,
  String typed,
  DateTime now,
) {
  for (final c in typed.split('')) {
    final before = text.single;
    history.record(Insert(space: c.trim().isEmpty), now, () => before);
    text[0] = before + c;
  }
}

void main() {
  test('shared split cases', () async {
    final split = await cases('split');
    expect(split.length, greaterThanOrEqualTo(20));
    for (final c in split) {
      final (chips, entry) = splitRecipients(c['input'] as String);
      expect(chips, c['chips'], reason: c['name'] as String);
      expect(entry, c['entry'], reason: c['name'] as String);
    }
  });

  test('shared stored cases', () async {
    for (final c in await cases('stored')) {
      final recipients = Recipients.parse(c['value'] as String);
      expect(texts(recipients), c['chips'], reason: c['name'] as String);
      expect(recipients.entry, '', reason: c['name'] as String);
    }
  });

  test('shared value cases', () async {
    for (final c in await cases('value')) {
      final recipients = Recipients(
        chips: List<String>.from(
          c['chips'] as List,
        ).map(RecipientChip.new).toList(),
        entry: c['entry'] as String,
      );
      expect(recipients.value, c['value'], reason: c['name'] as String);
    }
  });

  test('shared paste cases', () async {
    for (final c in await cases('paste')) {
      expect(
        pasteRecipientLines(c['clipboard'] as String),
        c['text'],
        reason: c['name'] as String,
      );
    }
  });

  test('chips are marked as Send would treat them', () async {
    for (final c in await cases('valid')) {
      expect(
        RecipientChip(c['chip'] as String).valid,
        c['valid'],
        reason: c['chip'] as String,
      );
    }
  });

  test('typing then committing keeps the draft value', () {
    final recipients = Recipients()..input('a@example.com; b@exa');
    expect(recipients.value, 'a@example.com, b@exa');
    recipients.input('b@example.com');
    expect(recipients.value, 'a@example.com, b@example.com');
    expect(recipients.commit(), isTrue);
    expect(texts(recipients), ['a@example.com', 'b@example.com']);
    expect(recipients.commit(), isFalse);
  });

  test('remove checks the chip it was shown', () {
    final recipients = Recipients.parse('a@example.com, b@example.com');
    expect(recipients.remove(0, 'b@example.com'), isFalse);
    expect(recipients.remove(5, 'a@example.com'), isFalse);
    expect(recipients.remove(1, 'b@example.com'), isTrue);
    expect(recipients.value, 'a@example.com');
  });

  test('reopening moves only the last chip into an empty entry', () {
    final recipients = Recipients.parse('a@example.com, b@example.com')
      ..entry = 'c';
    expect(recipients.reopenLast(), isFalse);
    recipients.entry = '';
    expect(recipients.reopenLast(), isTrue);
    expect(recipients.entry, 'b@example.com');
    expect(texts(recipients), ['a@example.com']);
    expect(recipients.value, 'a@example.com, b@example.com');
    expect(Recipients().reopenLast(), isFalse);
  });

  test('whitespace-only entries clear without a chip', () {
    final recipients = Recipients(entry: '  ');
    expect(recipients.commit(), isTrue);
    expect(recipients.chips, isEmpty);
    expect(recipients.entry, '');
  });

  group('history', () {
    final now = DateTime(2026, 10, 12);
    EditHistory<String> strings() => EditHistory<String>((s) => s.length);

    test('typing undoes a word at a time', () {
      final history = strings();
      final text = [''];
      typed(history, text, 'hello big world', now);
      expect(history.undo(() => text.single), 'hello big');
      expect(history.undo(() => 'hello big'), 'hello');
      expect(history.undo(() => 'hello'), '');
      expect(history.undo(() => ''), isNull);
    });

    test('redo replays undone steps until a new edit', () {
      final history = strings();
      typed(history, [''], 'one two', now);
      expect(history.undo(() => 'one two'), 'one');
      expect(history.redo(() => 'one'), 'one two');
      expect(history.undo(() => 'one two'), 'one');
      history.record(const Insert(space: false), now, () => 'one');
      expect(history.redo(() => 'one!'), isNull);
      expect(history.undo(() => 'one!'), 'one');
    });

    test('a pause or a different edit starts a new step', () {
      final history = strings();
      final text = [''];
      typed(history, text, 'abc', now);
      typed(history, text, 'def', now.add(EditHistory.pause));
      final later = now.add(EditHistory.pause);
      history.record(const Delete(), later, () => 'abcdef');
      history.record(const Delete(), later, () => 'abcde');
      history.record(const OtherEdit(), later, () => 'abcd');
      history.record(const OtherEdit(), later, () => 'pasted');
      final undone = <String>[];
      for (String? s; (s = history.undo(() => '')) != null;) {
        undone.add(s!);
      }
      expect(undone, ['pasted', 'abcd', 'abcdef', 'abc', '']);
    });

    test('sealing ends the open step', () {
      final history = strings();
      final text = [''];
      typed(history, text, 'ab', now);
      history.seal();
      typed(history, text, 'cd', now);
      expect(history.undo(() => 'abcd'), 'ab');
    });

    test('history is bounded by steps and bytes', () {
      final history = strings();
      for (var step = 0; step < EditHistory.steps + 20; step++) {
        history.record(const OtherEdit(), now, () => '$step');
      }
      expect(history.length, EditHistory.steps);
      final large = strings();
      final chunk = 'x' * (EditHistory.bytes ~/ 3);
      for (var i = 0; i < 5; i++) {
        large.record(const OtherEdit(), now, () => chunk);
      }
      expect(large.size, lessThanOrEqualTo(EditHistory.bytes));
      expect(large.length, 3);
    });

    test('field changes are classified by their values', () {
      expect(FieldEdit.between('ab', 'abc'), const Insert(space: false));
      expect(FieldEdit.between('ab', 'ab '), const Insert(space: true));
      expect(FieldEdit.between('ab', 'axb'), const Insert(space: false));
      expect(FieldEdit.between('abc', 'ab'), const Delete());
      expect(FieldEdit.between('ab', 'ab, cd@example.com'), const OtherEdit());
      expect(FieldEdit.between('ab', 'xy'), const OtherEdit());
      expect(FieldEdit.between('é', 'éü'), const Insert(space: false));
    });
  });

  group('controller', () {
    var clock = DateTime(2026, 10, 12);
    RecipientsController controller([String value = '']) =>
        RecipientsController(value, clock: () => clock);

    test('a separator turns finished addresses into chips', () {
      final c = controller();
      expect(c.edit('a@example.com;b@exa'), isTrue);
      expect(c.chips.map((chip) => chip.text), ['a@example.com']);
      expect(c.entry.text, '${recipientSentinel}b@exa');
      expect(c.value, 'a@example.com, b@exa');
    });

    test('deleting the sentinel reopens the last chip', () {
      final c = controller('a@example.com, b@example.com');
      expect(c.entry.text, recipientSentinel);
      expect(c.edit(''), isTrue);
      expect(c.entry.text, '${recipientSentinel}b@example.com');
      expect(c.value, 'a@example.com, b@example.com');
      expect(c.chips, hasLength(1));
    });

    test('a pasted column splits on its line breaks', () {
      final c = controller();
      c.edit('a@example.com\nb@example.com\r\nc@example.com\n');
      expect(c.chips.map((chip) => chip.text), [
        'a@example.com',
        'b@example.com',
      ]);
      expect(c.value, 'a@example.com, b@example.com, c@example.com');
    });

    test('a typed line break commits the entry', () {
      final c = controller();
      c.edit('alex@example.com');
      c.edit('alex@example.com\n');
      expect(c.chips.single.text, 'alex@example.com');
      expect(c.entry.text, recipientSentinel);
    });

    test('undo restores chips and redo replays them', () {
      final c = controller('a@example.com');
      c.remove(0, 'a@example.com');
      expect(c.value, '');
      expect(c.step(HistoryStep.undo), isTrue);
      expect(c.value, 'a@example.com');
      expect(c.step(HistoryStep.redo), isTrue);
      expect(c.value, '');
      expect(c.step(HistoryStep.redo), isFalse);
    });

    test('typing pauses split undo steps', () {
      final c = controller();
      c.edit('a');
      c.edit('ab');
      clock = clock.add(EditHistory.pause);
      c.edit('abc');
      expect(c.step(HistoryStep.undo), isTrue);
      expect(c.value, 'ab');
      expect(c.step(HistoryStep.undo), isTrue);
      expect(c.value, '');
    });
  });
}
