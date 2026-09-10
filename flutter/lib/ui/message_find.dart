import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import '../data/message_search.dart';
import '../model/message_find.dart';
import 'icons.dart';

class MessageFindBar extends StatelessWidget {
  const MessageFindBar({
    super.key,
    required this.find,
    required this.query,
    required this.focus,
    required this.close,
    required this.loading,
  });
  final MessageFind find;
  final TextEditingController query;
  final FocusNode focus;
  final VoidCallback close;
  final bool loading;
  @override
  Widget build(BuildContext context) => Semantics(
    container: true,
    explicitChildNodes: true,
    child: CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.enter, shift: true): () =>
            find.next(true),
      },
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 8, 8, 8),
        child: Column(
          children: [
            Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: query,
                    focusNode: focus,
                    decoration: const InputDecoration(
                      labelText: 'Find in message',
                      isDense: true,
                    ),
                    onChanged: find.setQuery,
                    onEditingComplete: () {},
                    onSubmitted: (_) => find.next(),
                  ),
                ),
                IconButton(
                  tooltip: 'Close Find',
                  onPressed: close,
                  icon: const ShepIcon('close'),
                ),
              ],
            ),
            Row(
              children: [
                Expanded(
                  child: Semantics(
                    liveRegion: true,
                    child: Text(
                      loading ? 'Loading message…' : find.error ?? find.status,
                    ),
                  ),
                ),
                if (find.error != null)
                  TextButton(
                    onPressed: find.retry,
                    child: const Text('Retry Find'),
                  ),
                IconButton(
                  tooltip: 'Match case',
                  isSelected: find.matchCase,
                  onPressed: find.toggleCase,
                  icon: const ShepIcon('type'),
                ),
                IconButton(
                  tooltip: 'Previous match',
                  onPressed: find.hits.isEmpty || find.pending
                      ? null
                      : () => find.next(true),
                  icon: const ShepIcon('up'),
                ),
                IconButton(
                  tooltip: 'Next match',
                  onPressed: find.hits.isEmpty || find.pending
                      ? null
                      : () => find.next(),
                  icon: const ShepIcon('down'),
                ),
              ],
            ),
          ],
        ),
      ),
    ),
  );
}

class _HighlightedText extends TextEditingController {
  _HighlightedText(String text) : super(text: text);
  List<SearchHit> hits = [];
  int active = 0, block = 0;
  @override
  TextSpan buildTextSpan({
    required BuildContext context,
    TextStyle? style,
    required bool withComposing,
  }) {
    final scheme = Theme.of(context).colorScheme;
    final children = <TextSpan>[];
    var offset = 0;
    for (var i = 0; i < hits.length; i++) {
      final hit = hits[i];
      if (hit.block != block) continue;
      if (hit.start > offset) {
        children.add(TextSpan(text: text.substring(offset, hit.start)));
      }
      children.add(
        TextSpan(
          text: text.substring(hit.start, hit.end),
          style: TextStyle(
            backgroundColor: i == active
                ? scheme.primary
                : scheme.primaryContainer,
            color: i == active ? scheme.onPrimary : scheme.onPrimaryContainer,
          ),
        ),
      );
      offset = hit.end;
    }
    children.add(TextSpan(text: text.substring(offset)));
    return TextSpan(style: style, children: children);
  }
}

/// Read-only native selection controls with an explicit caret reveal target.
/// Match ranges always refer to the untouched displayed text's UTF-16 offsets.
class SearchableMessageText extends StatefulWidget {
  const SearchableMessageText({
    super.key,
    required this.text,
    required this.block,
    required this.find,
  });
  final String text;
  final int block;
  final MessageFind find;
  @override
  State<SearchableMessageText> createState() => _SearchableMessageTextState();
}

class _SearchableMessageTextState extends State<SearchableMessageText>
    implements TextSelectionGestureDetectorBuilderDelegate {
  @override
  final editableTextKey = GlobalKey<EditableTextState>();
  @override
  bool get forcePressEnabled => false;
  @override
  bool get selectionEnabled => true;
  late final controller = _HighlightedText(widget.text);
  final focus = FocusNode();
  late final gestures = TextSelectionGestureDetectorBuilder(delegate: this);
  int jump = -1;
  @override
  void dispose() {
    controller.dispose();
    focus.dispose();
    super.dispose();
  }

  @override
  void didUpdateWidget(SearchableMessageText old) {
    super.didUpdateWidget(old);
    if (old.text != widget.text) controller.text = widget.text;
  }

  @override
  Widget build(BuildContext context) {
    final find = widget.find;
    controller.hits = find.open ? find.hits : [];
    controller.active = find.active;
    controller.block = widget.block;
    if (find.open &&
        find.hits.isNotEmpty &&
        jump != find.jump &&
        find.hits[find.active].block == widget.block) {
      jump = find.jump;
      final target = find.hits[find.active].start;
      final current = jump;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted && widget.find.open && widget.find.jump == current) {
          final editable = editableTextKey.currentState?.renderEditable;
          if (editable != null) {
            final caret = editable.getLocalRectForCaret(
              TextPosition(offset: target),
            );
            // Reveal the whole line with context, not only the caret's center
            // at the viewport edge. The body itself has no internal scrolling.
            editable.showOnScreen(rect: caret.inflate(32));
          }
        }
      });
    }
    final scheme = Theme.of(context).colorScheme;
    return gestures.buildGestureDetector(
      behavior: HitTestBehavior.translucent,
      child: EditableText(
        key: editableTextKey,
        controller: controller,
        focusNode: focus,
        readOnly: true,
        showCursor: false,
        maxLines: null,
        rendererIgnoresPointer: true,
        style: DefaultTextStyle.of(
          context,
        ).style.copyWith(fontSize: 14, height: 1.8, color: scheme.onSurface),
        cursorColor: scheme.primary,
        backgroundCursorColor: scheme.surface,
        selectionColor: scheme.primaryContainer,
        selectionControls: materialTextSelectionHandleControls,
        showSelectionHandles: true,
        contextMenuBuilder: (context, state) =>
            AdaptiveTextSelectionToolbar.editableText(editableTextState: state),
      ),
    );
  }
}
