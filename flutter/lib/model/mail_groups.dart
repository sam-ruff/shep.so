import 'dart:async';
import '../data/groups.dart';
import 'mail.dart' show newDraftIdentity;
import 'mail_selection.dart';

/// Drives the durable group journal: one frozen review at a time, one owned
/// step at a time, bounded History pages and explicit failure decisions.
/// Membership stays in the repository; this controller holds counts only.
class MailGroups {
  MailGroups({
    required this.repository,
    required this.changed,
    required this.refreshMail,
  });
  final GroupRepository repository;
  final void Function() changed;
  final Future<void> Function() refreshMail;
  List<GroupJob> jobs = [];
  GroupJob? review;
  GroupJob? completed;
  bool preparing = false, running = false, historyLoading = false;
  bool _disposed = false, _pumpAgain = false, _historyAgain = false;
  Completer<void> _historyIdle = Completer<void>()..complete();
  String? error, historyError;
  Timer? _toast, _repaint;

  GroupJob? get active => jobs
      .where((j) => j.state == 'running' || j.state == 'undoing')
      .firstOrNull;
  Iterable<GroupJob> get needingReview => jobs.where((j) => j.attention > 0);
  bool get idle => !running && !preparing && review == null;

  Future<GroupJob?> prepare(
    MailSelection selection,
    GroupAction action, {
    String? folder,
  }) async {
    final snapshot = selection.snapshot;
    if (preparing || snapshot == null || !selection.ready) return null;
    preparing = true;
    error = null;
    changed();
    try {
      final data = await repository.groups({
        'kind': 'prepare',
        'id': newDraftIdentity(),
        'selection': snapshot.id,
        'expected': snapshot.revision,
        'action': action.toJson(folder: folder),
        'scope': selection.scope(),
      });
      if (_disposed) return null;
      review = GroupJob(data as Map<String, dynamic>);
      // The frozen review owns the captured membership from here on.
      selection.done();
      return review;
    } catch (e) {
      error = 'Could not prepare this action. $e';
      return null;
    } finally {
      preparing = false;
      changed();
    }
  }

  Future<bool> approve() async {
    final current = review;
    if (current == null) return false;
    try {
      final data = await repository.groups({
        'kind': 'approve',
        'id': current.id,
      });
      if (_disposed) return false;
      review = null;
      _replace(GroupJob(data as Map<String, dynamic>));
      changed();
      // Approved intent paints from the repository before any step runs.
      unawaited(refreshMail());
      unawaited(pump());
      return true;
    } catch (e) {
      error = '$e';
      review = null;
      changed();
      return false;
    }
  }

  Future<void> decline() async {
    final current = review;
    if (current == null) return;
    review = null;
    changed();
    try {
      await repository.groups({'kind': 'decline', 'id': current.id});
    } catch (_) {
      // A retired review is swept by the journal on its next prepare/open.
    }
  }

  /// Executes owned steps until the journal is idle or paused. A second call
  /// while running only asks the loop to continue.
  Future<void> pump() async {
    if (_disposed) return;
    if (running) {
      _pumpAgain = true;
      return;
    }
    running = true;
    error = null;
    changed();
    try {
      var steps = 0;
      do {
        _pumpAgain = false;
        while (!_disposed) {
          final reply = await repository.groupStep();
          if (_disposed) return;
          if (reply['idle'] == true) break;
          if (reply['requires_credentials'] != null) {
            throw StateError(
              'The account credential is unavailable. Reconnect it in Preferences and retry.',
            );
          }
          steps++;
          _requestRepaint();
          if (steps % 10 == 0) await refreshHistory();
        }
      } while (_pumpAgain && !_disposed);
    } catch (e) {
      error = 'Group action stopped. Retry to continue. $e';
    } finally {
      _repaint?.cancel();
      _repaint = null;
      if (!_disposed) await refreshHistory();
      // The loop is only idle once History reflects its last receipt.
      running = false;
      if (!_disposed) {
        unawaited(refreshMail());
        changed();
      }
    }
  }

  void _requestRepaint() {
    if (_repaint != null) return;
    // Repaint at most every 250 ms while steps land; the final refresh
    // happens when the loop stops.
    _repaint = Timer(const Duration(milliseconds: 250), () {
      _repaint = null;
      if (_disposed) return;
      unawaited(refreshMail());
      unawaited(refreshHistory());
    });
  }

  Future<void> refreshHistory() async {
    if (_disposed) return;
    if (historyLoading) {
      // Coalesce with the read in flight and observe again after it.
      _historyAgain = true;
      await _historyIdle.future;
      return;
    }
    _historyAgain = false;
    final idle = _historyIdle = Completer<void>();
    historyLoading = true;
    try {
      final data = await repository.groups({'kind': 'history'});
      if (_disposed) return;
      final previous = {for (final j in jobs) j.id: j};
      jobs = (data['jobs'] as List)
          .map((j) => GroupJob(j as Map<String, dynamic>))
          .toList();
      for (final job in jobs) {
        final before = previous[job.id];
        if (before != null && before.active && job.finished) {
          _announce(job);
        }
      }
      historyError = null;
      if (data['runnable'] == true && !running) unawaited(pump());
    } catch (e) {
      historyError = 'Could not read group History. $e';
    } finally {
      historyLoading = false;
      changed();
      if (_historyAgain && !_disposed) {
        await refreshHistory();
      }
      idle.complete();
    }
  }

  void _announce(GroupJob job) {
    completed = job;
    _toast?.cancel();
    _toast = Timer(const Duration(seconds: 6), dismiss);
  }

  void dismiss() {
    _toast?.cancel();
    completed = null;
    changed();
  }

  void _replace(GroupJob job) {
    jobs = [job, ...jobs.where((j) => j.id != job.id)];
  }

  Future<void> _command(Map<String, Object?> command) async {
    try {
      final data = await repository.groups(command);
      if (_disposed) return;
      if (data is Map<String, dynamic> && data['id'] is String) {
        _replace(GroupJob(data));
      }
      error = null;
    } catch (e) {
      error = '$e';
    }
    changed();
  }

  Future<void> undo(GroupJob job) async {
    if (completed?.id == job.id) dismiss();
    await _command({'kind': 'undo', 'id': job.id});
    // The decision paints the restored rows before its inverse steps run.
    unawaited(refreshMail());
    unawaited(pump());
  }

  Future<void> pause(GroupJob job) => _command({'kind': 'pause', 'id': job.id});
  Future<void> resume(GroupJob job) async {
    await _command({'kind': 'resume', 'id': job.id});
    unawaited(pump());
  }

  Future<void> retry(GroupJob job, GroupItem item) async {
    await _command({'kind': 'retry', 'id': job.id, 'position': item.position});
    unawaited(pump());
  }

  Future<void> accept(GroupJob job, GroupItem item) =>
      _command({'kind': 'accept', 'id': job.id, 'position': item.position});

  Future<void> remove(GroupJob job) async {
    try {
      await repository.groups({'kind': 'remove', 'id': job.id});
      jobs = jobs.where((j) => j.id != job.id).toList();
      error = null;
    } catch (e) {
      error = '$e';
    }
    changed();
  }

  Future<({List<GroupItem> rows, int? nextAfter})> items(
    GroupJob job, {
    int? after,
  }) async {
    final data = await repository.groups({
      'kind': 'items',
      'id': job.id,
      'after': after,
    });
    return (
      rows: (data['rows'] as List)
          .map((r) => GroupItem(r as Map<String, dynamic>))
          .toList(),
      nextAfter: data['next_after'] as int?,
    );
  }

  void dispose() {
    _disposed = true;
    _toast?.cancel();
    _repaint?.cancel();
  }
}
