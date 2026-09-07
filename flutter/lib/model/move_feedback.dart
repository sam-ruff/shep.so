import 'dart:async';

/// A UI operation identity, independent of the provider's changing mail UID.
class MoveRecord {
  MoveRecord(this.id, this.account, this.originalFolder);
  String id;
  final String account, originalFolder;
  bool started = false, cancelled = false, committed = false;
  bool undoRequested = false, blocked = false, restoreCommitted = false;
}

/// The desktop's six-second, destination-scoped move notification contract.
/// Records captured by in-flight work outlive dismissal, but cannot revive it.
class MoveFeedback {
  MoveFeedback(this.changed);
  final void Function() changed;
  Timer? _timer;
  String? _folder, _account;
  bool _restored = false;
  List<MoveRecord> _records = [];
  List<MoveRecord> get records => _records;
  bool get visible => _records.isNotEmpty;
  bool get canUndo => !_restored && _records.any((r) => !r.blocked);
  String? get label {
    if (!visible) return null;
    final count = _records.length, noun = count == 1 ? 'message' : 'messages';
    if (_restored) return 'Restored $count $noun';
    return switch (_folder?.toLowerCase()) {
      'archive' => 'Archived $count $noun',
      'trash' => 'Deleted $count $noun',
      'inbox' => 'Moved $count $noun to Inbox',
      _ => 'Moved $count $noun to $_folder',
    };
  }

  void _schedule() {
    _timer?.cancel();
    _timer = Timer(const Duration(seconds: 6), dismiss);
  }

  MoveRecord add(String id, String account, String original, String folder) {
    final standard = ['archive', 'trash'].contains(folder.toLowerCase());
    final sameFolder = standard
        ? _folder?.toLowerCase() == folder.toLowerCase()
        : _folder == folder;
    if (_restored || !sameFolder || (!standard && _account != account)) {
      _records = [];
    }
    _folder = folder;
    _account = account;
    _restored = false;
    final record = MoveRecord(id, account, original);
    _records = [..._records, record];
    _schedule();
    return record;
  }

  List<MoveRecord> restore(List<MoveRecord> expected) {
    if (!canUndo || !identical(expected, _records)) return [];
    final accepted = _records
        .where((r) => !r.blocked)
        .toList()
        .reversed
        .toList();
    for (final record in accepted) {
      record.undoRequested = true;
      if (!record.started) record.cancelled = true;
    }
    _records = accepted;
    _restored = true;
    _schedule();
    return accepted;
  }

  void failed(MoveRecord record) {
    if (!_records.contains(record)) return;
    _records = _records.where((r) => !identical(r, record)).toList();
    if (_records.isEmpty) _timer?.cancel();
  }

  void retryRestore(MoveRecord record) {
    if (!_restored) _records = [];
    _restored = true;
    if (!_records.contains(record)) _records = [..._records, record];
    _schedule();
  }

  void removeAccount(String account) {
    if (!_records.any((r) => r.account == account)) return;
    _records = _records.where((r) => r.account != account).toList();
    if (_records.isEmpty) _timer?.cancel();
  }

  void dismiss() {
    _timer?.cancel();
    _records = [];
    changed();
  }

  void dispose() => _timer?.cancel();
}
