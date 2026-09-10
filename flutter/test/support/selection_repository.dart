import 'package:shep_mobile/data/selection.dart';
import 'package:shep_mobile/model/mail.dart';

/// Small synthetic preview transport. Production selection uses SQLite, and
/// native protocol tests exercise the same contract independently of this fake.
class PreviewSelectionRepository implements SelectionRepository {
  PreviewSelectionRepository(this.mail);
  final List<Mail> Function() mail;
  final captures = <String, _Capture>{};
  final aliases = <String, String>{};
  String canonical(String id) => aliases[id] ?? id;
  List<Mail> matching(Map scope) {
    final projection = scope['projection'] as Map? ?? {};
    final words = (scope['query'] as String? ?? '').toLowerCase().trim().split(
      RegExp(r'\s+'),
    );
    final result = mail()
        .map(
          (m) =>
              m.patch(Map<String, Object>.from(projection[m.id] as Map? ?? {})),
        )
        .where(
          (m) =>
              m.folder == scope['folder'] &&
              (scope['account'] == null ||
                  scope['account'] == m.account ||
                  scope['account'] == m.accountId) &&
              (scope['filter'] != 'Unread' || m.unread) &&
              (scope['filter'] != 'Flagged' || m.starred) &&
              words.every(
                (word) => '${m.sender} ${m.subject} ${m.body}'
                    .toLowerCase()
                    .contains(word),
              ),
        )
        .toList();
    result.sort((a, b) {
      final byDate = scope['oldest'] == true
          ? a.date.compareTo(b.date)
          : b.date.compareTo(a.date);
      return byDate == 0 ? a.id.compareTo(b.id) : byDate;
    });
    return result;
  }

  List<Object?> scopeKey(Map scope) => [
    scope['folder'],
    scope['account'],
    scope['query'] ?? '',
    scope['filter'] ?? '',
    scope['oldest'] ?? false,
  ];
  @override
  Future<dynamic> selection(
    Map<String, Object?> command, {
    List<String> observed = const [],
  }) async {
    if (observed.length > 50) throw StateError('One observed page');
    final id = command['id'] as String;
    if (command['kind'] == 'release') {
      captures.remove(id);
      return null;
    }
    if (command['kind'] == 'capture') {
      final previous = captures[id];
      final revision = command['revision'] as int;
      if (previous != null &&
          (previous.frozen || previous.revision >= revision)) {
        throw StateError('Stale capture');
      }
      final scope = command['scope'] as Map;
      final ids = matching(scope).map((m) => m.id).toList();
      captures[id] = _Capture(scopeKey(scope), revision, {
        for (var i = 0; i < ids.length; i++) ids[i]: i,
      }, command['all'] == true ? ids.toSet() : {});
    }
    final capture = captures[id];
    if (capture == null) throw StateError('Missing capture');
    for (final original in capture.positions.keys.toList()) {
      final target = canonical(original);
      if (target == original) continue;
      final ordinal = capture.positions.remove(original)!;
      capture.positions[target] =
          (capture.positions[target] ?? ordinal) < ordinal
          ? capture.positions[target]!
          : ordinal;
      if (capture.selected.remove(original)) capture.selected.add(target);
    }
    if (command['kind'] == 'change') {
      if (capture.frozen ||
          capture.revision != command['expected'] ||
          scopeKey(command['scope'] as Map).toString() !=
              capture.scope.toString()) {
        throw StateError('Stale selection');
      }
      final change = command['change'] as Map;
      if (change['kind'] == 'clear') {
        capture.selected.clear();
      } else if (change['kind'] == 'set') {
        final target = canonical(change['id'] as String);
        if (!capture.positions.containsKey(target)) {
          if (!matching(command['scope'] as Map).any((m) => m.id == target)) {
            throw StateError('Outside scope');
          }
          capture.positions[target] =
              capture.positions.values.fold(-1, (a, b) => a > b ? a : b) + 1;
        }
        if (change['clear_others'] == true) capture.selected.clear();
        if (change['selected'] == true) {
          capture.selected.add(target);
        } else {
          capture.selected.remove(target);
        }
      } else {
        final a = capture.positions[canonical(change['anchor'] as String)];
        final b = capture.positions[canonical(change['target'] as String)];
        if (a == null || b == null) throw StateError('Outside capture');
        if (change['additive'] != true) capture.selected.clear();
        capture.selected.addAll(
          capture.positions.entries
              .where(
                (e) => e.value >= (a < b ? a : b) && e.value <= (a > b ? a : b),
              )
              .map((e) => e.key),
        );
      }
      capture.revision++;
    }
    if (command['kind'] == 'freeze') {
      if (capture.revision != command['expected']) {
        throw StateError('Stale freeze');
      }
      final target = command['target'] as String;
      if (captures.containsKey(target)) throw StateError('Duplicate capture');
      captures[target] = _Capture(capture.scope, 0, {
        for (final entry in capture.positions.entries)
          if (capture.selected.contains(entry.key)) entry.key: entry.value,
      }, Set.of(capture.selected))..frozen = true;
      return snapshot(target, observed);
    }
    if (command['kind'] == 'page') {
      if (capture.revision != command['expected']) {
        throw StateError('Stale page');
      }
      final available = {for (final m in mail()) m.id: m};
      final rows =
          capture.positions.entries
              .where(
                (e) =>
                    capture.selected.contains(e.key) &&
                    available.containsKey(e.key) &&
                    e.value > (command['after'] as int? ?? -1),
              )
              .toList()
            ..sort((a, b) => a.value.compareTo(b.value));
      final page = rows.take(50).map((e) {
        final m = available[e.key]!;
        return {
          'position': e.value,
          'id': m.id,
          'account': m.accountId.isEmpty ? m.account : m.accountId,
          'folder': m.folder,
          'unread': m.unread,
          'starred': m.starred,
        };
      }).toList();
      return {
        'revision': capture.revision,
        'rows': page,
        'next_after': page.length == 50 ? page.last['position'] : null,
      };
    }
    return snapshot(id, observed);
  }

  Map<String, dynamic> snapshot(String id, List<String> observed) {
    final capture = captures[id]!;
    final available = mail()
        .where((m) => capture.selected.contains(m.id))
        .toList();
    final groups = <String, Map<String, dynamic>>{};
    for (final m in available) {
      final account = m.accountId.isEmpty ? m.account : m.accountId;
      final group = groups.putIfAbsent(
        '$account:${m.folder}',
        () => {
          'account': account,
          'folder': m.folder,
          'total': 0,
          'unread': 0,
          'starred': 0,
        },
      );
      group['total'] = (group['total'] as int) + 1;
      if (m.unread) group['unread'] = (group['unread'] as int) + 1;
      if (m.starred) group['starred'] = (group['starred'] as int) + 1;
    }
    return {
      'id': id,
      'revision': capture.revision,
      'frozen': capture.frozen,
      'total': capture.positions.length,
      'selected': capture.selected.length,
      'available': available.length,
      'unread': available.where((m) => m.unread).length,
      'starred': available.where((m) => m.starred).length,
      'groups': groups.values.toList(),
      'visible': observed
          .map(canonical)
          .where(
            (key) =>
                capture.selected.contains(key) &&
                available.any((m) => m.id == key),
          )
          .toList(),
      'positions': {
        for (final original in observed)
          if (capture.positions.containsKey(canonical(original)))
            canonical(original): capture.positions[canonical(original)]!,
      },
      'aliases': aliases,
    };
  }
}

class _Capture {
  _Capture(this.scope, this.revision, this.positions, this.selected);
  final List<Object?> scope;
  int revision;
  bool frozen = false;
  final Map<String, int> positions;
  final Set<String> selected;
}
