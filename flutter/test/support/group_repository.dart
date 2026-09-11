import 'dart:async';
import 'package:shep_mobile/data/groups.dart';
import 'package:shep_mobile/model/mail.dart';
import 'selection_repository.dart';

class _Item {
  _Item(this.position, this.mail, this.folder, this.unread, this.starred);
  final int position;
  final String mail, folder;
  final bool unread, starred;
  String state = 'pending';
  String? reason;
  Map<String, Object?>? applied;
  Map<String, Object?>? before;
}

class _Job {
  _Job(this.seq, this.id, this.action, this.fields);
  final int seq;
  final String id;
  final Map<String, Object?> action, fields;
  String state = 'staging';
  bool undo = false;
  int approved = 0, undone = 0, revision = 0;
  final items = <_Item>[];
  String? error;
  int count(String state) => items.where((i) => i.state == state).length;
}

/// Synthetic durable journal mirroring the Rust contract for previews and
/// host tests: frozen membership, one owned step, receipts, Undo, bounded
/// History and injected failed/unconfirmed outcomes. Production uses SQLite.
class PreviewGroupRepository implements GroupRepository {
  PreviewGroupRepository({
    required this.selection,
    required this.mail,
    required this.mutate,
  });
  final PreviewSelectionRepository selection;
  final List<Mail> Function() mail;
  final Future<void> Function(String id, Map<String, Object> fields) mutate;
  final jobs = <String, _Job>{};
  final intents = <String, Map<String, int>>{};
  final calls = <String>[];
  int _seq = 0, clock = 0;

  /// Injection: the next step for these mail ids ends with this outcome.
  final outcomes = <String, String>{};
  Completer<void>? hold;
  Completer<void>? stepStarted;
  bool stepping = false;

  void recordIntent(String id, Iterable<String> fields) {
    clock++;
    for (final field in fields) {
      intents.putIfAbsent(id, () => {})[field] = clock;
    }
  }

  /// Approved intent applied per field, newest job first, like the Rust
  /// paging projection. Undoing items paint their baseline.
  List<Mail> projected(List<Mail> rows) {
    final active = jobs.values.where(
      (j) =>
          j.state == 'running' || j.state == 'paused' || j.state == 'undoing',
    );
    if (active.isEmpty) return rows;
    final intent = <String, Map<String, Object>>{};
    for (final job in active.toList()..sort((a, b) => a.seq.compareTo(b.seq))) {
      for (final item in job.items) {
        Map<String, Object>? fields;
        if (item.state == 'pending' || item.state == 'sending') {
          fields = Map<String, Object>.from(
            job.fields.map((k, v) => MapEntry(k, v as Object)),
          );
        } else if ((item.state == 'undoing' || item.state == 'reversing') &&
            item.before != null) {
          fields = {
            for (final key in item.applied!.keys) key: item.before![key]!,
          };
        }
        if (fields != null) {
          intent.putIfAbsent(item.mail, () => {}).addAll(fields);
        }
      }
    }
    return rows
        .map((m) => intent.containsKey(m.id) ? m.patch(intent[m.id]!) : m)
        .toList();
  }

  Map<String, Object?> _summary(_Job job) {
    final groups = <String, Map<String, Object?>>{};
    for (final item in job.items) {
      final account =
          mail()
              .where((m) => m.id == item.mail)
              .map((m) => m.accountId.isEmpty ? m.account : m.accountId)
              .firstOrNull ??
          'preview';
      final group = groups.putIfAbsent(
        '$account:${item.folder}',
        () => {
          'account': account,
          'folder': item.folder,
          'total': 0,
          'unread': 0,
          'starred': 0,
        },
      );
      group['total'] = (group['total'] as int) + 1;
      if (item.unread) group['unread'] = (group['unread'] as int) + 1;
      if (item.starred) group['starred'] = (group['starred'] as int) + 1;
    }
    final counts = <String, int>{};
    for (final item in job.items) {
      counts[item.state] = (counts[item.state] ?? 0) + 1;
    }
    return {
      'id': job.id,
      'seq': job.seq,
      'action': job.action,
      'fields': job.fields,
      'state': job.state,
      'scope': const {},
      'created': job.seq,
      'total': job.items.length,
      'revision': job.revision,
      'error': job.error,
      'undo': job.undo,
      'counts': counts,
      'groups': groups.values.toList(),
    };
  }

  _Job _job(String id) {
    final job = jobs[id];
    if (job == null) {
      throw StateError('This group action is no longer in History.');
    }
    return job;
  }

  void _settle(_Job job) {
    const runnable = {'pending', 'sending', 'undoing', 'reversing'};
    if ({'running', 'undoing', 'paused'}.contains(job.state) &&
        !job.items.any((i) => runnable.contains(i.state))) {
      job.state = 'finished';
    }
    job.revision++;
  }

  static Map<String, Object?> fieldsOf(Map action) => switch (action['kind']) {
    'archive' => {'folder': 'Archive'},
    'delete' => {'folder': 'Trash'},
    'move' => {'folder': action['folder']},
    'read' => {'unread': false},
    'unread' => {'unread': true},
    'flag' => {'starred': true},
    'unflag' => {'starred': false},
    _ => throw StateError('Unknown action'),
  };

  @override
  Future<dynamic> groups(Map<String, Object?> command) async {
    final kind = command['kind'] as String;
    calls.add(kind);
    switch (kind) {
      case 'prepare':
        final finished = jobs.values
            .where((j) => j.state == 'finished')
            .toList();
        while (jobs.length >= 20 && finished.isNotEmpty) {
          jobs.remove(finished.removeAt(0).id);
        }
        if (jobs.length >= 20) {
          throw StateError(
            'History keeps 20 group actions. Finish or remove older ones first.',
          );
        }
        final action = Map<String, Object?>.from(command['action'] as Map);
        final job = _Job(
          ++_seq,
          command['id'] as String,
          action,
          fieldsOf(action),
        );
        jobs[job.id] = job;
        final frozen = await selection.selection({
          'kind': 'freeze',
          'id': command['selection'],
          'expected': command['expected'],
          'target': '${job.id}-frozen',
        });
        var after = -1;
        while (true) {
          final page = await selection.selection({
            'kind': 'page',
            'id': frozen['id'],
            'expected': 0,
            'after': after,
          });
          for (final row in page['rows'] as List) {
            final current = mail().where((m) => m.id == row['id']).firstOrNull;
            final item = _Item(
              row['position'] as int,
              row['id'] as String,
              current?.folder ?? row['folder'] as String,
              current?.unread ?? row['unread'] as bool,
              current?.starred ?? row['starred'] as bool,
            );
            if (current == null) {
              item.state = 'skipped';
              item.reason = 'Message is no longer cached';
            }
            job.items.add(item);
          }
          if (page['next_after'] == null) break;
          after = page['next_after'] as int;
        }
        await selection.selection({'kind': 'release', 'id': frozen['id']});
        await selection.selection({
          'kind': 'release',
          'id': command['selection'],
        });
        job.state = 'review';
        return _summary(job);
      case 'approve':
        final job = _job(command['id'] as String);
        if (job.state != 'review') {
          throw StateError(
            'This review is no longer open. Select the messages again.',
          );
        }
        job.state = 'running';
        job.approved = ++clock;
        _settle(job);
        return _summary(job);
      case 'decline':
        final job = jobs[command['id'] as String];
        if (job != null && job.state != 'review') {
          throw StateError(
            'This group action was approved. Use Undo in History instead.',
          );
        }
        jobs.remove(command['id']);
        return {'declined': true};
      case 'step':
        return step();
      case 'pause':
        final job = _job(command['id'] as String);
        if (job.state != 'running' && job.state != 'undoing') {
          throw StateError('This group action is not running.');
        }
        job.state = 'paused';
        job.revision++;
        return _summary(job);
      case 'resume':
        final job = _job(command['id'] as String);
        if (job.state != 'paused') {
          throw StateError('This group action is not paused.');
        }
        job.state = job.undo ? 'undoing' : 'running';
        _settle(job);
        return _summary(job);
      case 'undo':
        final job = _job(command['id'] as String);
        if (job.undo) {
          throw StateError('This group action is already being undone.');
        }
        if (!{'running', 'paused', 'finished'}.contains(job.state)) {
          throw StateError(
            'This group action cannot be undone from its current state.',
          );
        }
        job.undo = true;
        job.undone = ++clock;
        for (final item in job.items) {
          if (item.state == 'pending') {
            item.state = 'cancelled';
            item.reason = 'Cancelled before sending';
          } else if (item.state == 'done') {
            item.state = 'undoing';
            item.reason = null;
          }
        }
        job.state = 'undoing';
        _settle(job);
        return _summary(job);
      case 'retry':
        final job = _job(command['id'] as String);
        final item = job.items.firstWhere(
          (i) => i.position == command['position'],
        );
        if (item.state == 'uncertain' || item.state == 'undo_uncertain') {
          throw StateError(
            'The server result is unknown. Refresh and check the folder, then accept the current state; Shep never repeats an unconfirmed step.',
          );
        }
        if (item.state != 'failed' && item.state != 'undo_failed') {
          throw StateError('Only a failed step can be retried.');
        }
        item.state = job.undo ? 'undoing' : 'pending';
        item.reason = null;
        if (job.state == 'finished') {
          job.state = job.undo ? 'undoing' : 'running';
        }
        job.revision++;
        return _summary(job);
      case 'accept':
        final job = _job(command['id'] as String);
        final item = job.items.firstWhere(
          (i) => i.position == command['position'],
        );
        if (item.state != 'uncertain' && item.state != 'undo_uncertain') {
          throw StateError('Only an unconfirmed step can be accepted.');
        }
        item.state = 'accepted';
        item.reason = 'Current state accepted without a server confirmation';
        job.revision++;
        return _summary(job);
      case 'history':
        final listed = jobs.values.where((j) => j.state != 'cancelled').toList()
          ..sort((a, b) => b.seq.compareTo(a.seq));
        return {
          'jobs': listed.take(20).map(_summary).toList(),
          'runnable': jobs.values.any(
            (j) =>
                (j.state == 'running' && j.count('pending') > 0) ||
                (j.state == 'undoing' && j.count('undoing') > 0),
          ),
        };
      case 'items':
        final job = _job(command['id'] as String);
        final after = command['after'] as int? ?? -1;
        final rows = job.items.where((i) => i.position > after).take(50).map((
          i,
        ) {
          final current = mail().where((m) => m.id == i.mail).firstOrNull;
          return {
            'position': i.position,
            'mail': i.mail,
            'account': current?.account ?? '',
            'folder': i.folder,
            'unread': i.unread,
            'starred': i.starred,
            'state': i.state,
            'reason': i.reason,
            'subject': current?.subject,
            'sender': current?.sender,
          };
        }).toList();
        return {
          'rows': rows,
          'next_after': rows.length == 50 ? rows.last['position'] : null,
        };
      case 'remove':
        final job = _job(command['id'] as String);
        if (job.state != 'finished') {
          throw StateError(
            'Pause and finish or undo this group action before removing it from History.',
          );
        }
        jobs.remove(job.id);
        return {'removed': true};
    }
    throw StateError('Unknown group command $kind');
  }

  @override
  Future<Map<String, dynamic>> groupStep() async =>
      Map<String, dynamic>.from(await step() as Map);

  Future<Map<String, Object?>> step() async {
    if (stepping) {
      throw StateError('A group action step is already in progress.');
    }
    stepping = true;
    try {
      final ordered = jobs.values.toList()
        ..sort((a, b) => a.seq.compareTo(b.seq));
      for (final job in ordered) {
        final inverse = job.state == 'undoing';
        if (job.state != 'running' && !inverse) continue;
        final item = job.items
            .where((i) => i.state == (inverse ? 'undoing' : 'pending'))
            .firstOrNull;
        if (item == null) {
          _settle(job);
          continue;
        }
        stepStarted?.complete();
        stepStarted = null;
        return await _execute(job, item, inverse);
      }
      return {'idle': true};
    } finally {
      stepping = false;
    }
  }

  Future<Map<String, Object?>> _execute(
    _Job job,
    _Item item,
    bool inverse,
  ) async {
    Map<String, Object?> finish(String state, [String? reason]) {
      // Acknowledged after Undo was requested: reverse it with its receipt.
      if (state == 'done' && job.undo) state = 'undoing';
      item.state = state;
      item.reason = reason;
      if (state == 'uncertain' || state == 'undo_uncertain') {
        job.state = 'paused';
      }
      _settle(job);
      return {
        'stepped': true,
        'job': job.id,
        'position': item.position,
        'outcome': state,
        'reason': reason,
        'mail': item.mail,
      };
    }

    final current = mail().where((m) => m.id == item.mail).firstOrNull;
    if (current == null) {
      return finish('skipped', 'Message is no longer cached');
    }
    final expectedFolder = inverse
        ? (item.applied?['folder'] ?? item.folder)
        : item.folder;
    if (current.folder != expectedFolder) {
      return finish(
        inverse ? 'undo_skipped' : 'skipped',
        inverse
            ? 'Changed since the group ran; Undo skipped'
            : 'Changed since the review; skipped',
      );
    }
    var fields = inverse
        ? {
            for (final key in item.applied!.keys)
              key: item.before![key] as Object,
          }
        : job.fields.map((k, v) => MapEntry(k, v as Object));
    final approved = inverse ? job.undone : job.approved;
    final hadFields = fields.isNotEmpty;
    fields.removeWhere((key, _) => (intents[item.mail]?[key] ?? 0) > approved);
    if (hadFields && fields.isEmpty) {
      return finish(
        inverse ? 'undo_skipped' : 'skipped',
        'A newer change owns this message; skipped',
      );
    }
    fields.removeWhere((key, value) => current.field(key) == value);
    if (fields.isEmpty) {
      return finish(
        inverse ? 'undo_skipped' : 'skipped',
        inverse ? 'Already restored' : 'Already up to date',
      );
    }
    item.state = inverse ? 'reversing' : 'sending';
    job.revision++;
    if (hold != null) await hold!.future;
    final injected = outcomes.remove(item.mail);
    if (injected == 'failed') {
      return finish(
        inverse ? 'undo_failed' : 'failed',
        'Fixture rejected this step',
      );
    }
    if (injected == 'uncertain') {
      return finish(
        inverse ? 'undo_uncertain' : 'uncertain',
        'The server did not confirm this change. Shep will not repeat this move.',
      );
    }
    try {
      await mutate(item.mail, fields);
    } catch (e) {
      final ambiguous = fields.containsKey('folder');
      return finish(
        ambiguous
            ? (inverse ? 'undo_uncertain' : 'uncertain')
            : (inverse ? 'undo_failed' : 'failed'),
        ambiguous ? '$e Shep will not repeat this move.' : '$e',
      );
    }
    if (!inverse) {
      item.applied = fields;
      item.before = {for (final key in fields.keys) key: current.field(key)};
    }
    return finish(inverse ? 'undone' : 'done');
  }
}
