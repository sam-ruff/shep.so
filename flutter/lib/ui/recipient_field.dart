import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../model/recipients.dart';
import 'icons.dart';
import 'theme.dart';

/// Leads the entry while chips exist. Soft keyboards often send no key event
/// for Backspace in an empty field, but they do delete this character, which
/// is how the last chip reopens for editing.
const recipientSentinel = '\u{200B}';

enum HistoryStep { undo, redo }

/// Owns one recipient field's chips, entry text and undo history.
class RecipientsController extends ChangeNotifier {
  RecipientsController(String value, {DateTime Function()? clock})
    : _recipients = Recipients.parse(value),
      _clock = clock ?? DateTime.now {
    entry.value = _entryValue(null);
  }

  final DateTime Function() _clock;
  final entry = TextEditingController();
  final history = EditHistory<Recipients>((state) => state.weight);
  Recipients _recipients;

  List<RecipientChip> get chips => List.unmodifiable(_recipients.chips);

  /// The draft string: chips joined with ", ", then any unfinished entry.
  String get value => _recipients.value;

  /// Same as [value], so callers can treat this like a text controller.
  String get text => value;

  /// Applies the raw entry text after the user typed, pasted or deleted.
  bool edit(String raw) {
    final previous = _recipients;
    final hadSentinel = previous.chips.isNotEmpty;
    final text = raw.replaceAll(recipientSentinel, '');
    if (hadSentinel && raw.isEmpty && previous.entry.isEmpty) {
      final next = previous.copy()..reopenLast();
      return _apply(next, const OtherEdit(), cursorAtEnd: true);
    }
    if (text.contains(RegExp('[\n\r\t]'))) {
      // A single typed line break is Enter on a hardware keyboard.
      final typedEnter =
          '\n'.allMatches(text).length == 1 &&
          !text.contains(RegExp('[\r\t]')) &&
          text.replaceFirst('\n', '') == previous.entry;
      if (typedEnter) return commit();
      final next = previous.copy()..input(pasteRecipientLines(text));
      return _apply(next, const OtherEdit(), cursorAtEnd: true);
    }
    final next = previous.copy()..input(text);
    final split = next.chips.length != previous.chips.length;
    final edit = split
        ? const OtherEdit()
        : FieldEdit.between(previous.entry, next.entry);
    return _apply(next, edit, cursorAtEnd: split, raw: raw);
  }

  /// Turns the unfinished entry into a chip.
  bool commit() {
    final next = _recipients.copy();
    if (!next.commit()) {
      _sync(cursorAtEnd: true);
      return false;
    }
    _recipients = next;
    // Undo after a commit returns to the typed text, not the chip.
    history.seal();
    _sync(cursorAtEnd: true);
    notifyListeners();
    return true;
  }

  bool remove(int index, String text) {
    final next = _recipients.copy();
    if (!next.remove(index, text)) return false;
    return _apply(next, const OtherEdit(), cursorAtEnd: true);
  }

  /// Undoes or redoes this field. Returns whether anything changed.
  bool step(HistoryStep step) {
    final current = _recipients;
    final state = switch (step) {
      HistoryStep.undo => history.undo(() => current),
      HistoryStep.redo => history.redo(() => current),
    };
    if (state == null) return false;
    _recipients = state;
    _sync(cursorAtEnd: true);
    notifyListeners();
    return true;
  }

  bool _apply(
    Recipients next,
    FieldEdit edit, {
    required bool cursorAtEnd,
    String? raw,
  }) {
    final before = _recipients;
    if (next == before) {
      _sync(cursorAtEnd: cursorAtEnd, raw: raw);
      return false;
    }
    history.record(edit, _clock(), () => before);
    _recipients = next;
    _sync(cursorAtEnd: cursorAtEnd, raw: raw);
    notifyListeners();
    return true;
  }

  String get _prefix => _recipients.chips.isEmpty ? '' : recipientSentinel;

  TextEditingValue _entryValue(int? offset) {
    final text = _prefix + _recipients.entry;
    final at = (offset ?? text.length).clamp(_prefix.length, text.length);
    return TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: at),
    );
  }

  /// Rewrites the entry only when it differs, so IME composition survives.
  void _sync({required bool cursorAtEnd, String? raw}) {
    final text = _prefix + _recipients.entry;
    if (entry.text == text) return;
    int? offset;
    if (!cursorAtEnd && raw != null) {
      final selection = entry.selection;
      final cursor = selection.isValid
          ? selection.extentOffset.clamp(0, raw.length)
          : raw.length;
      final before = raw.substring(0, cursor);
      offset = before.replaceAll(recipientSentinel, '').length + _prefix.length;
    }
    entry.value = _entryValue(offset);
  }

  @override
  void dispose() {
    entry.dispose();
    super.dispose();
  }
}

class _HistoryIntent extends Intent {
  const _HistoryIntent(this.step);
  final HistoryStep step;
}

Map<ShortcutActivator, Intent> _historyShortcuts() {
  final apple =
      defaultTargetPlatform == TargetPlatform.macOS ||
      defaultTargetPlatform == TargetPlatform.iOS;
  SingleActivator key(LogicalKeyboardKey key, {bool shift = false}) =>
      SingleActivator(key, control: !apple, meta: apple, shift: shift);
  return {
    key(LogicalKeyboardKey.keyZ): const _HistoryIntent(HistoryStep.undo),
    key(LogicalKeyboardKey.keyZ, shift: true): const _HistoryIntent(
      HistoryStep.redo,
    ),
    key(LogicalKeyboardKey.keyY): const _HistoryIntent(HistoryStep.redo),
  };
}

/// A To/Cc/Bcc field that shows finished addresses as removable chips.
class RecipientField extends StatefulWidget {
  const RecipientField({
    super.key,
    required this.controller,
    required this.label,
    required this.onChanged,
    this.readOnly = false,
  });

  final RecipientsController controller;
  final String label;
  final bool readOnly;

  /// Called after a user change to the field's value.
  final VoidCallback onChanged;

  @override
  State<RecipientField> createState() => _RecipientFieldState();
}

class _RecipientFieldState extends State<RecipientField> {
  final focus = FocusNode();

  @override
  void initState() {
    super.initState();
    focus.addListener(focusChanged);
    widget.controller.addListener(rebuild);
  }

  @override
  void didUpdateWidget(RecipientField old) {
    super.didUpdateWidget(old);
    if (old.controller != widget.controller) {
      old.controller.removeListener(rebuild);
      widget.controller.addListener(rebuild);
    }
  }

  void rebuild() {
    if (mounted) setState(() {});
  }

  void focusChanged() {
    if (!focus.hasFocus && !widget.readOnly && widget.controller.commit()) {
      widget.onChanged();
    }
    rebuild();
  }

  void changed(bool changed) {
    if (changed) widget.onChanged();
  }

  @override
  void dispose() {
    widget.controller.removeListener(rebuild);
    focus.removeListener(focusChanged);
    focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final controller = widget.controller;
    final chips = controller.chips;
    final entry = CallbackShortcuts(
      bindings: {
        for (final MapEntry(:key, :value) in _historyShortcuts().entries)
          key: () {
            if (widget.readOnly) return;
            changed(controller.step((value as _HistoryIntent).step));
          },
      },
      child: TextField(
        controller: controller.entry,
        focusNode: focus,
        readOnly: widget.readOnly,
        // Multi-line input keeps pasted line breaks so a column of addresses
        // splits; the entry itself never keeps them.
        minLines: 1,
        maxLines: null,
        keyboardType: TextInputType.emailAddress,
        textInputAction: TextInputAction.done,
        autocorrect: false,
        enableSuggestions: false,
        onChanged: (raw) => changed(controller.edit(raw)),
        onSubmitted: (_) => changed(controller.commit()),
        // Keep the keyboard open for the next address.
        onEditingComplete: () {},
        // The surrounding InputDecorator draws the field outline.
        decoration:
            InputDecoration.collapsed(
              hintText: chips.isEmpty ? 'name@example.com' : null,
            ).copyWith(
              enabledBorder: InputBorder.none,
              focusedBorder: InputBorder.none,
              disabledBorder: InputBorder.none,
            ),
      ),
    );
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onTap: widget.readOnly ? null : focus.requestFocus,
      child: InputDecorator(
        isFocused: focus.hasFocus,
        isEmpty: false,
        decoration: InputDecoration(
          labelText: widget.label,
          contentPadding: const EdgeInsets.fromLTRB(12, 17, 8, 5),
        ),
        child: LayoutBuilder(
          builder: (context, constraints) => Wrap(
            crossAxisAlignment: WrapCrossAlignment.center,
            spacing: 4,
            children: [
              for (final (index, chip) in chips.indexed)
                _RecipientChip(
                  key: ValueKey((index, chip.text)),
                  chip: chip,
                  onRemove: widget.readOnly
                      ? null
                      : () => changed(controller.remove(index, chip.text)),
                ),
              // Keyed so adding a chip keeps the entry's focus and IME state.
              ConstrainedBox(
                key: const ValueKey('entry'),
                constraints: BoxConstraints(
                  minWidth: constraints.maxWidth < 160
                      ? constraints.maxWidth
                      : 160,
                  maxWidth: constraints.maxWidth,
                  minHeight: 44,
                ),
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 4,
                    vertical: 12,
                  ),
                  child: IntrinsicWidth(child: entry),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _RecipientChip extends StatelessWidget {
  const _RecipientChip({super.key, required this.chip, required this.onRemove});

  final RecipientChip chip;
  final VoidCallback? onRemove;

  @override
  Widget build(BuildContext context) {
    final colors = ShepColors.of(context);
    final invalid = !chip.valid;
    final foreground = invalid ? colors.flag : colors.text;
    return Semantics(
      container: true,
      label: invalid ? '${chip.text}, not a valid address' : chip.text,
      child: Stack(
        children: [
          // The outline is 32px tall; the row and its x target stay 44px.
          Positioned.fill(
            top: 6,
            bottom: 6,
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: invalid ? colors.errorSurface : colors.subtle,
                border: Border.all(
                  color: invalid ? colors.flag : colors.border,
                ),
                borderRadius: BorderRadius.circular(ShepRadius.ghost),
              ),
            ),
          ),
          SizedBox(
            height: 44,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                const SizedBox(width: 10),
                if (invalid) ...[
                  ShepIcon('info', size: 14, color: colors.flag),
                  const SizedBox(width: 4),
                ],
                ExcludeSemantics(
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 240),
                    child: Text(
                      chip.text,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: ShepText.secondary,
                        color: foreground,
                        decoration: invalid ? TextDecoration.underline : null,
                        decorationStyle: TextDecorationStyle.wavy,
                        decorationColor: colors.flag,
                      ),
                    ),
                  ),
                ),
                Semantics(
                  container: true,
                  button: true,
                  enabled: onRemove != null,
                  label: 'Remove ${chip.text}',
                  child: Tooltip(
                    message: 'Remove ${chip.text}',
                    excludeFromSemantics: true,
                    child: InkResponse(
                      onTap: onRemove,
                      radius: 18,
                      child: SizedBox(
                        width: 36,
                        height: 44,
                        child: Center(
                          child: ShepIcon(
                            'close',
                            size: 14,
                            color: invalid ? colors.flag : colors.muted,
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
