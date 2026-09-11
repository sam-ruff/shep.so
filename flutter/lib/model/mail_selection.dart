import 'dart:async';
import 'dart:collection';
import '../data/selection.dart';
import 'mail.dart' show newDraftIdentity;

class _Gesture {
  _Gesture(this.change, {this.all = false, this.range});
  final Map<String, Object?> change;
  final bool all;
  final (int, int)? range;
  bool failed = false;
}

/// A bounded gesture queue and rendered-row observations, never a full inbox's
/// selected-ID set. The captured membership belongs to the native repository.
class MailSelection {
  MailSelection({
    required this.repository,
    required this.scope,
    required this.currentCount,
    required this.changed,
  });
  final SelectionRepository repository;
  final Map<String, Object?> Function() scope;
  final int Function() currentCount;
  final void Function() changed;
  bool mode = false, _running = false, _disposed = false;
  String? _id, _backend, anchor, error, warning;
  int? _anchorPosition;
  SelectionSnapshot? snapshot;
  final _gestures = Queue<_Gesture>();
  final _watched = <String, int>{};
  final _dirty = <String, int>{};
  int _observationGeneration = 0;
  final _positions = <String, int>{};
  final _chosen = <String>{};
  final _aliases = <String, String>{};
  String _canonical(String id) => _aliases[id] ?? id;
  bool get pending => mode && (snapshot == null || _gestures.isNotEmpty);
  bool get ready => mode && !pending && error == null && count > 0;
  int get count {
    var result = snapshot?.selected ?? 0;
    final chosen = Set<String>.of(_chosen);
    for (final gesture in _gestures) {
      if (gesture.failed) continue;
      final change = gesture.change;
      if (gesture.all) {
        result = currentCount();
        chosen.addAll({..._watched.keys, ..._gestureIds}.map(_canonical));
      } else if (change['kind'] == 'clear') {
        result = 0;
        chosen.clear();
      } else if (change['kind'] == 'set') {
        if (change['clear_others'] == true) {
          result = 0;
          chosen.clear();
        }
        final id = _canonical(change['id'] as String);
        if (change['selected'] == true) {
          if (chosen.add(id)) result++;
        } else if (chosen.remove(id)) {
          result--;
        }
      } else if (gesture.range case (final int start, final int end)) {
        if (change['additive'] != true) {
          result = end - start + 1;
          chosen.clear();
        }
        for (final entry in _positions.entries) {
          if (entry.value >= start && entry.value <= end) {
            chosen.add(entry.key);
          }
        }
      }
    }
    // A literal bound: dart2js shifts are 32-bit, so `1 << 53` would be 0.
    return result.clamp(0, 9007199254740991);
  }

  bool selected(String id) {
    id = _canonical(id);
    var value = _chosen.contains(id);
    for (final gesture in _gestures) {
      if (gesture.failed) continue;
      final change = gesture.change;
      if (gesture.all) {
        value = true;
      } else if (change['kind'] == 'clear') {
        value = false;
      } else if (change['kind'] == 'set') {
        if (change['clear_others'] == true) value = false;
        if (_canonical(change['id'] as String) == id) {
          value = change['selected'] == true;
        }
      } else if (gesture.range case (final int start, final int end)) {
        if (change['additive'] != true) value = false;
        final position = _positions[id];
        if (position != null && position >= start && position <= end) {
          value = true;
        }
      } else if (change['kind'] == 'range') {
        if (change['additive'] != true) value = false;
        if (id == _canonical(change['target'] as String) ||
            id == _canonical(change['anchor'] as String)) {
          value = true;
        }
      }
    }
    return mode && value;
  }

  void watch(String id) {
    _watched[id] = (_watched[id] ?? 0) + 1;
    _markDirty([id]);
    scheduleMicrotask(_pump);
  }

  void unwatch(String id) {
    final remaining = (_watched[id] ?? 1) - 1;
    if (remaining > 0) {
      _watched[id] = remaining;
    } else {
      _watched.remove(id);
      _dirty.remove(id);
      _prune();
    }
  }

  void _markDirty(Iterable<String> ids) {
    final generation = ++_observationGeneration;
    for (final id in ids) {
      _dirty[id] = generation;
    }
  }

  Set<String> get _gestureIds => {
    for (final gesture in _gestures)
      for (final key in ['id', 'anchor', 'target'])
        if (gesture.change[key] case final String id) id,
  };

  void _prune() {
    final needed = {
      ..._watched.keys.map(_canonical),
      ..._gestureIds.map(_canonical),
      if (anchor != null) _canonical(anchor!),
    };
    _positions.removeWhere((id, _) => !needed.contains(id));
    _chosen.removeWhere((id) => !needed.contains(id));
    _aliases.removeWhere((_, target) => !needed.contains(target));
  }

  void start() {
    if (mode || _disposed) return;
    mode = true;
    _id = newDraftIdentity();
    snapshot = null;
    error = null;
    _markDirty(_watched.keys);
    changed();
    unawaited(_pump());
  }

  void done() {
    mode = false;
    _id = null;
    anchor = null;
    _anchorPosition = null;
    snapshot = null;
    error = null;
    warning = null;
    _gestures.clear();
    _chosen.clear();
    _positions.clear();
    _aliases.clear();
    changed();
    unawaited(_pump());
  }

  void all() {
    start();
    _enqueue(_Gesture({}, all: true));
  }

  void clear() {
    if (_enqueue(_Gesture({'kind': 'clear'}))) {
      anchor = null;
      _anchorPosition = null;
    }
  }

  void toggle(String id, {bool clearOthers = false}) {
    final next = clearOthers || !selected(id);
    start();
    if (_enqueue(
      _Gesture({
        'kind': 'set',
        'id': id,
        'selected': next,
        'clear_others': clearOthers,
      }),
    )) {
      anchor = id;
      _anchorPosition = _positions[_canonical(id)];
    }
  }

  void range(String id, {bool additive = false}) {
    if (!mode || anchor == null) {
      toggle(id, clearOthers: !additive);
      return;
    }
    final from = _positions[_canonical(anchor!)] ?? _anchorPosition;
    final to = _positions[_canonical(id)];
    _enqueue(
      _Gesture(
        {'kind': 'range', 'anchor': anchor, 'target': id, 'additive': additive},
        range: from == null || to == null
            ? null
            : (from < to ? from : to, from > to ? from : to),
      ),
    );
  }

  void refresh() {
    if (!mode) return;
    _markDirty(_watched.keys);
    unawaited(_pump());
  }

  void retry() {
    error = null;
    if (_gestures.isNotEmpty) _gestures.first.failed = false;
    changed();
    unawaited(_pump());
  }

  bool _enqueue(_Gesture gesture) {
    if (!mode) return false;
    if (_gestures.length >= 32) {
      warning = 'Selection is catching up. Retry that click after it finishes.';
      changed();
      return false;
    }
    warning = null;
    _gestures.add(gesture);
    changed();
    unawaited(_pump());
    return true;
  }

  void _accept(SelectionSnapshot result, Map<String, int?> observed) {
    snapshot = result;
    _aliases.addAll(result.aliases);
    for (final id in observed.keys) {
      final target = _canonical(id);
      _chosen.remove(target);
      _positions.remove(target);
      if (result.visible.contains(target)) _chosen.add(target);
      if (result.positions[target] case final position?) {
        _positions[target] = position;
      }
      if (_dirty[id] == observed[id]) _dirty.remove(id);
    }
    if (anchor != null) {
      _anchorPosition = _positions[_canonical(anchor!)] ?? _anchorPosition;
    }
    _prune();
  }

  Future<void> _pump() async {
    if (_running) return;
    _running = true;
    try {
      while (true) {
        if (_backend != null && _backend != _id) {
          final old = _backend!;
          try {
            await repository.selection({'kind': 'release', 'id': old});
            _backend = null;
          } catch (_) {
            if (!_disposed) {
              error =
                  'Could not close the previous selection. Retry to select messages.';
              changed();
            }
            break;
          }
          continue;
        }
        if (_disposed || !mode || error != null) break;
        final id = _id!;
        final gesture = snapshot == null || _gestures.isEmpty
            ? null
            : _gestures.first;
        if (snapshot != null && gesture == null && _dirty.isEmpty) break;
        final observed =
            (gesture != null || snapshot == null
                    ? {..._gestureIds, ..._watched.keys}
                    : _dirty.keys)
                .take(50)
                .toList();
        final observationVersions = {for (final id in observed) id: _dirty[id]};
        final command = snapshot == null || gesture?.all == true
            ? <String, Object?>{
                'kind': 'capture',
                'id': id,
                'revision': snapshot == null ? 0 : snapshot!.revision + 1,
                'scope': scope(),
                'all': gesture?.all == true,
              }
            : gesture == null
            ? <String, Object?>{'kind': 'observe', 'id': id}
            : <String, Object?>{
                'kind': 'change',
                'id': id,
                'expected': snapshot!.revision,
                'scope': scope(),
                'change': gesture.change,
              };
        // The token may have committed even if the bridge loses its reply.
        _backend = id;
        try {
          SelectionSnapshot result;
          try {
            result = SelectionSnapshot(
              await repository.selection(command, observed: observed)
                  as Map<String, dynamic>,
            );
          } catch (_) {
            final recovered = SelectionSnapshot(
              await repository.selection({
                    'kind': 'observe',
                    'id': id,
                  }, observed: observed)
                  as Map<String, dynamic>,
            );
            final committedRevision = command['kind'] == 'capture'
                ? command['revision'] as int
                : command['kind'] == 'change'
                ? (command['expected'] as int) + 1
                : snapshot!.revision;
            // One controller owns this unpredictable token and serializes all
            // writes. Exactly the next revision proves its last request committed.
            if (recovered.revision != committedRevision) rethrow;
            result = recovered;
          }
          if (_id != id || _disposed) continue;
          if (gesture != null) {
            _gestures.removeFirst();
            _markDirty(_watched.keys.where((id) => !observed.contains(id)));
          }
          _accept(result, observationVersions);
          changed();
        } catch (_) {
          if (_id != id || _disposed) continue;
          gesture?.failed = true;
          error =
              'Could not update the selection. Retry, or choose Done and select again.';
          changed();
          break;
        }
      }
    } finally {
      _running = false;
    }
  }

  void dispose() {
    _disposed = true;
    done();
  }
}
