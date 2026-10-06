import 'dart:async';
import '../data/groups.dart';
import 'mail_groups.dart';

class GroupHistory {
  GroupHistory({required this.groups, required this.changed});
  final MailGroups groups;
  final void Function() changed;
  List<GroupJob> jobs = [];
  List<GroupItem> rows = [];
  String? selected;
  int? before,
      retryBefore,
      nextBefore,
      previousBefore,
      after,
      nextAfter,
      previousAfter;
  bool hasNewer = false, hasPrevious = false;
  bool loading = false, itemsLoading = false, _disposed = false;
  String? error, itemsError;
  int _pageGeneration = 0, _detailGeneration = 0, _observedUpdate = -1;
  ({int? cursor, int generation})? _requestedPage;
  bool _pageRunning = false;
  Completer<void> _pageIdle = Completer<void>()..complete();
  ({GroupJob job, int? cursor, int generation})? _requestedItems;
  bool _itemsRunning = false;
  Completer<void> _itemsIdle = Completer<void>()..complete();
  final _decisions = <(String, int)>{};
  GroupJob? get selectedJob => jobs.where((j) => j.id == selected).firstOrNull;

  Future<void> openTarget(GroupJob target) async {
    final generation = _pageGeneration + 1;
    await load(cursor: target.sequence > 0 ? target.sequence + 1 : null);
    if (_disposed || generation != _pageGeneration) return;
    final job = jobs.where((j) => j.id == target.id).firstOrNull;
    if (job == null) {
      error = 'This saved group is no longer available. Refresh History.';
      changed();
      return;
    }
    toggle(job);
  }

  void _clearDetails() {
    _detailGeneration++;
    _requestedItems = null;
    rows = [];
    after = nextAfter = previousAfter = null;
    itemsError = null;
    itemsLoading = false;
    hasPrevious = false;
  }

  Future<void> load({int? cursor, bool preserve = false}) {
    if (_disposed) return Future.value();
    final generation = ++_pageGeneration;
    if (!preserve || cursor != before) {
      selected = null;
      _clearDetails();
    }
    retryBefore = cursor;
    error = null;
    loading = true;
    changed();
    _requestedPage = (cursor: cursor, generation: generation);
    if (_pageRunning) return _pageIdle.future;
    _pageRunning = true;
    _pageIdle = Completer<void>();
    unawaited(_readPages());
    return _pageIdle.future;
  }

  Future<void> _readPages() async {
    final idle = _pageIdle;
    try {
      while (!_disposed && _requestedPage != null) {
        final request = _requestedPage!;
        _requestedPage = null;
        final revision = groups.updateRevision;
        try {
          final data = await groups.repository.groups({
            'kind': 'history',
            'before': request.cursor,
          });
          if (_disposed || request.generation != _pageGeneration) continue;
          if (revision != groups.updateRevision) {
            _requestedPage ??= request;
            continue;
          }
          final raw = data['jobs'] as List;
          if (raw.length > 20) {
            throw StateError('History returned more than one page.');
          }
          jobs = raw.map((j) => GroupJob(j as Map<String, dynamic>)).toList();
          before = request.cursor;
          nextBefore = data['next_before'] as int?;
          previousBefore = data['previous_before'] as int?;
          hasNewer = data['has_previous'] == true;
          if (selected != null && selectedJob == null) {
            selected = null;
            _clearDetails();
          }
        } catch (e) {
          if (!_disposed && request.generation == _pageGeneration) {
            if (revision != groups.updateRevision) {
              _requestedPage ??= request;
              continue;
            }
            error =
                'Could not read History. Retry the requested page or choose another page. $e';
          }
        }
      }
    } finally {
      _pageRunning = false;
      if (!_disposed) {
        loading = false;
        changed();
      }
      idle.complete();
    }
  }

  Future<void> remove(GroupJob job) async {
    if (_disposed || !jobs.any((j) => j.id == job.id)) return;
    if (!await groups.remove(job) || _disposed) return;
    jobs = jobs.where((j) => j.id != job.id).toList();
    if (selected == job.id) {
      selected = null;
      _clearDetails();
    }
    changed();
  }

  void observeUpdates() {
    final removed = _observedUpdate != groups.updateRevision
        ? groups.removedJob
        : null;
    _observedUpdate = groups.updateRevision;
    if (removed case final id?) {
      jobs = jobs.where((j) => j.id != id).toList();
      if (selected == id) {
        selected = null;
        _clearDetails();
      }
    }
    final observed = [
      ...groups.jobs,
      ...groups.activeJobs,
      ...groups.observedJobs,
      ?groups.updatedJob,
    ];
    jobs = jobs.map((job) {
      var current = job;
      for (final update in observed) {
        if (update.id == job.id && update.revision >= current.revision) {
          current = update;
        }
      }
      return current;
    }).toList();
  }

  void toggle(GroupJob job) {
    if (_disposed) return;
    if (loading) {
      _pageGeneration++;
      _requestedPage = null;
      loading = false;
      retryBefore = before;
    }
    final closing = selected == job.id;
    _clearDetails();
    selected = closing ? null : job.id;
    changed();
    if (!closing) unawaited(loadItems(job));
  }

  Future<void> loadItems(GroupJob job, {int? cursor}) {
    if (_disposed || selected != job.id) return Future.value();
    final generation = ++_detailGeneration;
    after = cursor;
    rows = [];
    nextAfter = previousAfter = null;
    hasPrevious = false;
    itemsError = null;
    itemsLoading = true;
    changed();
    _requestedItems = (job: job, cursor: cursor, generation: generation);
    if (_itemsRunning) return _itemsIdle.future;
    _itemsRunning = true;
    _itemsIdle = Completer<void>();
    unawaited(_readItems());
    return _itemsIdle.future;
  }

  Future<void> _readItems() async {
    final idle = _itemsIdle;
    try {
      while (!_disposed && _requestedItems != null) {
        final request = _requestedItems!;
        _requestedItems = null;
        bool owns() =>
            !_disposed &&
            selected == request.job.id &&
            request.generation == _detailGeneration;
        try {
          final page = await groups.items(request.job, after: request.cursor);
          if (!owns()) continue;
          if (page.rows.length > 50) {
            throw StateError('Group details returned more than one page.');
          }
          rows = page.rows;
          nextAfter = page.nextAfter;
          previousAfter = page.previousAfter;
          hasPrevious = page.hasPrevious;
        } catch (e) {
          if (owns()) {
            itemsError = 'Could not read these messages. Retry this page. $e';
          }
        } finally {
          if (owns()) {
            itemsLoading = false;
            changed();
          }
        }
      }
    } finally {
      _itemsRunning = false;
      idle.complete();
    }
  }

  int get detailGeneration => _detailGeneration;
  int get pageGeneration => _pageGeneration;
  bool pending(GroupJob job, GroupItem item) =>
      _decisions.contains((job.id, item.position));
  Future<void> decide(
    GroupJob job,
    GroupItem item,
    int generation, {
    required bool accept,
  }) async {
    if (_disposed ||
        itemsLoading ||
        selected != job.id ||
        generation != _detailGeneration ||
        !rows.contains(item)) {
      return;
    }
    final key = (job.id, item.position);
    if (_decisions.contains(key)) return;
    if (_decisions.length >= 32) {
      itemsError = 'Group recovery is catching up. Retry after it finishes.';
      changed();
      return;
    }
    _decisions.add(key);
    final cursor = after;
    changed();
    try {
      if (accept) {
        await groups.accept(job, item);
      } else {
        await groups.retry(job, item);
      }
    } finally {
      _decisions.remove(key);
      if (!_disposed) {
        observeUpdates();
        changed();
        if (selected == job.id && generation == _detailGeneration) {
          unawaited(loadItems(selectedJob ?? job, cursor: cursor));
        }
      }
    }
  }

  void dispose() {
    _disposed = true;
    _pageGeneration++;
    _requestedPage = null;
    _clearDetails();
    jobs = [];
  }
}
