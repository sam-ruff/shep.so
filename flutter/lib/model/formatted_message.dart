import 'dart:async';
import 'dart:math';
import 'package:flutter/foundation.dart';
import '../data/formatted_message.dart';
import 'message_find.dart';

/// Per-reader lifetime. Results and runtime messages are bound to a generation;
/// preparing a new message never makes an old frame its action source.
class FormattedMessage extends ChangeNotifier {
  FormattedMessage(this.repository, this.id);
  final FormattedMessageRepository repository;
  final String id;
  String generation = '';
  PreparedMessage? prepared;
  String? error;
  bool loading = false, plain = false, ready = false, hasQuotes = false;
  List<String> blocks = [];
  int layout = 0, _jump = -1;
  bool _disposed = false, _dark = false, _quotes = false;
  String _configuration = '', _highlight = '';
  final commands = StreamController<Map<String, Object?>>.broadcast(sync: true);
  bool get html => prepared?.document != null && error == null && !plain;
  Future<void> load({required bool dark, required bool quotes}) async {
    final random = Random.secure();
    generation = List.generate(
      24,
      (_) => random.nextInt(256).toRadixString(16).padLeft(2, '0'),
    ).join();
    final current = generation;
    _dark = dark;
    _quotes = quotes;
    loading = true;
    prepared = null;
    error = null;
    ready = false;
    layout = 0;
    blocks = [];
    hasQuotes = false;
    _configuration = '';
    _highlight = '';
    _jump = -1;
    notifyListeners();
    try {
      final result = await repository.formattedMessage(
        id,
        generation: current,
        dark: dark,
        quotes: quotes,
      );
      if (!_disposed && generation == current) prepared = result;
    } catch (_) {
      if (!_disposed && generation == current) {
        error =
            'Could not format this cached message. Use plain text or retry.';
      }
    } finally {
      if (!_disposed && generation == current) {
        loading = false;
        notifyListeners();
      }
    }
  }

  void setPlain(bool value) {
    plain = value;
    notifyListeners();
  }

  void displayError(String expected) {
    if (_disposed || generation != expected) return;
    error = 'Could not display formatted mail. Use plain text or retry.';
    ready = false;
    notifyListeners();
  }

  bool receive(Map<String, dynamic> value) {
    if (_disposed || value['generation'] != generation) return false;
    switch (value['type']) {
      case 'ready':
        ready = true;
        _configuration = '';
        configure(dark: _dark, quotes: _quotes);
        notifyListeners();
      case 'content':
        final revision = value['layout'], text = value['blocks'];
        if (revision is! int ||
            revision <= layout ||
            text is! List ||
            text.any((v) => v is! String)) {
          return false;
        }
        layout = revision;
        blocks = text.cast<String>();
        hasQuotes = value['hasQuotes'] == true;
        _highlight = '';
        notifyListeners();
      case 'error':
        displayError(generation);
    }
    return true;
  }

  void configure({required bool dark, required bool quotes}) {
    _dark = dark;
    _quotes = quotes;
    final key = '$dark:$quotes';
    if (!ready || _disposed || key == _configuration) return;
    _configuration = key;
    commands.add({
      'type': 'configure',
      'generation': generation,
      'dark': dark,
      'quotes': quotes,
      'shortcuts': ['Control+f', 'Meta+f', 'Escape'],
    });
  }

  /// Returns whether the outer Flutter scroll view should reveal the document.
  bool highlight(MessageFind find) {
    if (!ready || !html || layout == 0 || _disposed) return false;
    final key =
        '$layout:${find.revision}:${find.active}:${find.open}:${find.pending}';
    if (key == _highlight) return false;
    _highlight = key;
    final jump =
        find.open &&
        !find.pending &&
        find.hits.isNotEmpty &&
        find.jump != _jump;
    if (jump) _jump = find.jump;
    commands.add({
      'type': 'highlight',
      'generation': generation,
      'layout': layout,
      'revision': find.revision,
      'hits': find.open ? find.hits.map((h) => h.toJson()).toList() : [],
      'active': find.active,
      'jump': jump,
    });
    return jump;
  }

  @override
  void dispose() {
    _disposed = true;
    unawaited(commands.close());
    super.dispose();
  }
}
