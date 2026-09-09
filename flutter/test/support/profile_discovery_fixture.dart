import 'dart:async';
import 'package:shep_mobile/data/profile_discovery.dart';

DiscoveryState discoveryState({
  int revision = 0,
  String phase = 'initial',
  int files = 0,
  int profiles = 0,
  String? error,
}) => DiscoveryState.fromJson({
  'revision': revision,
  'phase': phase,
  'files': files,
  'profiles': profiles,
  'pending': 0,
  'incomplete_profiles': 0,
  'error': error,
});
String fixtureUuid(int value) =>
    '00000000-0000-0000-0000-${value.toRadixString(16).padLeft(12, '0')}';

class FixtureProfileDiscovery implements ProfileDiscoveryRepository {
  FixtureProfileDiscovery({this.count = 2});
  final int count;
  static const principal = 'drive:fixture-owner';
  DiscoveryState saved = discoveryState();
  String? active;
  int opens = 0,
      closes = 0,
      advances = 0,
      reads = 0,
      refreshes = 0,
      retries = 0;
  bool failNext = false, failClose = false, wrongSession = false;
  Completer<void>? holdOpen, holdAdvance, holdRefresh;
  @override
  Future<DiscoverySession> open(
    String session,
    String token,
    String namespace,
    String? expectedPrincipal,
  ) async {
    opens++;
    await holdOpen?.future;
    if (active != null) {
      throw const DiscoveryFailure('Another discovery session is still open.');
    }
    if (expectedPrincipal != null && expectedPrincipal != principal) {
      throw const DiscoveryFailure('Drive returned a different account.');
    }
    active = session;
    return DiscoverySession.fromJson({
      'session': wrongSession ? fixtureUuid(1) : session,
      'scope': {'namespace': namespace, 'principal': principal},
      'state': {
        'revision': saved.revision,
        'phase': saved.phase,
        'files': saved.files,
        'profiles': saved.profiles,
        'pending': 0,
        'incomplete_profiles': 0,
        'error': saved.error,
      },
    });
  }

  void _check(String session) {
    if (active != session) {
      throw const DiscoveryFailure('Profile discovery changed.');
    }
  }

  @override
  Future<DiscoveryState> state(String session) async {
    _check(session);
    return saved;
  }

  @override
  Future<DiscoveryState> advance(String session) async {
    _check(session);
    advances++;
    await holdAdvance?.future;
    if (failNext) {
      failNext = false;
      saved = discoveryState(
        revision: saved.revision + 1,
        phase: saved.phase,
        files: saved.files,
        profiles: saved.profiles,
        error: 'Profile files could not be read. Retry discovery.',
      );
      throw DiscoveryFailure(saved.error!);
    }
    saved = discoveryState(
      revision: saved.revision + 1,
      phase: saved.phase == 'initial'
          ? 'files'
          : saved.phase == 'files'
          ? 'changes'
          : 'complete',
      files: count,
      profiles: count,
    );
    return saved;
  }

  @override
  Future<DiscoveryState> retry(String session, int revision) async {
    _check(session);
    retries++;
    saved = discoveryState(
      revision: saved.revision + 1,
      phase: saved.phase,
      files: saved.files,
      profiles: saved.profiles,
    );
    return saved;
  }

  @override
  Future<DiscoveryState> refresh(
    String session,
    int revision, {
    required bool full,
  }) async {
    _check(session);
    refreshes++;
    await holdRefresh?.future;
    saved = discoveryState(
      revision: saved.revision + 1,
      phase: full ? 'initial' : 'changes',
      files: saved.files,
      profiles: saved.profiles,
    );
    return saved;
  }

  @override
  Future<List<DiscoveredProfile>> profiles(
    String session,
    String? after,
  ) async {
    _check(session);
    reads++;
    final rows = List.generate(
      saved.profiles,
      (i) => DiscoveredProfile.fromJson({
        'profile': fixtureUuid(100 + i),
        'generation': fixtureUuid(10000 + i),
        'name': i == 0
            ? 'Personal'
            : i == 1
            ? 'Work'
            : 'Profile ${i + 1}',
        'name_conflict': false,
        'accounts': i == 0 ? 2 : 1,
        'settings': 3,
        'waiting': 0,
        'ready': 0,
        'conflicts': i == 1 ? 1 : 0,
        'removed': false,
        'initialized': true,
      }),
    );
    return rows
        .where((r) => after == null || r.cursor.compareTo(after) > 0)
        .take(50)
        .toList();
  }

  @override
  Future<void> close(String session) async {
    closes++;
    if (failClose) {
      throw const DiscoveryFailure('Could not close discovery. Retry.');
    }
    if (active == session) active = null;
  }
}
