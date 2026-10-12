import '../data/incoming_sync.dart';

class _Episode {
  _Episode(this.identity, this.firstFailure, this.error);
  final IncomingIdentity identity;
  final DateTime firstFailure;
  String error;
  bool dismissed = false;
}

class SyncNotices {
  SyncNotices({DateTime Function()? now}) : _now = now ?? _elapsedClock();
  static DateTime Function() _elapsedClock() {
    final watch = Stopwatch()..start();
    return () => DateTime.fromMicrosecondsSinceEpoch(
      watch.elapsedMicroseconds,
      isUtc: true,
    );
  }

  final DateTime Function() _now;
  final Map<String, _Episode> _episodes = {};
  List<IncomingResult> _manualFailures = const [];

  void reconcile(List<IncomingAccount> accounts) {
    final identities = {
      for (final account in accounts) account.id: account.identity,
    };
    _episodes.removeWhere((id, episode) => identities[id] != episode.identity);
    _manualFailures = _manualFailures
        .where(
          (result) => identities[result.account.id] == result.account.identity,
        )
        .toList();
  }

  List<IncomingResult> observe(
    List<IncomingResult> results,
    List<IncomingAccount> accounts, {
    required bool automatic,
  }) {
    reconcile(accounts);
    final identities = {
      for (final account in accounts) account.id: account.identity,
    };
    final current = results
        .where(
          (result) => identities[result.account.id] == result.account.identity,
        )
        .toList();
    for (final result in current) {
      final error = result.error;
      if (error == null) {
        _episodes.remove(result.account.id);
      } else {
        final episode = _episodes[result.account.id];
        if (episode == null && automatic) {
          _episodes[result.account.id] = _Episode(
            result.account.identity,
            _now(),
            error,
          );
        } else if (episode != null) {
          episode.error = error;
        }
      }
    }
    _manualFailures = [
      ..._manualFailures.where(
        (failure) => !current.any(
          (result) =>
              result.account.id == failure.account.id &&
              (!automatic || result.error == null),
        ),
      ),
      if (!automatic) ...current.where((result) => result.error != null),
    ];
    return current;
  }

  String? visibleNotice(List<IncomingAccount> accounts) {
    reconcile(accounts);
    final current = {for (final account in accounts) account.id: account};
    return describe([
          for (final result in _manualFailures)
            IncomingResult(current[result.account.id]!, error: result.error),
        ]) ??
        notice(accounts);
  }

  String? notice(List<IncomingAccount> accounts) {
    reconcile(accounts);
    final failures = <IncomingResult>[];
    for (final account in accounts) {
      final episode = _episodes[account.id];
      if (episode == null ||
          episode.dismissed ||
          _now().difference(episode.firstFailure) <
              const Duration(seconds: 30)) {
        continue;
      }
      failures.add(IncomingResult(account, error: episode.error));
    }
    return describe(failures);
  }

  Duration? nextNoticeDelay(List<IncomingAccount> accounts) {
    reconcile(accounts);
    Duration? next;
    for (final episode in _episodes.values) {
      if (episode.dismissed) continue;
      final remaining =
          const Duration(seconds: 30) - _now().difference(episode.firstFailure);
      if (remaining <= Duration.zero) continue;
      if (next == null || remaining < next) next = remaining;
    }
    return next;
  }

  void dismiss() {
    _manualFailures = const [];
    for (final episode in _episodes.values) {
      if (_now().difference(episode.firstFailure) >=
          const Duration(seconds: 30)) {
        episode.dismissed = true;
      }
    }
  }

  static String? describe(List<IncomingResult> results) {
    final failures = results.where((result) => result.error != null).toList();
    if (failures.isEmpty) return null;
    if (failures.length == 1) {
      final result = failures.single;
      return '${result.account.name} sync failed: ${result.error}';
    }
    return '${failures.length} accounts could not refresh:\n${failures.map((result) => '${result.account.name}: ${result.error}').join('\n')}';
  }
}
