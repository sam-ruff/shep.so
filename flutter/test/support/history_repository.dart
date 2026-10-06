import 'dart:async';
import 'package:shep_mobile/data/groups.dart';
import 'preview_repository.dart';

class HistoryRepository extends PreviewRepository {
  HistoryRepository({int groups = 65, int items = 125})
    : super(delay: Duration.zero) {
    for (var n = 0; n < groups; n++) {
      final id = 'group-${n.toString().padLeft(3, '0')}';
      records[id] = {
        'id': id,
        'seq': n + 1,
        'state': 'finished',
        'revision': 1,
        'total': items,
        'action': {'kind': 'move', 'folder': id},
        'groups': <Map<String, dynamic>>[],
      };
      members[id] = List.generate(
        items,
        (p) => <String, dynamic>{
          'position': p,
          'mail': '$id-mail-$p',
          'subject': '$id message ${p + 1}',
          'sender': 'Fixture',
          'state': p.isEven ? 'failed' : 'uncertain',
          'reason': 'Fixture needs review',
          'folder': 'Inbox',
          'account': 'fixture',
        },
      );
    }
  }
  final records = <String, Map<String, dynamic>>{};
  final members = <String, List<Map<String, dynamic>>>{};
  final calls = <Map<String, Object?>>[];
  final decisions = <(String, String, int, String)>[];
  String? heldDecisionKind, heldDecisionJob;
  Completer<void>? decisionHold;
  final decisionStarted = Completer<void>();
  String? heldItemsJob;
  Completer<void>? itemsHold;
  Completer<void>? historyHold;
  Completer<void>? ownerHold;
  bool _ownerHeld = false, reportRunnable = false;
  int stepCalls = 0;
  bool _historyHeld = false;
  bool _itemsHeld = false, failItems = false, failHistory = false;

  Map<String, dynamic> summary(String id) => {
    ...records[id]!,
    'counts': {
      for (final state in {...members[id]!.map((r) => r['state'] as String)})
        state: members[id]!.where((r) => r['state'] == state).length,
    },
  };

  @override
  Future<dynamic> groups(Map<String, Object?> command) async {
    calls.add(Map.of(command));
    final kind = command['kind'];
    if (kind == 'history') {
      if (failHistory && command.containsKey('before')) {
        failHistory = false;
        throw StateError('History read fixture');
      }
      final listed = records.keys.toList()
        ..sort(
          (a, b) =>
              (records[b]!['seq'] as int).compareTo(records[a]!['seq'] as int),
        );
      final before = command['before'] as int?;
      final page = listed
          .where(
            (id) => before == null || (records[id]!['seq'] as int) < before,
          )
          .take(20)
          .toList();
      final first = page.isEmpty
          ? (before == null ? 2147483647 : before - 1)
          : records[page.first]!['seq'] as int;
      final newer = listed
          .where((id) => (records[id]!['seq'] as int) > first)
          .toList()
          .reversed
          .take(21)
          .toList();
      final active = listed
          .where(
            (id) => {
              'running',
              'undoing',
              'paused',
            }.contains(records[id]!['state']),
          )
          .take(20)
          .toList();
      final attention = listed
          .where(
            (id) => members[id]!.any(
              (r) => groupAttentionStates.contains(r['state']),
            ),
          )
          .toList();
      final result = {
        'jobs': page.map(summary).toList(),
        'next_before':
            page.isNotEmpty &&
                listed.any(
                  (id) =>
                      (records[id]!['seq'] as int) <
                      (records[page.last]!['seq'] as int),
                )
            ? records[page.last]!['seq']
            : null,
        'has_previous': newer.isNotEmpty,
        'previous_before': newer.length > 20
            ? records[newer.last]!['seq']
            : null,
        'active': active.map(summary).toList(),
        'attention': attention.fold<int>(
          0,
          (n, id) =>
              n +
              members[id]!
                  .where((r) => groupAttentionStates.contains(r['state']))
                  .length,
        ),
        'attention_job': attention.isEmpty ? null : summary(attention.last),
        'tracked': (command['tracked'] as List? ?? [])
            .where(records.containsKey)
            .map((id) => summary(id as String))
            .toList(),
        'runnable': reportRunnable,
      };
      if (historyHold != null &&
          command.containsKey('before') &&
          !_historyHeld) {
        _historyHeld = true;
        await historyHold!.future;
      }
      if (ownerHold != null && !command.containsKey('before') && !_ownerHeld) {
        _ownerHeld = true;
        await ownerHold!.future;
      }
      return result;
    }
    final id = command['id'] as String;
    if (kind == 'items') {
      if (failItems) {
        failItems = false;
        throw StateError('Item read fixture');
      }
      final after = command['after'] as int? ?? -1;
      final selected = members[id]!
          .where((r) => (r['position'] as int) > after)
          .take(50)
          .map((r) => Map<String, dynamic>.of(r))
          .toList();
      final first = selected.isEmpty
          ? after + 1
          : selected.first['position'] as int;
      final earlier = members[id]!
          .where((r) => (r['position'] as int) < first)
          .toList()
          .reversed
          .take(51)
          .toList();
      final result = {
        'rows': selected,
        'next_after':
            selected.isNotEmpty &&
                members[id]!.any(
                  (r) =>
                      (r['position'] as int) >
                      (selected.last['position'] as int),
                )
            ? selected.last['position']
            : null,
        'has_previous': earlier.isNotEmpty,
        'previous_after': earlier.length > 50 ? earlier.last['position'] : null,
      };
      if (heldItemsJob == id && itemsHold != null && !_itemsHeld) {
        _itemsHeld = true;
        await itemsHold!.future;
      }
      return result;
    }
    if (kind == 'retry' || kind == 'accept') {
      final position = command['position'] as int;
      final item = members[id]!.firstWhere((r) => r['position'] == position);
      decisions.add((kind as String, id, position, item['mail'] as String));
      item['state'] = kind == 'retry' ? 'pending' : 'accepted';
      records[id]!['revision'] = (records[id]!['revision'] as int) + 1;
      if (kind == 'retry') records[id]!['state'] = 'running';
      final result = summary(id);
      if (kind == heldDecisionKind &&
          id == heldDecisionJob &&
          decisionHold != null) {
        if (!decisionStarted.isCompleted) decisionStarted.complete();
        await decisionHold!.future;
      }
      return result;
    }
    if (kind == 'pause') records[id]!['state'] = 'paused';
    if (kind == 'resume') records[id]!['state'] = 'running';
    if (kind == 'undo') records[id]!['undo'] = true;
    if (kind == 'remove') {
      records.remove(id);
      members.remove(id);
      return {'removed': true};
    }
    records[id]!['revision'] = (records[id]!['revision'] as int) + 1;
    return summary(id);
  }

  @override
  Future<Map<String, dynamic>> groupStep() async {
    stepCalls++;
    return {'idle': true};
  }
}
