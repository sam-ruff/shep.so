import 'dart:async';
import 'package:flutter/foundation.dart';
import '../data/message_search.dart';

typedef SearchText =
    Future<List<SearchHit>> Function(List<String>, String, bool);

/// One active search and one coalesced latest request. Older results and errors
/// cannot replace a new query, message, quote scope or closed search bar.
class MessageFind extends ChangeNotifier {
  MessageFind(this.search, {this.debounce = const Duration(milliseconds: 100)});
  final SearchText search;
  final Duration debounce;
  String query = '', identity = '';
  List<String> blocks = [];
  List<SearchHit> hits = [];
  bool open = false, matchCase = false, pending = false;
  int active = 0, revision = 0, jump = 0;
  String? error;
  bool _running = false, _ready = false, _disposed = false;
  Timer? _timer;
  String get status => pending
      ? 'Searching…'
      : query.isEmpty
      ? 'Search this message'
      : hits.isEmpty
      ? 'No matches'
      : '${active + 1} of ${hits.length}';
  void setSource(String id, List<String> next) {
    if (identity == id && listEquals(blocks, next)) return;
    identity = id;
    blocks = List.of(next);
    _invalidate();
  }

  void show() {
    open = true;
    _invalidate();
  }

  void close() {
    open = false;
    _invalidate();
  }

  void setQuery(String value) {
    query = value;
    _invalidate();
  }

  void toggleCase() {
    matchCase = !matchCase;
    _invalidate();
  }

  void retry() => _invalidate();
  void next([bool previous = false]) {
    if (hits.isEmpty || pending) return;
    active = (active + (previous ? -1 : 1)) % hits.length;
    jump++;
    notifyListeners();
  }

  void _invalidate() {
    revision++;
    hits = [];
    active = 0;
    error = null;
    _timer?.cancel();
    _ready = false;
    pending = open && query.isNotEmpty;
    if (pending) {
      _timer = Timer(debounce, () {
        _ready = true;
        _pump();
      });
    }
    notifyListeners();
  }

  Future<void> _pump() async {
    if (_disposed || _running || !_ready || !open) return;
    _ready = false;
    _running = true;
    final version = revision;
    try {
      final result = await search(List.of(blocks), query, matchCase);
      if (!_disposed && open && version == revision) {
        hits = result;
        active = 0;
        pending = false;
        error = null;
        jump++;
        notifyListeners();
      }
    } catch (_) {
      if (!_disposed && open && version == revision) {
        pending = false;
        error = 'Could not search this message. Retry Find.';
        notifyListeners();
      }
    } finally {
      _running = false;
      if (!_disposed && _ready) unawaited(_pump());
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    super.dispose();
  }
}
