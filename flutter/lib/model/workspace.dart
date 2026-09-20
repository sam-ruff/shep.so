import 'dart:async';
import 'dart:convert';
import 'dart:math';
import 'package:flutter/foundation.dart';
import '../data/repository.dart';
import '../data/settings_store.dart';
import '../data/accounts.dart';
import '../data/drafts.dart';
import '../data/outgoing.dart';
import '../data/printing.dart';
import 'mail.dart';
import 'mail_groups.dart';
import 'mail_selection.dart';
import 'move_feedback.dart';
import 'preferences.dart';
import '../data/groups.dart';
import '../data/selection.dart';
import 'google_connection.dart';
import 'profile_discovery.dart';
import '../data/profile_settings.dart';
import '../data/profile_enrollment.dart';
import '../data/profile_discovery.dart';
part 'profile_application.dart';

class _CalendarAdmissionAttempt {
  _CalendarAdmissionAttempt({
    required this.id,
    required this.subject,
    required this.mutation,
  });
  final String id, subject;
  final Map<String, Object?> mutation;
  bool closed = false;
}

class Workspace extends ChangeNotifier {
  Workspace(
    this.repository,
    this.settings, {
    this.printer = const SystemMessagePrinter(),
    this.google,
    this.profileDiscovery,
  }) : _mail = List.of(repository.cached),
       _confirmed = {for (final mail in repository.cached) mail.id: mail},
       events = List.of(repository.events);
  final MessagePrinter printer;
  final GoogleConnection? google;
  final ProfileDiscovery? profileDiscovery;
  final MailRepository repository;
  List<MailActivity> mailActivities = const [];
  List<CalendarActivity> calendarActivities = const [];
  List<CalendarSource> calendarSources = const [];
  final Set<String> _runningCalendarActions = {};
  final Map<String, _CalendarAdmissionAttempt> _calendarAdmissions = {};
  int _calendarViewRevision = 0;
  String? _calendarSubject;
  final Set<String> _resumingMailActivity = {};
  final Set<String> _resumingMailAccounts = {};
  final Set<String> _seenMailResumeAccounts = {};
  final Set<String> _blockedMailResumeAccounts = {};
  bool _mailResumeSweep = false, _mailResumePump = false;
  bool _mailResumeProgressed = false;
  int? _mailResumeAfterCreated;
  String? _mailResumeAfterId;
  String? _mailResumeQueryError;
  String? get mailResumeQueryError => _mailResumeQueryError;
  Iterable<MailActivity> get mailActivityReview =>
      mailActivities.where((action) => action.needsReview);
  Iterable<MailActivity> get mailActivityPending => mailActivities.where(
    (action) => const {'queued', 'waiting', 'running'}.contains(action.status),
  );
  final SettingsStore settings;
  List<Mail> _mail;
  final Map<String, Mail> _confirmed;
  final Map<String, int> _versions = {};
  final Map<String, Future<void>> _queues = {};
  final Map<String, Map<String, Object>> _projection = {};
  final Map<String, Draft> drafts = {};
  final Map<String, Draft> _pendingDrafts = {};
  final Map<String, String> _draftSaveErrors = {};
  final Map<String, int> _draftGenerations = {};
  int _draftGeneration = 0;
  String? draftSaveError(String id) => _draftSaveErrors[id];
  bool draftNeedsRetry(String id) => _draftSaveErrors.containsKey(id);

  int _stageDraft(Draft draft) {
    final generation = ++_draftGeneration;
    final current = _pendingDrafts[draft.id] ?? drafts[draft.id];
    if (current != null && current.revision > draft.revision) return generation;
    _pendingDrafts[draft.id] = draft;
    drafts[draft.id] = draft;
    _draftGenerations[draft.id] = generation;
    _changed();
    return generation;
  }

  void stageDraft(Draft draft) => _stageDraft(draft);

  Future<bool> retryDraft(String id) async {
    final draft = _pendingDrafts[id] ?? drafts[id];
    if (draft == null) return false;
    return saveDraft(draft);
  }

  void _forgetDraft(String id) {
    _draftGenerations[id] = ++_draftGeneration;
    drafts.remove(id);
    _pendingDrafts.remove(id);
    _draftSaveErrors.remove(id);
  }

  final Set<String> _removedAccounts = {};
  List<CalendarEntry> events;
  Preferences preferences = const Preferences();
  String folder = 'Inbox', query = '', filter = 'All';
  String? account;
  bool newestFirst = true, syncing = false, savingPreferences = false;
  String? preferenceSaveError;
  VoidCallback? _preferenceRetry;
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
  final Map<String, int> _preferenceFields = {};
  final Set<String> _unsavedPreferences = {};
  late final ProfileEnrollmentDevice _profileApplication =
      WorkspaceProfileApplication(this);
  ProfileEnrollmentDevice? get profileApplication =>
      settings is ProfileSettingsStore && repository is ProfileAccountRepository
      ? _profileApplication
      : null;
  bool needsReconnect(String id) =>
      repository is ProfileAccountRepository &&
      (repository as ProfileAccountRepository).reconnectAccounts.contains(id);
  final Map<String, AccountConnectionAttempt> _connectionAttempts = {};
  final Set<String> _hiddenConnectionAttempts = {};
  final Map<String, int> _connectionGenerations = {};
  int _connectionGeneration = 0;
  List<AccountConnectionAttempt> get connectionAttempts {
    final durable = repository is DurableAccountRepository
        ? (repository as DurableAccountRepository).connectionAttempts
        : const <AccountConnectionAttempt>[];
    return [
      ..._connectionAttempts.values,
      ...durable.where(
        (item) =>
            !_connectionAttempts.containsKey(item.id) &&
            !_hiddenConnectionAttempts.contains(item.id) &&
            !_removedAccounts.contains(item.account.id),
      ),
    ];
  }

  Future<bool> connectAccount(
    MailAccount account,
    String incoming,
    String smtp, {
    AccountConnectionAttempt? retry,
  }) async {
    final durable = repository is DurableAccountRepository
        ? repository as DurableAccountRepository
        : null;
    if (durable == null) {
      await accountRepository!.connect(account, incoming, smtp);
      return true;
    }
    final admitted =
        retry ??
        await durable.admitConnection(newConnectionAttemptId(), account);
    if (_removedAccounts.contains(account.id)) {
      await durable.abandonConnection(admitted.id);
      return false;
    }
    _connectionAttempts.removeWhere(
      (id, item) => item.account.id == account.id && id != admitted.id,
    );
    _hiddenConnectionAttempts.remove(admitted.id);
    final generation = ++_connectionGeneration;
    _connectionGenerations[admitted.id] = generation;
    _connectionAttempts[admitted.id] = admitted.copy(status: 'waiting');
    _changed();
    unawaited(
      _executeConnection(durable, admitted, incoming, smtp, generation),
    );
    return true;
  }

  Future<void> _executeConnection(
    DurableAccountRepository durable,
    AccountConnectionAttempt attempt,
    String incoming,
    String smtp,
    int generation,
  ) async {
    try {
      await durable.executeConnection(attempt, incoming, smtp);
      if (_connectionGenerations[attempt.id] != generation) return;
      _connectionAttempts.remove(attempt.id);
      _connectionGenerations.remove(attempt.id);
      unawaited(refresh());
    } catch (failure) {
      if (_connectionGenerations[attempt.id] != generation) return;
      final current = _connectionAttempts[attempt.id];
      if (current?.id != attempt.id) return;
      try {
        await durable.failConnection(attempt.id, '$failure');
      } catch (_) {
        // Keep the original failure available for retry in this session.
      }
      if (_connectionGenerations[attempt.id] != generation) return;
      _connectionAttempts[attempt.id] = current!.copy(
        status: 'reentry',
        error: '$failure',
      );
    }
    _changed();
  }

  Future<void> abandonConnection(String attempt) async {
    _connectionAttempts.remove(attempt);
    _connectionGenerations[attempt] = ++_connectionGeneration;
    _hiddenConnectionAttempts.add(attempt);
    _changed();
    if (repository case final DurableAccountRepository durable) {
      try {
        await durable.abandonConnection(attempt);
      } catch (failure) {
        _hiddenConnectionAttempts.remove(attempt);
        error = '$failure';
        _changed();
      }
    }
  }

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

  /// Captured selection and durable group actions live in the repository;
  /// these controllers hold bounded observations and counts only.
  late final MailSelection? selection = repository is SelectionRepository
      ? MailSelection(
          repository: repository as SelectionRepository,
          scope: _selectionScope,
          currentCount: () => resultCount,
          changed: _changed,
        )
      : null;
  late final MailGroups? groups = repository is GroupRepository
      ? MailGroups(
          repository: repository as GroupRepository,
          changed: _changed,
          refreshMail: _repaint,
        )
      : null;
  Map<String, Object?> _selectionScope() => {
    'folder': folder,
    'account': account,
    'query': query,
    'filter': filter,
    'oldest': !newestFirst,
    'projection': {
      for (final entry in _projection.entries) entry.key: Map.of(entry.value),
    },
  };
  Future<void> _repaint() async {
    if (accountRepository != null) {
      await loadPage();
      return;
    }
    if (pending == 0 && !_disposed) {
      final rows = repository.cached;
      _mail = List.of(rows);
      _confirmed
        ..clear()
        ..addEntries(rows.map((m) => MapEntry(m.id, m)));
      _changed();
    }
  }

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
    if (google case final GoogleConnection connection) {
      unawaited(connection.load());
    }
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
        for (final draft in native.savedDrafts) {
          final current = drafts[draft.id];
          if (!_pendingDrafts.containsKey(draft.id) &&
              (current == null || current.revision <= draft.revision)) {
            drafts[draft.id] = draft;
          }
        }
        await loadPage();
        await refreshMailActivity(resume: true);
      } catch (e) {
        error = '$e';
      }
      _syncTimer = Timer.periodic(const Duration(seconds: 15), (_) {
        if (_foreground && !syncing && native.mailAccounts.isNotEmpty) {
          unawaited(refresh());
        }
        if (_foreground && !savingPreferences && _unsavedPreferences.isEmpty) {
          unawaited(profileDiscovery?.syncTick(profileApplication));
        }
      });
    }
    if (native == null && repository is MailActivityRepository) {
      await refreshMailActivity(resume: true);
    }
    // Saved groups recover at startup: runnable ones continue, paused ones
    // wait for an explicit decision in History.
    unawaited(groups?.refreshHistory());
    _changed();
  }

  Future<void> refreshMailActivity({bool resume = false}) async {
    final source = switch (repository) {
      MailActivityRepository value => value,
      _ => null,
    };
    if (source == null) return;
    try {
      mailActivities = await source.mailActions();
      _changed();
      if (resume) {
        await _resumeQueuedMailActivity(source);
      }
    } catch (e) {
      error = 'Could not load mail activity. $e';
      _changed();
    }
  }

  Future<void> _resumeQueuedMailActivity(MailActivityRepository source) async {
    if (_disposed) return;
    if (!_mailResumeSweep) {
      _mailResumeSweep = true;
      _mailResumeProgressed = false;
      _mailResumeAfterCreated = null;
      _mailResumeAfterId = null;
      _blockedMailResumeAccounts.clear();
      _seenMailResumeAccounts.clear();
    }
    if (_mailResumePump) return;
    _mailResumePump = true;
    try {
      while (_resumingMailActivity.length < 32) {
        final List<MailActivity> runnable;
        try {
          runnable = await source.runnableMailActions(
            afterCreated: _mailResumeAfterCreated,
            afterId: _mailResumeAfterId,
          );
        } catch (e) {
          if (!_disposed) {
            _mailResumeQueryError =
                'Could not resume saved mail actions. Retry Activity. $e';
            error = _mailResumeQueryError;
            _changed();
          }
          _mailResumeSweep = false;
          _mailResumeAfterCreated = null;
          _mailResumeAfterId = null;
          return;
        }
        if (_disposed) {
          _mailResumeSweep = false;
          return;
        }
        if (_mailResumeQueryError case final previous?) {
          _mailResumeQueryError = null;
          if (error == previous) error = null;
          _changed();
        }
        if (runnable.isEmpty) {
          if (_resumingMailActivity.isEmpty) {
            _mailResumeAfterCreated = null;
            _mailResumeAfterId = null;
            if (_mailResumeProgressed) {
              _mailResumeProgressed = false;
              _seenMailResumeAccounts.clear();
              continue;
            }
            _mailResumeSweep = false;
          }
          return;
        }
        for (final action in runnable) {
          if (_resumingMailActivity.length >= 32) return;
          _mailResumeAfterCreated = action.created;
          _mailResumeAfterId = action.id;
          if (_resumingMailActivity.contains(action.id) ||
              _resumingMailAccounts.contains(action.account) ||
              _seenMailResumeAccounts.contains(action.account) ||
              _blockedMailResumeAccounts.contains(action.account)) {
            continue;
          }
          _resumingMailActivity.add(action.id);
          _resumingMailAccounts.add(action.account);
          _seenMailResumeAccounts.add(action.account);
          unawaited(_resumeMailActivity(source, action));
        }
      }
    } finally {
      _mailResumePump = false;
      if (!_disposed && _mailResumeSweep && _resumingMailActivity.isEmpty) {
        unawaited(Future.microtask(() => _resumeQueuedMailActivity(source)));
      }
    }
  }

  Future<void> _refreshMailProjection() async {
    if (accountRepository == null) return;
    await loadPage();
    final current = _reader;
    final native = accountRepository;
    if (current == null || native == null) return;
    final revision = _revision;
    try {
      final detail = await native.detail(current.id);
      if (revision == _revision && _reader?.id == current.id) {
        _reader = detail;
        _bodies.remove(current.id);
        _bodies[current.id] = detail;
        _changed();
      }
    } catch (_) {
      // The page refresh remains authoritative when the body is unavailable.
    }
  }

  Future<void> _resumeMailActivity(
    MailActivityRepository source,
    MailActivity action,
  ) async {
    try {
      await source.resumeMailAction(action);
      _mailResumeProgressed = true;
    } catch (e) {
      error = '$e';
      _blockedMailResumeAccounts.add(action.account);
    } finally {
      _resumingMailActivity.remove(action.id);
      _resumingMailAccounts.remove(action.account);
      if (!_disposed) await refreshMailActivity();
      if (!_disposed && _mailResumeSweep) {
        unawaited(_resumeQueuedMailActivity(source));
      }
    }
  }

  Future<void> retryMailActivity(MailActivity action) async {
    if (action.status == 'rejected') {
      await change(action.mail, action.fields, force: true);
    } else {
      final source = switch (repository) {
        MailActivityRepository value => value,
        _ => null,
      };
      if (source != null) await _inspectMailActivity(source, action);
    }
  }

  Future<void> _inspectMailActivity(
    MailActivityRepository source,
    MailActivity action,
  ) async {
    try {
      await source.inspectMailAction(action);
    } catch (e) {
      error = '$e';
    }
    await refreshMailActivity();
    await _refreshMailProjection();
  }

  Future<void> cancelMailActivity(MailActivity action) async {
    final source = switch (repository) {
      MailActivityRepository value => value,
      _ => null,
    };
    if (source == null) return;
    if (!action.canResume) return;
    try {
      await source.cancelMailAction(action.id);
    } catch (e) {
      error = '$e';
    }
    await refreshMailActivity();
    await _refreshMailProjection();
  }

  Future<void> undoMailActivity(
    MailActivity action, {
    void Function()? onAdmitted,
  }) async {
    final source = switch (repository) {
      MailActivityRepository value => value,
      _ => null,
    };
    if (source == null || !action.canUndo) return;
    try {
      await source.undoMailAction(
        action,
        onAdmitted: () {
          unawaited(_refreshMailProjection());
          onAdmitted?.call();
        },
      );
    } catch (e) {
      error = '$e';
    }
    await refreshMailActivity();
    await _refreshMailProjection();
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
      // Arrivals are observed, never selected: the capture decides.
      selection?.refresh();
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
      _unread = (_unread + after - before).clamp(0, 9007199254740991);
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
      total = (total + matching.length - beforeCount).clamp(
        0,
        9007199254740991,
      );
    }
  }

  Future<void> accountRemoved(String id) async {
    final removedAttempts = connectionAttempts
        .where((attempt) => attempt.account.id == id)
        .map((attempt) => attempt.id)
        .toList();
    _removedAccounts.add(id);
    _connectionAttempts.removeWhere((_, attempt) => attempt.account.id == id);
    for (final attempt in removedAttempts) {
      _hiddenConnectionAttempts.add(attempt);
      _connectionGenerations[attempt] = ++_connectionGeneration;
    }
    moves.removeAccount(id);
    undoFailures.removeWhere((r) => r.account == id);
    if (_readCandidate?.accountId == id) _readCandidate = null;
    final removedDrafts = drafts.values
        .where((draft) => draft.accountId == id)
        .map((draft) => draft.id)
        .toList();
    for (final draft in removedDrafts) {
      _forgetDraft(draft);
    }
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
    selection?.done();
    notice = 'Account removed from this device.';
    error = null;
    _changed();
    await loadPage();
    await groups?.refreshHistory();
  }

  Future<void> savePreferences(Preferences value) async {
    final before = preferences.profileSettings(),
        after = value.profileSettings();
    final changes = <String, Object?>{};
    for (final key in before.keys) {
      if (before[key] != after[key]) {
        _unsavedPreferences.add(key);
        _preferenceFields[key] = (_preferenceFields[key] ?? 0) + 1;
      }
    }
    for (final key in _unsavedPreferences) {
      changes[key] = after[key];
    }
    preferences = value;
    final generations = Map<String, int>.of(_preferenceFields);
    final revision = ++_settingsRevision;
    savingPreferences = true;
    _changed();
    _settingsQueue = _settingsQueue.then((_) async {
      try {
        if (settings case final ProfileSettingsStore profileStore) {
          final saved = await profileStore.saveLocal(changes);
          _mergeProfilePreferences(saved, generations);
        } else {
          await settings.write(value);
        }
        _unsavedPreferences.removeWhere(
          (key) => (_preferenceFields[key] ?? 0) == (generations[key] ?? 0),
        );
        if (revision == _settingsRevision) {
          notice = 'Preferences saved';
          if (error == preferenceSaveError) error = null;
          if (identical(retry, _preferenceRetry)) retry = null;
          preferenceSaveError = null;
          _preferenceRetry = null;
        }
      } catch (_) {
        if (revision == _settingsRevision) {
          final previousError = preferenceSaveError;
          preferenceSaveError =
              'Could not save preferences. Retry to keep these changes after restarting.';
          _preferenceRetry = () {
            unawaited(savePreferences(preferences));
          };
          if (error == null || error == previousError) {
            error = preferenceSaveError;
            retry = _preferenceRetry;
          }
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
      if (query != value) selection?.done();
      query = value;
      limit = 50;
      unawaited(loadPage());
      _changed();
    });
  }

  void navigate(String value, {String? inAccount}) {
    unawaited(finishReading());
    folder = value;
    account = inAccount;
    limit = 50;
    selection?.done();
    unawaited(loadPage());
    _changed();
  }

  void setFilter(String value) {
    unawaited(finishReading());
    filter = value;
    limit = 50;
    selection?.done();
    unawaited(loadPage());
    _changed();
  }

  void sort() {
    unawaited(finishReading());
    newestFirst = !newestFirst;
    limit = 50;
    selection?.done();
    unawaited(loadPage());
    _changed();
  }

  void more() {
    unawaited(finishReading());
    limit += 50;
    unawaited(loadPage(append: true));
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
      selection?.toggle(id);
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
    final observedLineage = current.lineage;
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
    final durable = switch (repository) {
      DurableMutationRepository value => value,
      _ => null,
    };
    final cancelOnly =
        durable != null &&
        restoring?.cancelled == true &&
        restoring?.actionId != null;
    final actionId = durable == null
        ? null
        : cancelOnly
        ? restoring!.actionId
        : newDraftIdentity();
    if (move != null) move.actionId = actionId;
    var admissionFailed = false;
    final admission = durable == null
        ? Future<Object?>.value()
        : (cancelOnly
                  ? (restoring!.admitted ?? Future<Object?>.value())
                        .then((failure) {
                          if (failure != null) throw failure;
                          return durable.cancelAdmittedMutation(actionId!);
                        })
                        .then((_) {
                          restoring.admissionCancelled = true;
                        })
                  : observedLineage == null
                  ? Future<void>.error(
                      const MailOperationFailure(
                        'This message changed since it was shown. Refresh the folder and retry.',
                      ),
                    )
                  : durable.admitMutation(
                      id,
                      fields,
                      actionId!,
                      observedLineage,
                    ))
              .then<Object?>(
                (_) => null,
                onError: (Object failure, StackTrace _) {
                  admissionFailed = true;
                  final target = _canonical(id);
                  final rollback = <String, Object>{};
                  for (final key in fields.keys) {
                    if (_versions['$target:$key'] == revision &&
                        _confirmed.containsKey(target)) {
                      rollback[key] = _confirmed[target]!.field(key);
                    }
                  }
                  _patchMail(target, rollback);
                  if (rollback.isNotEmpty) {
                    error =
                        'Could not update ${current.subject}. The affected display was restored. ${failure is MailOperationFailure ? failure.message : 'Retry.'}';
                  }
                  if (move != null && !move.undoRequested) moves.failed(move);
                  _changed();
                  return failure;
                },
              );
    if (move != null) {
      move.admitted = admission;
    }
    final before = Future.wait(
      _queues.entries
          .where((entry) => _canonical(entry.key) == id)
          .map((entry) => entry.value),
    );
    final job = admission.then((admissionFailure) async {
      if (admissionFailure != null) return;
      await before;
      var target = _canonical(id);
      if (!_confirmed.containsKey(target)) return;
      if (cancelOnly) return;
      if (move?.cancelled == true ||
          (restoring != null && !restoring.committed)) {
        if (durable != null && move?.admissionCancelled != true) {
          try {
            await durable.cancelAdmittedMutation(actionId!);
          } catch (e) {
            error = '$e';
            _changed();
          }
        }
        return;
      }
      if (move != null) move.started = true;
      try {
        if (durable != null) {
          await durable.executeMutation(target, fields, actionId!);
        } else {
          await repository.mutate(target, fields);
        }
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
    if (identical(_queues[id], job)) {
      if (admissionFailed) {
        _queues[id] = before;
        unawaited(
          before.whenComplete(() {
            if (identical(_queues[id], before)) {
              _queues.remove(id);
              _changed();
            }
          }),
        );
      } else {
        _queues.remove(id);
      }
    }
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
    final generation = _stageDraft(draft);
    try {
      await repository.saveDraft(draft);
      if (_removedAccounts.contains(draft.accountId)) {
        error =
            'This account was removed. Copy this text into a new draft with a connected account.';
        _changed();
        return false;
      }
      final owns = _draftGenerations[draft.id] == generation;
      final current = drafts[draft.id];
      if (owns && (current == null || current.revision <= draft.revision)) {
        drafts[draft.id] = draft;
      }
      if (owns) {
        _pendingDrafts.remove(draft.id);
        _draftSaveErrors.remove(draft.id);
      }
      notice = 'Draft saved';
      _changed();
      return true;
    } catch (e) {
      final message = e is MailOperationFailure
          ? e.message
          : 'Draft could not be saved. Retry here or from Drafts.';
      if (_draftGenerations[draft.id] == generation) {
        error = message;
        _draftSaveErrors[draft.id] = message;
      }
      _changed();
      return false;
    }
  }

  Future<bool> discardDraft(Draft draft) async {
    final native = accountRepository;
    if (native == null) return false;
    try {
      await native.discard(draft.id, draft.revision);
      _forgetDraft(draft.id);
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
      _forgetDraft(entry.draftId);
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
      _forgetDraft(draft.id);
      if (repository is OutgoingRepository) {
        notice = 'Message queued in Outbox.';
      }
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

  Future<void> cancelOutgoing(OutgoingEntry entry) async {
    final outbox = repository as OutgoingRepository;
    await outbox.cancelOutgoing(entry.id);
    notice = 'Delivery cancelled. The draft was kept.';
    _changed();
  }

  Future<bool> saveEvent(CalendarEntry entry) async {
    final revision = ++_calendarViewRevision;
    final durable = switch (repository) {
      DurableCalendarRepository value => value,
      _ => null,
    };
    if (durable != null) {
      final before = events
          .where(
            (event) => event.id == entry.id && event.sourceId == entry.sourceId,
          )
          .firstOrNull;
      final subject = _calendarSubject ?? google?.active?.subject ?? '';
      final key = '${entry.sourceId}\u0000${entry.id}';
      final mutation = <String, Object?>{
        'save': <String, Object?>{
          'before': before?.toCalendarJson(),
          'after': entry.toCalendarJson(),
        },
      };
      var pending = _calendarAdmissions[key];
      pending?.closed = false;
      if (pending != null &&
          jsonEncode(pending.mutation) != jsonEncode(mutation)) {
        try {
          final known = await durable.calendarActionAdmission(pending.id);
          if (known != null) {
            if (const {'rejected', 'cancelled'}.contains(known.status)) {
              _calendarAdmissions.remove(key);
              pending = null;
            } else if (known.status == 'succeeded' && known.saved != null) {
              final saved = known.saved!;
              if (_disposed || revision != _calendarViewRevision) {
                if (!_disposed) unawaited(refreshCalendarActivity());
                return false;
              }
              events = [
                ...events.where(
                  (event) =>
                      (event.id != entry.id ||
                          event.sourceId != entry.sourceId) &&
                      (event.id != saved.id ||
                          event.sourceId != saved.sourceId),
                ),
                saved,
              ];
              error =
                  'The previous event was saved with its provider identity. Reopen it before applying these edits.';
              _changed();
              return false;
            } else {
              if (!_disposed &&
                  const {
                    'queued',
                    'waiting',
                    'repair',
                  }.contains(known.status)) {
                unawaited(
                  _runCalendarAction(
                    known.id,
                    subject: known.subject,
                    repair: known.status == 'repair',
                  ),
                );
              } else if (!_disposed && known.status == 'uncertain') {
                unawaited(refreshCalendarActivity());
              }
              error =
                  'The previous save for this event must finish or be reviewed before saving these edits.';
              _changed();
              return false;
            }
          }
          if (known == null) {
            _calendarAdmissions.remove(key);
            pending = null;
          }
        } catch (exception) {
          error =
              'Could not confirm whether the previous save was admitted. Your edits are still open. $exception';
          _changed();
          return false;
        }
      }
      if (!_canReserveCalendarAdmission(pending)) return false;
      pending ??= _CalendarAdmissionAttempt(
        id: _calendarActionId(),
        subject: subject,
        mutation: mutation,
      );
      _calendarAdmissions[key] = pending;
      CalendarAdmission admission;
      try {
        admission =
            await durable.calendarActionAdmission(pending.id) ??
            await durable.admitCalendarAction(
              pending.id,
              entry,
              before,
              subject: pending.subject,
            );
      } catch (exception) {
        try {
          final known = await durable.calendarActionAdmission(pending.id);
          if (known == null) {
            error = 'The event was not admitted. Your edits are still open.';
            _changed();
            return false;
          }
          admission = known;
        } catch (_) {
          error =
              'Could not confirm whether the event was admitted. Your edits are still open. $exception';
          _changed();
          return false;
        }
      }
      if (admission.status != 'succeeded' || admission.saved == null) {
        _calendarAdmissions.remove(key);
      }
      if (const {'rejected', 'cancelled'}.contains(admission.status)) {
        await refreshCalendarActivity();
        error = 'The event was not saved. Your edits are still open.';
        _changed();
        return false;
      }
      if (!_disposed &&
          const {'queued', 'waiting', 'repair'}.contains(admission.status)) {
        unawaited(
          _runCalendarAction(
            admission.id,
            subject: admission.subject,
            repair: admission.status == 'repair',
          ),
        );
      }
      if (_disposed || revision != _calendarViewRevision) {
        if (!_disposed) unawaited(refreshCalendarActivity());
        return true;
      }
      _calendarViewRevision++;
      final projected = admission.saved ?? entry;
      events = [
        ...events.where(
          (event) =>
              (event.id != entry.id || event.sourceId != entry.sourceId) &&
              (event.id != projected.id ||
                  event.sourceId != projected.sourceId),
        ),
        projected,
      ];
      notice = admission.status == 'succeeded'
          ? 'Event saved'
          : admission.status == 'uncertain'
          ? 'Event needs checking'
          : 'Event queued';
      _changed();
      return true;
    }
    try {
      await repository.saveEvent(entry);
      events = [
        ...events.where(
          (event) => event.id != entry.id || event.sourceId != entry.sourceId,
        ),
        entry,
      ];
      notice = 'Event saved';
      _changed();
      return true;
    } catch (_) {
      error = 'Event could not be saved. Keep the form open and retry.';
      _changed();
      return false;
    }
  }

  String _calendarActionId() {
    final random = Random.secure();
    return '${DateTime.now().microsecondsSinceEpoch.toRadixString(16)}-${List.generate(16, (_) => random.nextInt(256).toRadixString(16).padLeft(2, '0')).join()}';
  }

  void releaseCalendarEditor(String sourceId, String eventId) {
    for (final key in [
      '$sourceId\u0000$eventId',
      '$sourceId\u0000$eventId\u0000delete',
    ]) {
      final pending = _calendarAdmissions[key];
      if (pending != null) {
        pending.closed = true;
        unawaited(_reconcileClosedCalendarEditor(key, pending));
      }
    }
  }

  bool _canReserveCalendarAdmission(_CalendarAdmissionAttempt? pending) {
    if (pending != null || _calendarAdmissions.length < 32) return true;
    error =
        'Check the unresolved calendar changes before saving another event.';
    _changed();
    return false;
  }

  Future<void> _reconcileClosedCalendarEditor(
    String key,
    _CalendarAdmissionAttempt pending,
  ) async {
    final durable = switch (repository) {
      DurableCalendarRepository value => value,
      _ => null,
    };
    if (durable == null) return;
    try {
      final known = await durable.calendarActionAdmission(pending.id);
      if (!pending.closed || !identical(_calendarAdmissions[key], pending)) {
        return;
      }
      if (known == null) {
        _calendarAdmissions.remove(key);
        return;
      }
      if (const {'succeeded', 'rejected', 'cancelled'}.contains(known.status)) {
        final revision = _calendarViewRevision;
        final snapshot = await durable.calendarSnapshot();
        if (_disposed ||
            revision != _calendarViewRevision ||
            !pending.closed ||
            !identical(_calendarAdmissions[key], pending)) {
          return;
        }
        _calendarViewRevision++;
        calendarSources = snapshot.sources;
        _calendarSubject = snapshot.subject;
        events = snapshot.events;
        _calendarAdmissions.remove(key);
        _changed();
        return;
      }
      if (!_disposed &&
          const {'queued', 'waiting', 'repair'}.contains(known.status)) {
        unawaited(
          _runCalendarAction(
            known.id,
            subject: known.subject,
            repair: known.status == 'repair',
          ),
        );
      } else if (!_disposed) {
        unawaited(refreshCalendarActivity());
      }
    } catch (_) {
      // Keep the exact identity until a later read establishes its outcome.
    }
  }

  Future<bool> deleteEvent(CalendarEntry entry) async {
    final revision = ++_calendarViewRevision;
    final durable = repository as DurableCalendarRepository;
    final subject = _calendarSubject ?? google?.active?.subject ?? '';
    final key = '${entry.sourceId}\u0000${entry.id}\u0000delete';
    final mutation = <String, Object?>{
      'delete': <String, Object?>{'before': entry.toCalendarJson()},
    };
    var pending = _calendarAdmissions[key];
    pending?.closed = false;
    if (pending != null &&
        jsonEncode(pending.mutation) != jsonEncode(mutation)) {
      try {
        final known = await durable.calendarActionAdmission(pending.id);
        if (known == null ||
            const {'rejected', 'cancelled'}.contains(known.status)) {
          _calendarAdmissions.remove(key);
          pending = null;
        } else {
          error =
              'The previous deletion must finish or be reviewed before deleting this changed event.';
          _changed();
          return false;
        }
      } catch (exception) {
        error =
            'Could not confirm the previous deletion. The changed event remains open. $exception';
        _changed();
        return false;
      }
    }
    if (!_canReserveCalendarAdmission(pending)) return false;
    pending ??= _CalendarAdmissionAttempt(
      id: _calendarActionId(),
      subject: subject,
      mutation: mutation,
    );
    _calendarAdmissions[key] = pending;
    CalendarAdmission admission;
    try {
      admission =
          await durable.calendarActionAdmission(pending.id) ??
          await durable.admitCalendarDelete(
            pending.id,
            entry,
            subject: pending.subject,
          );
    } catch (exception) {
      try {
        final known = await durable.calendarActionAdmission(pending.id);
        if (known == null) {
          error =
              'The deletion was not admitted. Retry to use the same request.';
          _changed();
          return false;
        }
        admission = known;
      } catch (_) {
        error =
            'Could not confirm whether the deletion was admitted. The event remains open. $exception';
        _changed();
        return false;
      }
    }
    _calendarAdmissions.remove(key);
    if (const {'rejected', 'cancelled'}.contains(admission.status)) {
      await refreshCalendarActivity();
      error = 'The event was not deleted.';
      _changed();
      return false;
    }
    if (!_disposed &&
        const {'queued', 'waiting', 'repair'}.contains(admission.status)) {
      unawaited(
        _runCalendarAction(
          admission.id,
          subject: admission.subject,
          repair: admission.status == 'repair',
        ),
      );
    }
    if (_disposed || revision != _calendarViewRevision) {
      if (!_disposed) unawaited(refreshCalendarActivity());
      return true;
    }
    _calendarViewRevision++;
    events = events
        .where(
          (event) => event.id != entry.id || event.sourceId != entry.sourceId,
        )
        .toList();
    notice = admission.status == 'succeeded'
        ? 'Event deleted'
        : admission.status == 'uncertain'
        ? 'Deletion needs checking'
        : 'Event deletion queued';
    _changed();
    return true;
  }

  Future<void> openCalendar() async {
    await refreshCalendarActivity();
    await syncCalendar();
    await refreshCalendarActivity(resume: true);
  }

  Future<void> syncCalendar() async {
    final durable = switch (repository) {
      DurableCalendarRepository value => value,
      _ => null,
    };
    if (durable == null || google == null) return;
    final revision = _calendarViewRevision;
    try {
      final scopes = google!.active?.permissions.scopes
          .where((scope) => scope.contains('/auth/calendar.'))
          .toList();
      final subject = google!.active?.subject;
      final token = await _calendarToken(subject, scopes ?? const []);
      final now = DateTime.now().toUtc();
      await durable.syncCalendar(
        token,
        now.subtract(const Duration(days: 365)),
        now.add(const Duration(days: 365)),
        subject: subject!,
      );
      if (_disposed) return;
      await refreshCalendarActivity();
    } catch (exception) {
      if (_disposed || revision != _calendarViewRevision) return;
      error = '$exception';
      _changed();
    }
  }

  Future<void> refreshCalendarActivity({bool resume = false}) async {
    final durable = switch (repository) {
      DurableCalendarRepository value => value,
      _ => null,
    };
    if (durable == null) return;
    final revision = ++_calendarViewRevision;
    try {
      final retired = <String, _CalendarAdmissionAttempt>{};
      for (final item in _calendarAdmissions.entries.toList()) {
        if (!item.value.closed) continue;
        try {
          final known = await durable.calendarActionAdmission(item.value.id);
          if (known == null ||
              const {
                'succeeded',
                'rejected',
                'cancelled',
              }.contains(known.status)) {
            retired[item.key] = item.value;
          }
        } catch (_) {
          // A failed lookup cannot retire an unknown local write.
        }
      }
      final activities = await durable.calendarActions();
      final snapshot = await durable.calendarSnapshot();
      if (_disposed || revision != _calendarViewRevision) return;
      calendarActivities = activities;
      calendarSources = snapshot.sources;
      _calendarSubject = snapshot.subject;
      events = snapshot.events;
      for (final item in retired.entries) {
        if (item.value.closed &&
            identical(_calendarAdmissions[item.key], item.value)) {
          _calendarAdmissions.remove(item.key);
        }
      }
      _changed();
      if (resume) {
        for (final action in calendarActivities.where(
          (action) => action.canResume,
        )) {
          unawaited(
            _runCalendarAction(
              action.id,
              subject: action.subject,
              repair: action.status == 'repair',
            ),
          );
        }
      }
    } catch (exception) {
      if (_disposed || revision != _calendarViewRevision) return;
      error = '$exception';
      _changed();
    }
  }

  Future<String> _calendarToken(String? subject, List<String> scopes) async {
    final connection = google;
    final active = connection?.active;
    if (subject == null ||
        subject.isEmpty ||
        connection == null ||
        active?.subject != subject) {
      throw StateError(
        'Reconnect the Google account used by this calendar, then retry.',
      );
    }
    final generation = connection.grantGeneration;
    final token = await connection.accessToken(scopes);
    if (connection.grantGeneration != generation ||
        !identical(connection.active, active)) {
      throw StateError(
        'Google changed while this request was starting. Retry with the original account.',
      );
    }
    return token;
  }

  Future<void> _runCalendarAction(
    String id, {
    required String? subject,
    bool repair = false,
  }) async {
    final durable = repository as DurableCalendarRepository;
    if (!_runningCalendarActions.add(id)) return;
    try {
      if (repair) {
        await durable.repairCalendarAction(id);
        return;
      }
      final token = await _calendarToken(subject, const [
        'https://www.googleapis.com/auth/calendar.events',
      ]);
      await durable.executeCalendarAction(id, token, subject: subject!);
    } catch (exception) {
      try {
        await durable.waitCalendarAction(id, '$exception');
      } catch (_) {}
    } finally {
      _runningCalendarActions.remove(id);
      await refreshCalendarActivity();
    }
  }

  Future<void> retryCalendarActivity(CalendarActivity action) async {
    if (!action.canResume) return;
    await _runCalendarAction(
      action.id,
      subject: action.subject,
      repair: action.status == 'repair',
    );
  }

  Future<void> cancelCalendarActivity(CalendarActivity action) async {
    final durable = repository as DurableCalendarRepository;
    if (!action.canCancel) return;
    _calendarViewRevision++;
    try {
      await durable.cancelCalendarAction(action.id);
    } catch (exception) {
      error = '$exception';
    }
    await refreshCalendarActivity();
  }

  Future<void> inspectCalendarActivity(CalendarActivity action) async {
    final durable = repository as DurableCalendarRepository;
    if (!action.canInspect) return;
    try {
      final token = await _calendarToken(action.subject, const [
        'https://www.googleapis.com/auth/calendar.events.readonly',
      ]);
      await durable.inspectCalendarAction(
        action.id,
        token,
        subject: action.subject!,
      );
    } catch (exception) {
      error = '$exception';
    }
    await refreshCalendarActivity();
  }

  @override
  void dispose() {
    _disposed = true;
    _mailResumeSweep = false;
    profileDiscovery?.dispose();
    google?.dispose();
    moves.dispose();
    selection?.dispose();
    groups?.dispose();
    _searchTimer?.cancel();
    _syncTimer?.cancel();
    super.dispose();
  }
}
