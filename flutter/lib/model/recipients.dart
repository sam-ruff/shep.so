/// Recipient chips for the To, Cc and Bcc fields.
///
/// Drafts keep the comma-separated string; chips are only the editing model.
/// `shared/recipient-cases.json` is the splitting contract shared with the
/// desktop and browser clients.
library;

import 'dart:collection';

class RecipientChip {
  RecipientChip(this.text) : valid = recipientValid(text);
  final String text;

  /// Whether Send would accept this address.
  final bool valid;

  @override
  bool operator ==(Object other) =>
      other is RecipientChip && other.text == text;

  @override
  int get hashCode => text.hashCode;
}

class Recipients {
  Recipients({List<RecipientChip>? chips, this.entry = ''})
    : chips = chips ?? [];

  /// Every part of a saved draft value becomes a chip.
  factory Recipients.parse(String value) => Recipients()
    ..input(value)
    ..commit();

  final List<RecipientChip> chips;

  /// The unfinished text in the field's entry.
  String entry;

  Recipients copy() => Recipients(chips: [...chips], entry: entry);

  /// The draft value: every chip, then the unfinished entry.
  String get value => [
    ...chips.map((chip) => chip.text),
    if (entry.isNotEmpty) entry,
  ].join(', ');

  int get weight =>
      entry.length + chips.fold(0, (sum, chip) => sum + chip.text.length);

  /// Replaces the entry text, turning each finished address into a chip.
  void input(String text) {
    final (finished, rest) = splitRecipients(text);
    chips.addAll(finished.map(RecipientChip.new));
    entry = rest;
  }

  /// Turns the unfinished entry into a chip. Returns whether anything changed.
  bool commit() {
    final text = entry.trim();
    if (text.isEmpty) {
      final changed = entry.isNotEmpty;
      entry = '';
      return changed;
    }
    chips.add(RecipientChip(text));
    entry = '';
    return true;
  }

  /// Removes a chip if it still holds the text the view showed.
  bool remove(int index, String text) {
    if (index < 0 || index >= chips.length || chips[index].text != text) {
      return false;
    }
    chips.removeAt(index);
    return true;
  }

  /// Moves the last chip back into an empty entry for editing.
  bool reopenLast() {
    if (entry.isNotEmpty || chips.isEmpty) return false;
    entry = chips.removeLast().text;
    return true;
  }

  @override
  bool operator ==(Object other) =>
      other is Recipients &&
      other.entry == entry &&
      other.chips.length == chips.length &&
      Iterable.generate(
        chips.length,
      ).every((i) => other.chips[i].text == chips[i].text);

  @override
  int get hashCode => Object.hash(entry, Object.hashAll(chips));
}

bool _whitespace(int rune) => String.fromCharCode(rune).trim().isEmpty;

/// Splits typed or pasted text into finished addresses and the unfinished rest.
///
/// Commas, semicolons and line breaks separate addresses, except inside a
/// quoted display name or `<...>`. A space also finishes a complete bare
/// address or one ending in `>`, so pasted space-separated lists split too.
(List<String>, String) splitRecipients(String text) {
  final finished = <String>[];
  final current = StringBuffer();
  var quoted = false, escaped = false;
  var depth = 0;
  void finish() {
    final part = current.toString().trim();
    if (part.isNotEmpty) finished.add(part);
    current.clear();
  }

  for (final rune in text.runes) {
    final c = String.fromCharCode(rune);
    if (quoted) {
      current.write(c);
      if (escaped) {
        escaped = false;
      } else if (c == r'\') {
        escaped = true;
      } else if (c == '"') {
        quoted = false;
      }
      continue;
    }
    switch (c) {
      case '"':
        quoted = true;
        current.write(c);
      case '<':
        depth++;
        current.write(c);
      case '>':
        if (depth > 0) depth--;
        current.write(c);
      case ',' || ';' || '\n' || '\r' when depth == 0:
        finish();
      case _
          when _whitespace(rune) &&
              depth == 0 &&
              _complete(current.toString().trim()):
        finish();
      default:
        current.write(c);
    }
  }
  return (finished, current.toString().trimLeft());
}

final _bareBlocked = RegExp(r'[\s<>"()]');

bool _complete(String text) {
  if (text.endsWith('>')) return true;
  final at = text.indexOf('@');
  if (at < 0) return false;
  final local = text.substring(0, at), domain = text.substring(at + 1);
  return local.isNotEmpty &&
      domain.isNotEmpty &&
      !domain.contains('@') &&
      !_bareBlocked.hasMatch(text);
}

/// Keeps a copied column of addresses apart: single-line entries drop line breaks.
String pasteRecipientLines(String text) {
  if (!text.contains(RegExp('[\n\r\t]'))) return text;
  return text
      .replaceAll('\t', ' ')
      .split(RegExp('[\n\r]'))
      .map((line) => line.trim())
      .where((line) => line.isNotEmpty)
      .join(', ');
}

final _address = RegExp(r'^[^\s@<>"(),;:\[\]\\]+@[^\s@<>"(),;:\[\]\\]+$');
final _named = RegExp(r'^(.*?)\s*<([^<>]*)>$');
final _quotedName = RegExp(r'^"(?:[^"\\]|\\.)*"$');
final _phrase = RegExp(r'^[^"<>@,;:\\()\[\]]+$');

/// Whether Send would accept [chip] as exactly one mailbox: a bare address,
/// or a plain or quoted display name followed by `<address>`.
bool recipientValid(String chip) {
  final text = chip.trim();
  if (text.contains(RegExp('[\r\n\x00]'))) return false;
  if (_address.hasMatch(text)) return true;
  final named = _named.firstMatch(text);
  if (named == null) return false;
  final name = named.group(1)!.trim();
  return _address.hasMatch(named.group(2)!) &&
      (name.isEmpty || _quotedName.hasMatch(name) || _phrase.hasMatch(name));
}

/// How a field changed, so typing can coalesce into word-sized undo steps.
sealed class FieldEdit {
  const FieldEdit();

  /// Classifies a single-line field change by comparing its values.
  factory FieldEdit.between(String before, String after) {
    final old = before.runes.length, next = after.runes.length;
    if (next == old + 1 && after.startsWith(before)) {
      return Insert(space: _whitespace(after.runes.last));
    }
    if (next == old + 1) return const Insert(space: false);
    if (next + 1 == old) return const Delete();
    return const OtherEdit();
  }

  bool continues(FieldEdit next) => switch ((this, next)) {
    // A word ends where a space follows it.
    (Insert(space: false), Insert(space: true)) => false,
    (Insert(), Insert()) || (Delete(), Delete()) => true,
    _ => false,
  };
}

class Insert extends FieldEdit {
  const Insert({required this.space});
  final bool space;

  @override
  bool operator ==(Object other) => other is Insert && other.space == space;

  @override
  int get hashCode => space.hashCode;
}

class Delete extends FieldEdit {
  const Delete();

  @override
  bool operator ==(Object other) => other is Delete;

  @override
  int get hashCode => 1;
}

/// Paste, a structural change or chip removal: always its own step.
class OtherEdit extends FieldEdit {
  const OtherEdit();

  @override
  bool operator ==(Object other) => other is OtherEdit;

  @override
  int get hashCode => 2;
}

/// Bounded undo/redo snapshots for one composer field.
class EditHistory<T> {
  EditHistory(this.weight);

  static const steps = 100;
  static const bytes = 4 << 20;
  static const pause = Duration(seconds: 2);

  final int Function(T) weight;
  final _undo = Queue<T>();
  final _redo = <T>[];
  int _bytes = 0;
  (FieldEdit, DateTime)? _open;

  int get length => _undo.length;
  int get size => _bytes;

  /// Saves the state before an edit unless the edit continues the open step.
  void record(FieldEdit edit, DateTime now, T Function() before) {
    final open = _open;
    final continues =
        open != null &&
        now.difference(open.$2) < pause &&
        open.$1.continues(edit);
    for (final state in _redo) {
      _bytes -= weight(state);
    }
    _redo.clear();
    _open = edit is OtherEdit ? null : (edit, now);
    if (!continues) _push(before());
  }

  /// Ends the open step, so the next edit starts a new one.
  void seal() => _open = null;

  T? undo(T Function() current) {
    if (_undo.isEmpty) return null;
    final previous = _undo.removeLast();
    _bytes -= weight(previous);
    final now = current();
    _bytes += weight(now);
    _redo.add(now);
    _open = null;
    return previous;
  }

  T? redo(T Function() current) {
    if (_redo.isEmpty) return null;
    final next = _redo.removeLast();
    _bytes -= weight(next);
    _push(current());
    _open = null;
    return next;
  }

  void _push(T state) {
    _bytes += weight(state);
    _undo.addLast(state);
    while (_undo.length > steps || (_bytes > bytes && _undo.length > 1)) {
      _bytes -= weight(_undo.removeFirst());
    }
  }
}
