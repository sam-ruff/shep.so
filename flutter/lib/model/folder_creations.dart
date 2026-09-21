import 'dart:async';
import 'dart:math';
import 'package:flutter/foundation.dart';
import '../data/folders.dart';

class FolderCreations extends ChangeNotifier {
  FolderCreations(this.repository, {required this.changed});
  final FolderCreationRepository repository;
  final VoidCallback changed;
  List<FolderCreation> entries = const [];
  List<FolderAccount> accounts = const [];
  String? error;
  final Set<String> _running = {}, _accounts = {};
  final Map<String, String> failures = {};
  bool _disposed = false, _foreground = true;
  int _observation = 0;
  Timer? _timer;
  bool get supportsChanges => repository is FolderChangeRepository;
  Iterable<String> visibleNames(String account, Iterable<String> names) {
    final hidden = <String>{
      for (final entry in entries.where(
        (entry) =>
            entry.account == account &&
            entry.pending &&
            entry.status != 'rejected' &&
            entry.mutation != null,
      ))
        for (final member
            in (entry.plan?['members'] as List? ?? const []).cast<Map>())
          member['mailbox']['name'] as String,
    };
    return names.where((name) => !hidden.contains(name));
  }

  Future<Map<String, dynamic>> reviewChange(
    FolderAccount account,
    String source,
    Object action,
  ) {
    if (_disposed || !_foreground) {
      throw StateError('Reopen the workspace before reviewing a folder.');
    }
    return (repository as FolderChangeRepository).reviewFolderChange(
      account,
      source,
      action,
    );
  }

  Future<void> admitChange(String id, Map<String, dynamic> review) async {
    if (_disposed || !_foreground) {
      throw StateError('Reopen the workspace before changing a folder.');
    }
    final result = await (repository as FolderChangeRepository)
        .admitFolderChange(id, review);
    if (_disposed) return;
    ++_observation;
    entries = [
      result,
      ...entries.where((entry) => entry.id != result.id),
    ].take(50).toList();
    _changed();
    if (_foreground) unawaited(_execute(result));
  }

  String parentLabel(FolderCreation entry) {
    final parent = entry.parent;
    if (parent == null) return 'Account root';
    return accounts
            .where((account) => account.id == entry.account)
            .firstOrNull
            ?.parentLabel(parent) ??
        parent;
  }

  static String identity() {
    final random = Random.secure();
    final bytes = List.generate(16, (_) => random.nextInt(256));
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    final value = bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
    return '${value.substring(0, 8)}-${value.substring(8, 12)}-${value.substring(12, 16)}-${value.substring(16, 20)}-${value.substring(20)}';
  }

  void _changed() {
    if (_disposed) return;
    notifyListeners();
    changed();
  }

  Future<void> initialise() async {
    await refresh(resume: true);
    if (_disposed) return;
    _timer ??= Timer.periodic(const Duration(seconds: 15), (_) {
      if (_foreground) unawaited(refresh(resume: true));
    });
  }

  void foreground(bool active) {
    _foreground = active;
    if (active) unawaited(refresh(resume: true));
  }

  Future<void> refresh({bool resume = false}) async {
    if (_disposed) return;
    final generation = ++_observation;
    try {
      final currentAccounts = await repository.folderOptions();
      final currentEntries = await repository.folderCreations();
      if (_disposed || generation != _observation) return;
      accounts = currentAccounts;
      entries = currentEntries;
      failures.removeWhere(
        (id, _) => !entries.any((entry) => entry.id == id && entry.pending),
      );
      error = null;
      _changed();
      if (resume && _foreground) {
        for (final entry in entries) {
          if (entry.runnable &&
              !_running.contains(entry.id) &&
              !_accounts.contains(entry.account) &&
              _running.length < 8) {
            unawaited(_execute(entry));
          }
        }
      }
    } catch (_) {
      if (_disposed || generation != _observation) return;
      error = 'Could not load folder activity. Refresh to try again.';
      _changed();
    }
  }

  Future<void> admit(
    String id,
    FolderAccount account,
    String? parent,
    String name,
  ) async {
    if (_disposed || !_foreground) {
      throw StateError('Reopen the workspace before creating a folder.');
    }
    final result = await repository.admitFolder(id, account, parent, name);
    if (_disposed) return;
    ++_observation;
    entries = [
      result,
      ...entries.where((entry) => entry.id != result.id),
    ].take(50).toList();
    _changed();
    if (_foreground) unawaited(_execute(result));
  }

  Future<void> decide(FolderCreation entry, String decision) async {
    if (_disposed || !_foreground) return;
    try {
      final result = await repository.decideFolder(entry, decision);
      if (_disposed) return;
      ++_observation;
      entries = [
        for (final existing in entries)
          existing.id == result.id ? result : existing,
      ];
      failures.remove(entry.id);
      _changed();
      if (result.runnable && _foreground) unawaited(_execute(result));
    } catch (_) {
      if (_disposed) return;
      failures[entry.id] =
          'Could not save that decision. Refresh folder activity and try again.';
      _changed();
    }
  }

  Future<void> _execute(FolderCreation entry) async {
    if (_disposed ||
        !_foreground ||
        _running.contains(entry.id) ||
        _accounts.contains(entry.account)) {
      return;
    }
    _running.add(entry.id);
    _accounts.add(entry.account);
    var completed = false;
    try {
      final result = await repository.executeFolder(entry);
      completed =
          result.status == 'succeeded' ||
          result.mutation != null &&
              (result.status == 'queued' ||
                  result.status == 'repair' &&
                      result.hasReceipt &&
                      result.revision > entry.revision);
      failures.remove(entry.id);
    } catch (_) {
      failures[entry.id] =
          'This folder request needs attention. Reconnect the account or refresh its saved status.';
    } finally {
      _running.remove(entry.id);
      _accounts.remove(entry.account);
      if (!_disposed) await refresh(resume: completed);
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    super.dispose();
  }
}
