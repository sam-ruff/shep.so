import 'dart:async';
import 'package:flutter/foundation.dart';
import '../data/repository.dart';
import '../data/settings_store.dart';
import '../data/accounts.dart';
import '../data/drafts.dart';
import '../data/outgoing.dart';
import '../data/printing.dart';
import 'mail.dart';
import 'move_feedback.dart';
import 'preferences.dart';

class Workspace extends ChangeNotifier {
  Workspace(
    this.repository,
    this.settings, {
    this.printer = const SystemMessagePrinter(),
  }) : _mail = List.of(repository.cached),
       _confirmed = {for (final mail in repository.cached) mail.id: mail},
       events = List.of(repository.events);
  final MessagePrinter printer;
  final MailRepository repository;
  final SettingsStore settings;
  List<Mail> _mail;
  final Map<String, Mail> _confirmed;
  final Map<String, int> _versions = {};
  final Map<String, Future<void>> _queues = {};
  final Map<String, Map<String, Object>> _projection = {};
  final Map<String, Draft> drafts = {};
  final Set<String> _removedAccounts = {};
  List<CalendarEntry> events;
  Preferences preferences = const Preferences();
  String folder = 'Inbox', query = '', filter = 'All';
  String? account;
  bool newestFirst = true, syncing = false, savingPreferences = false;
  bool _refreshAgain = false, _disposed = false;
  String? _error, notice;
  MoveRecord? _undoErrorOwner;
  String? get error => _error;
  set error(String? value) {
    _error = value;
    _undoErrorOwner = null;
  }

  late final moves = MoveFeedback(_changed);
  final Set<MoveRecord> undoFailures = {};
  VoidCallback? _flagUndo;
  int? _flagUndoRevision;
  List<MoveRecord>? _undoSnapshot;
  VoidCallback? _moveUndo;
  VoidCallback? get undo {
    if (!moves.visible) return _flagUndo;
    if (!moves.canUndo) return null;
    if (!identical(_undoSnapshot, moves.records)) {
      final snapshot = moves.records;
      _undoSnapshot = snapshot;
      _moveUndo = () => undoMoves(snapshot);
    }
    return _moveUndo;
  }

  String? get actionNotice =>
      moves.label ?? (_flagUndo != null ? 'Message updated' : null);
  Iterable<String> get _moveIds =>
      [...moves.records, ...undoFailures].map((r) => _canonical(r.id));
  void undoMoves(List<MoveRecord> expected) {
    for (final record in moves.restore(expected)) {
      unawaited(
        change(
          record.id,
          {'folder': record.originalFolder},
          offerUndo: false,
          quiet: true,
          restoring: record,
        ),
      );
    }
    _changed();
  }

  void dismissUndoFailures() {
    undoFailures.clear();
    _changed();
  }

  void retryUndos() {
    for (final record
        in undoFailures.where((r) => !r.restoreCommitted).toList()) {
      undoFailures.remove(record);
      moves.retryRestore(record);
      unawaited(
        change(
          record.id,
          {'folder': record.originalFolder},
          offerUndo: false,
          quiet: true,
          restoring: record,
          force: true,
        ),
      );
    }
    _changed();
  }

  Future<void> refreshRestored() async {
    final reviewing = undoFailures.where((r) => r.restoreCommitted).toList();
    await refresh();
    for (final record in reviewing) {
      final saved = repository.cached
          .where((m) => m.id == _canonical(record.id))
          .firstOrNull;
      if (saved?.folder == record.originalFolder) undoFailures.remove(record);
    }
    _changed();
  }

  String? _undoId;
  VoidCallback? retry;
  int _revision = 0, _settingsRevision = 0, limit = 50;
  Future<void> _settingsQueue = Future.value();
  Timer? _searchTimer;
  Timer? _syncTimer;
  int _pageRevision = 0, total = 0, _unread = 0;
  final Set<String> loadingBodies = {};
  final Map<String, Mail> _bodies = {};
  final Map<String, String> bodyErrors = {};
  bool _foreground = true;
  int get resultCount => accountRepository == null ? matching.length : total;
  List<String> get folders => {
    'Inbox',
    'Archive',
    'Sent',
    'Drafts',
    'Trash',
    'Spam',
    ...?accountRepository?.folderNames.values.expand(
      (names) => names.map((n) => n == 'INBOX' ? 'Inbox' : n),
    ),
  }.toList();
  void setForeground(bool active) {
    if (!active) unawaited(finishReading());
    _foreground = active;
  }

  AccountRepository? get accountRepository =>
      repository is AccountRepository ? repository as AccountRepository : null;
  final Set<String> selected = {};
  int get pending => _queues.length;
  List<String> get accounts => {
    ..._mail.map((m) => m.account),
    ...?accountRepository?.mailAccounts.map((a) => a.email),
  }.toList();
  int get unreadCount => accountRepository == null
      ? _mail.where((m) => m.folder == 'Inbox' && m.unread).length
      : _unread;
  List<Mail> get messages => List.unmodifiable(_mail);
  List<Mail> get matching {
    if (accountRepository != null) {
      return _mail
          .where(
            (m) =>
                (m.folder == folder ||
                    (folder == 'Sent' &&
                        _pageFolderScope == folder &&
                        (_pageFolders[m.accountId]?.contains(m.folder) ??
                            false))) &&
                (account == null || m.account == account) &&
                (filter != 'Unread' || m.unread) &&
                (filter != 'Flagged' || m.starred),
          )
          .toList();
    }
    final terms = query.toLowerCase().trim().split(RegExp(r'\s+'));
    final result = _mail
        .where(
          (m) =>
              m.folder == folder &&
              (account == null || m.account == account) &&
              (filter != 'Unread' || m.unread) &&
              (filter != 'Flagged' || m.starred) &&
              terms.every(
                (q) => '${m.sender} ${m.subject} ${m.body}'
                    .toLowerCase()
                    .contains(q),
              ),
        )
        .toList();
    result.sort(
      (a, b) =>
          newestFirst ? b.date.compareTo(a.date) : a.date.compareTo(b.date),
    );
    return result;
  }

  List<Mail> get visible => matching.take(limit).toList();
  String? _pageFolderScope;
  Map<String, Set<String>> _pageFolders = {};
  final Map<String, String> _aliases = {};
  String _canonical(String id) => _aliases[id] ?? id;
  Mail? _readCandidate;

  /// Cache retention is separate from deliberate reading: refresh and previews
  /// must never acknowledge mail that the user has not selected.
  void beginReading(String id) {
    id = _canonical(id);
    final current = mail(id);
    if (current == null) return;
    if (_readCandidate == null || _canonical(_readCandidate!.id) != id) {
      unawaited(finishReading());
      if (current.unread) _readCandidate = current;
    }
  }

  Future<void> finishReading({String? only}) async {
    final candidate = _readCandidate;
    if (candidate == null ||
        (only != null && _canonical(candidate.id) != _canonical(only))) {
      return;
    }
    _readCandidate = null;
    final id = _canonical(candidate.id);
    if (!_disposed && (mail(id) ?? _confirmed[id])?.unread == true) {
      await change(id, {'unread': false}, offerUndo: false, quiet: true);
    }
  }

  Mail? _reader;
  void retainReader(String id) {
    _reader = mail(id);
    if (_reader case final Mail current) {
      _confirmed.putIfAbsent(current.id, () => current);
    }
  }

  void releaseReader(String id) {
    if (_reader?.id == _canonical(id)) _reader = null;
  }

  Mail? mail(String id) =>
      _mail.where((m) => m.id == _canonical(id)).firstOrNull ??
      (_reader?.id == _canonical(id) ? _reader : null);
  bool loadingBody(String id) =>
      loadingBodies.contains(id) || loadingBodies.contains(_canonical(id));
  String? bodyError(String id) => bodyErrors[id] ?? bodyErrors[_canonical(id)];

  void _acceptAliases(Map<String, String> aliases) {
    _aliases.addAll(aliases);
    for (final entry in _aliases.entries.toList()) {
      _aliases[entry.key] = _canonical(entry.value);
    }
    if (_readCandidate case final Mail current) {
      _readCandidate = current.patch({'id': _canonical(current.id)});
    }
    if (_reader case final Mail current) {
      _reader = current.patch({'id': _canonical(current.id)});
    }
    for (final entry in aliases.entries) {
      final projection = _projection.remove(entry.key);
      if (projection != null) {
        final next = _projection.putIfAbsent(entry.value, () => {});
        for (final field in projection.keys) {
          if ((_versions['${entry.key}:$field'] ?? 0) >
              (_versions['${entry.value}:$field'] ?? 0)) {
            next[field] = projection[field]!;
          }
        }
      }
      final previous = _bodies.remove(entry.key);
      if (previous != null) _bodies.putIfAbsent(entry.value, () => previous);
      final error = bodyErrors.remove(entry.key);
      if (error != null) bodyErrors.putIfAbsent(entry.value, () => error);
      final confirmed = _confirmed.remove(entry.key);
      if (confirmed != null) {
        _confirmed.putIfAbsent(
          entry.value,
          () => confirmed.patch({'id': entry.value}),
        );
      }
      if (selected.remove(entry.key)) selected.add(entry.value);
      if (_undoId == entry.key) _undoId = entry.value;
      for (final field in ['folder', 'unread', 'starred']) {
        final revision = _versions.remove('${entry.key}:$field');
        final next = '${entry.value}:$field';
        if (revision != null && revision > (_versions[next] ?? 0)) {
          _versions[next] = revision;
        }
      }
    }
  }

  void _changed() {
    if (!_disposed) notifyListeners();
  }

  void clearError() {
    error = null;
    retry = null;
    _changed();
  }

  Future<void> initialize() async {
    final revision = _settingsRevision;
    try {
      final saved = await settings.read();
      if (revision == _settingsRevision) preferences = saved;
    } catch (_) {
      error =
          'Could not read preferences. Your saved settings have been kept; try reopening the app.';
    }
    final native = accountRepository;
    if (native != null) {
      try {
        await native.initialize();
        drafts.addEntries(native.savedDrafts.map((d) => MapEntry(d.id, d)));
        await loadPage();
      } catch (e) {
        error = '$e';
      }
      _syncTimer = Timer.periodic(const Duration(seconds: 15), (_) {
        if (_foreground && !syncing && native.mailAccounts.isNotEmpty) {
          unawaited(refresh());
        }
      });
    }
    _changed();
  }

  Future<void> loadPage({bool append = false}) async {
    final native = accountRepository;
    if (native == null || folder == 'Drafts') return;
    final pageRevision = ++_pageRevision, actionRevision = _revision;
    try {
      final page = await native.page(
        folder: folder,
        account: account,
        query: query,
        filter: filter,
        oldest: !newestFirst,
        offset: append ? matching.length : 0,
        projection: {
          for (final entry in _projection.entries)
            entry.key: Map.of(entry.value),
        },
      );
      if (pageRevision != _pageRevision) return;
      if (actionRevision != _revision) {
        unawaited(loadPage(append: append));
        return;
      }
      _acceptAliases(page.aliases);
      _pageFolderScope = folder;
      _pageFolders = append
          ? {
              ..._pageFolders,
              ...page.folderMembership.map(
                (id, names) => MapEntry(id, {...?_pageFolders[id], ...names}),
              ),
            }
          : page.folderMembership;
      final rows = page.mail
          .map(
            (m) => _bodies.containsKey(m.id) ? m.withDetail(_bodies[m.id]!) : m,
          )
          .toList();
      _mail = append ? [..._mail, ...rows] : rows;
      if (_reader case final Mail current) {
        _reader = _mail.where((m) => m.id == current.id).firstOrNull ?? current;
      }
      _confirmed.removeWhere(
        (id, _) =>
            id != _undoId &&
            id != _reader?.id &&
            id != _readCandidate?.id &&
            !_moveIds.contains(id) &&
            !_mail.any((m) => m.id == id) &&
            !_queues.keys.any((key) => _canonical(key) == id),
      );
      _aliases.removeWhere(
        (_, target) =>
            !_mail.any((m) => m.id == target) &&
            !_bodies.containsKey(target) &&
            target != _undoId &&
            target != _reader?.id &&
            target != _readCandidate?.id &&
            !_moveIds.contains(target) &&
            !_queues.keys.any((key) => _canonical(key) == target),
      );
      total = page.total;
      _unread = page.unread;
      for (final m in page.mail) {
        _confirmed[m.id] = page.confirmed[m.id] ?? m;
      }
      _confirmed.addAll(page.confirmed);
      _changed();
    } catch (e) {
      if (pageRevision == _pageRevision) {
        error = '$e';
        retry = () => unawaited(loadPage(append: append));
        _changed();
      }
    }
  }

  Future<void> loadBody(String id, {bool force = false}) async {
    id = _canonical(id);
    final requestedId = id;
    final native = accountRepository;
    if (native == null ||
        (!force && mail(id)?.bodyLoaded != false) ||
        !loadingBodies.add(id)) {
      return;
    }
    final revision = _revision;
    bodyErrors.remove(id);
    _changed();
    try {
      final detail = await native.detail(id);
      id = _canonical(id);
      // Metadata is always taken from current user intent, never from a body reply.
      if (mail(id) != null) {
        _bodies.remove(id);
        _bodies[id] = detail;
        if (_reader?.id == id) _reader = _reader!.withDetail(detail);
        while (_bodies.length > 8 ||
            _bodies.values.fold<int>(0, (sum, m) => sum + m.body.length * 2) >
                32 * 1024 * 1024) {
          _bodies.remove(_bodies.keys.first);
        }
        _mail = _mail
            .map(
              (m) => m.id == id
                  ? m.withDetail(detail)
                  : m.bodyLoaded && !_bodies.containsKey(m.id)
                  ? m.withoutBody()
                  : m,
            )
            .toList();
      }
    } catch (e) {
      id = _canonical(id);
      if (revision == _revision && mail(id) != null) bodyErrors[id] = '$e';
    } finally {
      loadingBodies.remove(requestedId);
      loadingBodies.remove(id);
      _changed();
    }
  }

  void _patchMail(String id, Map<String, Object> fields) {
    final beforeCount = matching.length;
    final current = mail(id) ?? _confirmed[id]?.patch(_projection[id] ?? {});
    if (current != null && accountRepository != null) {
      final changed = current.patch(fields);
      final before = current.folder == 'Inbox' && current.unread ? 1 : 0;
      final after = changed.folder == 'Inbox' && changed.unread ? 1 : 0;
      _unread = (_unread + after - before).clamp(0, 1 << 53);
    }
    if (_reader?.id == id) _reader = _reader!.patch(fields);
    if (_mail.any((m) => m.id == id)) {
      _mail = _mail.map((m) => m.id == id ? m.patch(fields) : m).toList();
    } else if (current != null &&
        fields.containsKey('folder') &&
        query.trim().isEmpty) {
      final changed = current.patch(fields);
      final inFolder =
          changed.folder == folder ||
          (folder == 'Sent' &&
              _pageFolderScope == folder &&
              (_pageFolders[changed.accountId]?.contains(changed.folder) ??
                  false));
      if (inFolder &&
          (account == null || changed.account == account) &&
          (filter != 'Unread' || changed.unread) &&
          (filter != 'Flagged' || changed.starred)) {
        _mail = [..._mail, changed.withoutBody()];
      }
    }
    if (accountRepository != null) {
      _mail.sort((a, b) {
        final order = newestFirst
            ? b.date.compareTo(a.date)
            : a.date.compareTo(b.date);
        return order == 0 ? a.id.compareTo(b.id) : order;
      });
      total = (total + matching.length - beforeCount).clamp(0, 1 << 53);
    }
  }

  Future<void> accountRemoved(String id) async {
    _removedAccounts.add(id);
    moves.removeAccount(id);
    undoFailures.removeWhere((r) => r.account == id);
    if (_readCandidate?.accountId == id) _readCandidate = null;
    drafts.removeWhere((_, draft) => draft.accountId == id);
    if (_confirmed[_undoId]?.accountId == id) {
      _flagUndo = null;
      _undoId = null;
    }
    _mail.removeWhere((m) => m.accountId == id);
    _bodies.removeWhere((_, m) => m.accountId == id);
    _projection.removeWhere((key, _) => _confirmed[key]?.accountId == id);
    _confirmed.removeWhere((_, m) => m.accountId == id);
    if (_reader?.accountId == id) _reader = null;
    account = null;
    folder = 'Inbox';
    selected.clear();
    notice = 'Account removed from this device.';
    error = null;
    _changed();
    await loadPage();
  }

  Future<void> savePreferences(Preferences value) async {
    preferences = value;
    final revision = ++_settingsRevision;
    savingPreferences = true;
    _changed();
    _settingsQueue = _settingsQueue.then((_) async {
      try {
        await settings.write(value);
        if (revision == _settingsRevision) {
          notice = 'Preferences saved';
          error = null;
        }
      } catch (_) {
        if (revision == _settingsRevision) {
          error =
              'Could not save preferences. Retry to keep these changes after restarting.';
          retry = () {
            unawaited(savePreferences(preferences));
          };
        }
      } finally {
        if (revision == _settingsRevision) savingPreferences = false;
        _changed();
      }
    });
    await _settingsQueue;
  }

  void search(String value) {
    unawaited(finishReading());
    _searchTimer?.cancel();
    _searchTimer = Timer(const Duration(milliseconds: 100), () {
      query = value;
      limit = 50;
      selected.clear();
      unawaited(loadPage());
      _changed();
    });
  }

  void navigate(String value, {String? inAccount}) {
    unawaited(finishReading());
    folder = value;
    account = inAccount;
    limit = 50;
    selected.clear();
    unawaited(loadPage());
    _changed();
  }

  void setFilter(String value) {
    unawaited(finishReading());
    filter = value;
    limit = 50;
    selected.clear();
    unawaited(loadPage());
    _changed();
  }

  void sort() {
    unawaited(finishReading());
    newestFirst = !newestFirst;
    limit = 50;
    unawaited(loadPage());
    _changed();
  }

  void more() {
    unawaited(finishReading());
    limit += 50;
    unawaited(loadPage(append: true));
    _changed();
  }

  void toggleSelection(String id) {
    id = _canonical(id);
    if (!selected.remove(id)) selected.add(id);
    _changed();
  }

  void selectAll() {
    selected.addAll(visible.map((m) => m.id));
    _changed();
  }

  Future<void> refresh() async {
    if (syncing) {
      _refreshAgain = true;
      notice = 'Refresh queued';
      _changed();
      return;
    }
    syncing = true;
    _changed();
    do {
      _refreshAgain = false;
      final revision = _revision;
      try {
        final result = await repository.refresh();
        if (accountRepository != null) {
          await loadPage();
          error = accountRepository!.warning;
          retry = error == null ? null : () => unawaited(refresh());
        }
        // A snapshot requested before any mutation cannot erase newer intent.
        if (accountRepository == null &&
            revision == _revision &&
            pending == 0) {
          _mail = List.of(result);
          _confirmed
            ..clear()
            ..addEntries(result.map((m) => MapEntry(m.id, m)));
        }
        notice = error != null
            ? null
            : repository.preview
            ? 'Preview refreshed'
            : 'Mail refreshed';
      } catch (e) {
        error = repository.preview
            ? 'Preview refresh failed. Retry when ready.'
            : '$e';
        retry = () {
          unawaited(refresh());
        };
      }
    } while (_refreshAgain && !_disposed);
    syncing = false;
    _changed();
  }

  Future<void> action(
    String id,
    MailAction action, {
    String? destination,
  }) async {
    final current = mail(id);
    if (current == null) return;
    if (action == MailAction.select) {
      toggleSelection(id);
      return;
    }
    final fields = switch (action) {
      MailAction.archive => <String, Object>{'folder': 'Archive'},
      MailAction.trash => <String, Object>{'folder': 'Trash'},
      MailAction.spam => <String, Object>{'folder': 'Spam'},
      MailAction.read => <String, Object>{'unread': !current.unread},
      MailAction.star => <String, Object>{'starred': !current.starred},
      MailAction.move when destination != null => <String, Object>{
        'folder': destination,
      },
      _ => <String, Object>{},
    };
    if (fields.isEmpty) return;
    await change(id, fields);
  }

  Future<void> change(
    String id,
    Map<String, Object> fields, {
    bool offerUndo = true,
    bool quiet = false,
    MoveRecord? restoring,
    bool force = false,
  }) async {
    id = _canonical(id);
    if (fields.containsKey('folder') &&
        _readCandidate != null &&
        _canonical(_readCandidate!.id) == id) {
      unawaited(finishReading(only: id));
    }
    if (!quiet &&
        fields.containsKey('unread') &&
        _readCandidate != null &&
        _canonical(_readCandidate!.id) == id) {
      _readCandidate = null;
    }
    final current =
        mail(id) ??
        (!offerUndo ? _confirmed[id]?.patch(_projection[id] ?? {}) : null);
    if (current == null || fields.isEmpty) return;
    if (!force &&
        fields.entries.every((e) => current.field(e.key) == e.value)) {
      return;
    }
    final move = fields.containsKey('folder') && offerUndo
        ? moves.add(
            id,
            current.accountId.isEmpty ? current.account : current.accountId,
            current.folder,
            fields['folder'] as String,
          )
        : null;
    final previous = {for (final key in fields.keys) key: current.field(key)};
    final revision = ++_revision;
    for (final key in fields.keys) {
      _versions['$id:$key'] = revision;
    }
    _patchMail(id, fields);
    _projection.putIfAbsent(id, () => {}).addAll(fields);
    selected.remove(id);
    if (move != null) {
      _flagUndo = null;
      _undoId = null;
      notice = null;
    } else if (offerUndo) {
      _flagUndoRevision = revision;
      _undoId = id;
      _flagUndo = () {
        if (_flagUndoRevision != revision) return;
        _flagUndo = null;
        unawaited(change(id, previous, offerUndo: false));
        _undoId = null;
      };
    }
    if (!quiet) {
      if (move == null) notice = 'Message updated';
      error = null;
      retry = null;
    }
    final before = Future.wait(
      _queues.entries
          .where((entry) => _canonical(entry.key) == id)
          .map((entry) => entry.value),
    );
    final job = before.then((_) async {
      var target = _canonical(id);
      if (!_confirmed.containsKey(target)) return;
      if (move?.cancelled == true ||
          (restoring != null && !restoring.committed)) {
        return;
      }
      if (move != null) move.started = true;
      try {
        await repository.mutate(target, fields);
        if (move != null) move.committed = true;
        if (restoring != null) {
          undoFailures.remove(restoring);
          if (identical(_undoErrorOwner, restoring)) {
            error = null;
            retry = null;
          }
        }
        target = _canonical(id);
        if (!_confirmed.containsKey(target)) return;
        _confirmed[target] = _confirmed[target]!.patch(fields);
      } catch (e) {
        target = _canonical(id);
        if (!_confirmed.containsKey(target)) return;
        if (move != null && !move.undoRequested) moves.failed(move);
        if (restoring != null) {
          moves.failed(restoring);
          undoFailures.add(restoring);
        }
        if (e is MailOperationFailure && e.committed) {
          if (restoring != null) restoring.restoreCommitted = true;
          _confirmed[target] = _confirmed[target]!.patch(fields);
          if (move != null) {
            move.committed = true;
            move.blocked = true;
          }
          error = e.message;
          if (restoring != null) _undoErrorOwner = restoring;
          if (!quiet && move == null && _flagUndoRevision == revision) {
            notice = null;
            _flagUndo = null;
            _undoId = null;
          }
          retry = () => unawaited(refresh());
          return;
        }
        final rollback = <String, Object>{};
        for (final key in fields.keys) {
          if (_versions['$target:$key'] == revision) {
            rollback[key] = _confirmed[target]!.field(key);
          }
        }
        _patchMail(target, rollback);
        if (rollback.isNotEmpty || move != null || restoring != null) {
          error =
              'Could not update ${current.subject}. The affected display was restored. ${e is MailOperationFailure ? e.message : 'Retry.'}';
          if (restoring != null) _undoErrorOwner = restoring;
          if (!quiet && move == null && _flagUndoRevision == revision) {
            notice = null;
            _flagUndo = null;
            _undoId = null;
          }
          retry = () {
            unawaited(
              e is MailOperationFailure
                  ? refresh()
                  : change(
                      id,
                      fields,
                      offerUndo: !quiet,
                      quiet: quiet,
                      restoring: restoring,
                      force: restoring != null,
                    ),
            );
          };
        }
      }
    });
    _queues[id] = job;
    if (restoring != null && accountRepository != null) unawaited(loadPage());
    _changed();
    await job;
    final target = _canonical(id);
    final projection = _projection[target];
    for (final field in fields.keys) {
      if (_versions['$target:$field'] == revision) projection?.remove(field);
    }
    if (projection?.isEmpty == true) _projection.remove(target);
    ++_revision; // A page captured before this acknowledgment must be retried.
    if (identical(_queues[id], job)) _queues.remove(id);
    if (accountRepository != null) {
      unawaited(loadPage());
    }
    _changed();
  }

  final _printing = <String>{};
  bool isPrinting(String id) => _printing.contains(id);
  Future<void> printMessage(String id, {required bool plain}) async {
    if (!_printing.add(id)) return;
    final startingError = error;
    _changed();
    try {
      final source = repository;
      if (source is! PrintRepository) {
        throw const MailOperationFailure(
          'Printing is unavailable in this preview. Use a connected native client.',
        );
      }
      final generation = newDraftIdentity();
      final prepared = await (source as PrintRepository).preparePrint(
        id,
        generation: generation,
        plain: plain,
      );
      if (_disposed) return;
      if (_removedAccounts.contains(prepared.accountId)) {
        throw const MailOperationFailure(
          'This account was removed. Open a connected message before printing.',
        );
      }
      if (prepared.issues.isNotEmpty) error = prepared.issues.join(' ');
      await printer.open(prepared, generation: generation);
      if (!_disposed && error == startingError) error = null;
    } catch (e) {
      if (!_disposed) error = '$e';
    } finally {
      _printing.remove(id);
      _changed();
    }
  }

  final _forwardRequests = <String, String>{};
  final _forwarding = <String>{};
  bool isForwarding(String id) => _forwarding.contains(id);
  Future<Draft?> forward(String id) async {
    if (!_forwarding.add(id)) return null;
    final startingError = error;
    _changed();
    try {
      final repository = this.repository;
      final Draft draft;
      if (repository is ForwardRepository) {
        final target = _forwardRequests.putIfAbsent(id, newDraftIdentity);
        draft = await (repository as ForwardRepository).forward(id, target);
      } else if (repository.preview) {
        final original = mail(id);
        if (original == null) return null;
        draft = Draft(
          id: newDraftIdentity(),
          accountId: original.accountId,
          subject: 'Fwd: ${original.subject}',
          body:
              '\n\n---------- Forwarded message ----------\nFrom: ${original.address}\nSubject: ${original.subject}\n\n${original.body}',
        );
        await repository.saveDraft(draft);
      } else {
        throw const MailOperationFailure(
          'Forwarding is unavailable. Reopen Shep and retry.',
        );
      }
      _forwardRequests.remove(id);
      if (_removedAccounts.contains(draft.accountId)) {
        throw const MailOperationFailure(
          'This account was removed. Open Drafts or choose a connected account.',
        );
      }
      drafts[draft.id] = draft;
      notice = 'Forward saved in Drafts';
      if (error == startingError) error = null;
      return draft;
    } catch (e) {
      error = '$e';
      return null;
    } finally {
      _forwarding.remove(id);
      _changed();
    }
  }

  Future<Draft?> reply(String id, bool all) async {
    unawaited(finishReading());
    try {
      final original = mail(id);
      if (original == null) return null;
      if (repository case final DraftRepository drafts) {
        return await drafts.reply(id, all);
      }
      return Draft(
        id: 'reply-${DateTime.now().microsecondsSinceEpoch}',
        accountId: original.accountId,
        to: original.address,
        subject: original.subject.toLowerCase().startsWith('re:')
            ? original.subject
            : 'Re: ${original.subject}',
        body: '\n\n> ${original.body.replaceAll('\n', '\n> ')}',
      );
    } catch (e) {
      error = '$e';
      _changed();
      return null;
    }
  }

  Future<bool> saveDraft(Draft draft) async {
    try {
      await repository.saveDraft(draft);
      if (_removedAccounts.contains(draft.accountId)) {
        error =
            'This account was removed. Copy this text into a new draft with a connected account.';
        _changed();
        return false;
      }
      drafts[draft.id] = draft;
      notice = 'Draft saved';
      _changed();
      return true;
    } catch (e) {
      error = e is MailOperationFailure
          ? e.message
          : 'Draft could not be saved. Keep this editor open and retry.';
      _changed();
      return false;
    }
  }

  Future<bool> discardDraft(Draft draft) async {
    final native = accountRepository;
    if (native == null) return false;
    try {
      await native.discard(draft.id, draft.revision);
      drafts.remove(draft.id);
      notice = 'Draft discarded';
      _changed();
      return true;
    } catch (e) {
      error = '$e';
      _changed();
      return false;
    }
  }

  Future<OutgoingResult> recoverOutgoing(
    OutgoingEntry entry,
    OutgoingAction action, {
    bool confirmed = false,
  }) async {
    final outbox = repository as OutgoingRepository;
    final result = await outbox.recoverOutgoing(
      entry.id,
      action,
      confirmed: confirmed,
    );
    if (result.recovery != null ||
        result.state == 'delivered' ||
        result.sent == 'saved' ||
        result.sent == 'local') {
      drafts.remove(entry.draftId);
    }
    if (result.draftId case final String id) {
      final recovered = accountRepository?.savedDrafts
          .where((d) => d.id == id)
          .firstOrNull;
      if (recovered != null &&
          (drafts[id]?.revision ?? -1) <= recovered.revision) {
        drafts[id] = recovered;
      }
    }
    notice =
        result.notice ??
        switch (result.recovery) {
          'returned' =>
            'Returned to drafts. Sending requires a new Send action.',
          'marked' => 'Recorded as sent after your review.',
          'local' => 'Sent copy kept locally.',
          _ => 'Delivery status checked.',
        };
    _changed();
    if (accountRepository != null && folder != 'Drafts') await loadPage();
    return result;
  }

  Future<bool> send(Draft draft) async {
    try {
      await repository.send(draft);
      drafts.remove(draft.id);
      _changed();
      return true;
    } catch (e) {
      error = repository.preview
          ? 'Preview cannot send mail. Your draft is still open.'
          : '$e';
      _changed();
      return false;
    }
  }

  Future<bool> saveEvent(CalendarEntry entry) async {
    try {
      await repository.saveEvent(entry);
      events = [...events.where((e) => e.id != entry.id), entry];
      notice = 'Event saved';
      _changed();
      return true;
    } catch (_) {
      error = 'Event could not be saved. Keep the form open and retry.';
      _changed();
      return false;
    }
  }

  @override
  void dispose() {
    _disposed = true;
    moves.dispose();
    _searchTimer?.cancel();
    _syncTimer?.cancel();
    super.dispose();
  }
}
