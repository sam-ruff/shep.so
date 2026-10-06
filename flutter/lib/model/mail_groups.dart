import 'dart:async';
import '../data/groups.dart';
import 'mail.dart' show newDraftIdentity;
import 'mail_selection.dart';

class _Review {
  _Review(this.job, this.selection, this.capture);
  final GroupJob job;
  final MailSelection selection;
  final SelectionCapture capture;
}

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
  List<GroupJob> activeJobs = [];
  List<GroupJob> observedJobs = [];
  int attentionCount = 0;
  GroupJob? attentionTarget, updatedJob;
  String? removedJob;
  int updateRevision = 0;
  _Review? _review;
  GroupJob? get review => _review?.job;
  bool deciding = false;
  bool _approvalUnknown = false;
  String? _cleanup;
  GroupJob? completed;
  bool preparing = false, running = false, historyLoading = false;
  bool _disposed = false, _pumpAgain = false, _historyAgain = false;
  Completer<void> _historyIdle = Completer<void>()..complete();
  String? error, historyError;
  Timer? _toast, _repaint;

  GroupJob? get active => activeJobs
      .where((j) => j.state == 'running' || j.state == 'undoing')
      .firstOrNull;
  Iterable<GroupJob> get needingReview => jobs.where((j) => j.attention > 0);
  bool get idle => !running && !preparing && review == null;

  Future<GroupJob?> prepare(
    MailSelection selection,
    GroupAction action, {
    String? folder,
  }) async {
    final capture = selection.capture;
    if (_disposed ||
        preparing ||
        deciding ||
        review != null ||
        capture == null) {
      return null;
    }
    final snapshot = capture.snapshot;
    final id = newDraftIdentity();
    preparing = true;
    error = null;
    changed();
    try {
      if (_cleanup case final old?) {
        if (!await _retire(old)) return null;
      }
      if (!selection.owns(capture) || _disposed) return null;
      GroupJob job;
      try {
        job = GroupJob(
          await repository.groups({
                'kind': 'prepare',
                'id': id,
                'selection': snapshot.id,
                'expected': snapshot.revision,
                'action': action.toJson(folder: folder),
                'scope': capture.scope,
              })
              as Map<String, dynamic>,
        );
      } catch (_) {
        // Recover the reserved identity rather than creating another review.
        job = await _inspect(id);
        if (job.state != 'review') rethrow;
      }
      if (_disposed || !selection.owns(capture)) {
        await _retire(id);
        return null;
      }
      _review = _Review(job, selection, capture);
      _approvalUnknown = false;
      return review;
    } catch (e) {
      await _retire(id);
      if (!_disposed && selection.owns(capture)) {
        error = 'Could not prepare this action. $e';
      }
      return null;
    } finally {
      preparing = false;
      if (!_disposed) changed();
    }
  }

  Future<GroupJob> _inspect(String id) async => GroupJob(
    await repository.groups({'kind': 'inspect', 'id': id})
        as Map<String, dynamic>,
  );

  Future<bool> _retire(String id) async {
    try {
      await repository.groups({'kind': 'decline', 'id': id});
      if (_cleanup == id) _cleanup = null;
      return true;
    } catch (e) {
      _cleanup = id;
      if (!_disposed) {
        error = 'Could not close the saved review. Retry this action. $e';
      }
      return false;
    }
  }

  Future<bool> approve({String? expected}) async {
    final current = _review;
    if (_disposed ||
        deciding ||
        current == null ||
        (expected != null && expected != current.job.id)) {
      return false;
    }
    if (!current.selection.owns(current.capture)) {
      await decline(expected: current.job.id);
      if (_disposed || (_review != null && _review != current)) return false;
      error = 'The selection changed. Review the selected messages again.';
      changed();
      return false;
    }
    deciding = true;
    error = null;
    changed();
    try {
      GroupJob job;
      final saved = _approvalUnknown ? await _inspect(current.job.id) : null;
      if (saved != null && !saved.inReview) {
        job = saved;
      } else {
        _approvalUnknown = true;
        try {
          job = GroupJob(
            await repository.groups({'kind': 'approve', 'id': current.job.id})
                as Map<String, dynamic>,
          );
        } catch (_) {
          job = await _inspect(current.job.id);
          _approvalUnknown = false;
          if (job.inReview) rethrow;
        }
      }
      if (!job.active && !job.finished) {
        throw StateError(
          'This saved review is no longer open. Cancel and review again.',
        );
      }
      _approved(current, job);
      return true;
    } catch (e) {
      if (!_disposed && _review == current) {
        error =
            'Could not confirm this action. Retry checks its saved status. $e';
      }
      return false;
    } finally {
      deciding = false;
      if (!_disposed) changed();
    }
  }

  void _approved(_Review current, GroupJob job) {
    _approvalUnknown = false;
    if (_cleanup == current.job.id) _cleanup = null;
    current.selection.complete(current.capture);
    if (_disposed || _review != current) return;
    _review = null;
    _replace(job);
    unawaited(refreshMail());
    unawaited(pump());
  }

  Future<bool> decline({String? expected}) async {
    final current = _review;
    if (deciding ||
        current == null ||
        (expected != null && expected != current.job.id)) {
      return false;
    }
    deciding = true;
    changed();
    try {
      if (_approvalUnknown) {
        final job = await _inspect(current.job.id);
        if (job.active || job.finished) {
          _approved(current, job);
          return true;
        }
        _approvalUnknown = false;
      }
      if (!await _retire(current.job.id)) return false;
      if (_review == current) _review = null;
      error = null;
      return true;
    } catch (e) {
      if (!_disposed) error = 'Could not check the saved action. Retry. $e';
      return false;
    } finally {
      deciding = false;
      if (!_disposed) changed();
    }
  }

  Future<void> retryPending() async {
    if (_disposed || deciding) return;
    final cleanup = _cleanup;
    if (cleanup == null) {
      if (review == null) await pump();
      return;
    }
    deciding = true;
    changed();
    try {
      if (await _retire(cleanup)) error = null;
    } finally {
      deciding = false;
      if (!_disposed) changed();
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

  Future<void> refreshHistory() {
    if (_disposed) return Future.value();
    _historyAgain = true;
    if (historyLoading) return _historyIdle.future;
    _historyIdle = Completer<void>();
    historyLoading = true;
    unawaited(_observeHistory());
    return _historyIdle.future;
  }

  Future<void> _observeHistory() async {
    final idle = _historyIdle;
    try {
      while (_historyAgain && !_disposed) {
        _historyAgain = false;
        final revision = updateRevision;
        try {
          final data = await repository.groups({
            'kind': 'history',
            'tracked': activeJobs.map((j) => j.id).toList(),
          });
          if (_disposed) return;
          if (revision != updateRevision) {
            _historyAgain = true;
            continue;
          }
          final previous = {
            for (final j in [...jobs, ...activeJobs]) j.id: j,
          };
          jobs = (data['jobs'] as List)
              .map((j) => GroupJob(j as Map<String, dynamic>))
              .toList();
          activeJobs =
              (data['active'] as List? ?? jobs.where((j) => j.active).toList())
                  .map(
                    (j) =>
                        j is GroupJob ? j : GroupJob(j as Map<String, dynamic>),
                  )
                  .toList();
          attentionCount =
              data['attention'] as int? ??
              needingReview.fold(0, (n, j) => n + j.attention);
          attentionTarget = data['attention_job'] is Map<String, dynamic>
              ? GroupJob(data['attention_job'] as Map<String, dynamic>)
              : null;
          final tracked = (data['tracked'] as List? ?? [])
              .map((j) => GroupJob(j as Map<String, dynamic>))
              .toList();
          observedJobs = [
            ...tracked,
            ...observedJobs.where((j) => !tracked.any((t) => t.id == j.id)),
          ].take(20).toList();
          for (final job in [
            ...jobs,
            ...(data['tracked'] as List? ?? []).map(
              (j) => GroupJob(j as Map<String, dynamic>),
            ),
          ]) {
            final before = previous[job.id];
            if (before != null && before.active && job.finished) {
              _announce(job);
            }
          }
          historyError = null;
          if (data['runnable'] == true && !running) unawaited(pump());
        } catch (e) {
          if (_disposed) return;
          if (revision != updateRevision) {
            _historyAgain = true;
            continue;
          }
          historyError = 'Could not read group History. $e';
        }
        if (!_disposed) changed();
      }
    } finally {
      historyLoading = false;
      if (!_disposed) changed();
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
    final known = [
      ...jobs,
      ...activeJobs,
      ...observedJobs,
      ?updatedJob,
    ].where((j) => j.id == job.id);
    if (known.any((j) => j.revision > job.revision)) return;
    jobs = [job, ...jobs.where((j) => j.id != job.id)]
      ..sort((a, b) => b.sequence.compareTo(a.sequence));
    jobs = jobs.take(20).toList();
    activeJobs = [
      if (job.active) job,
      ...activeJobs.where((j) => j.id != job.id),
    ];
    updatedJob = job;
    removedJob = null;
    updateRevision++;
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
      if (_disposed) return;
      error = '$e';
    }
    if (!_disposed) changed();
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

  Future<bool> remove(GroupJob job) async {
    try {
      await repository.groups({'kind': 'remove', 'id': job.id});
      if (_disposed) return false;
      jobs = jobs.where((j) => j.id != job.id).toList();
      activeJobs = activeJobs.where((j) => j.id != job.id).toList();
      removedJob = job.id;
      updatedJob = null;
      updateRevision++;
      error = null;
      changed();
      return true;
    } catch (e) {
      if (_disposed) return false;
      error = '$e';
    }
    if (!_disposed) changed();
    return false;
  }

  Future<
    ({
      List<GroupItem> rows,
      int? nextAfter,
      bool hasPrevious,
      int? previousAfter,
    })
  >
  items(GroupJob job, {int? after}) async {
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
      hasPrevious: data['has_previous'] == true,
      previousAfter: data['previous_after'] as int?,
    );
  }

  void dispose() {
    _disposed = true;
    _toast?.cancel();
    _repaint?.cancel();
    final id = !deciding && !_approvalUnknown ? review?.id ?? _cleanup : null;
    if (id != null) unawaited(_retire(id));
  }
}
