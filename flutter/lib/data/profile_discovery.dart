/// Small remote observations. Caught-up discovery is distinct from enrollment,
/// credential availability and applying a profile to local accounts/settings.
class DiscoveredProfile {
  DiscoveredProfile.fromJson(Map<String, dynamic> data)
    : revision = data['revision'] as int? ?? 0,
      profile = data['profile'] as String,
      generation = data['generation'] as String,
      name = data['name'] as String?,
      nameConflict = data['name_conflict'] as bool,
      accounts = data['accounts'] as int,
      settings = data['settings'] as int,
      waiting = data['waiting'] as int,
      ready = data['ready'] as int,
      conflicts = data['conflicts'] as int,
      removed = data['removed'] as bool,
      initialized = data['initialized'] as bool? ?? false;
  final String profile, generation;
  final String? name;
  final bool nameConflict, removed, initialized;
  final int revision, accounts, settings, waiting, ready, conflicts;
  String get cursor => '$profile:$generation';
}

class DiscoveryState {
  DiscoveryState.fromJson(Map<String, dynamic> data)
    : revision = data['revision'] as int,
      phase = data['phase'] as String,
      files = data['files'] as int,
      profiles = data['profiles'] as int,
      pending = data['pending'] as int,
      incompleteProfiles = data['incomplete_profiles'] as int,
      error = data['error'] as String?;
  final int revision, files, profiles, pending, incompleteProfiles;
  final String phase;
  final String? error;
  bool get complete => phase == 'complete' && error == null;
}

class DiscoverySession {
  DiscoverySession.fromJson(Map<String, dynamic> data)
    : id = data['session'] as String,
      namespace =
          (data['scope'] as Map<String, dynamic>)['namespace'] as String,
      principal =
          (data['scope'] as Map<String, dynamic>)['principal'] as String,
      state = DiscoveryState.fromJson(data['state'] as Map<String, dynamic>);
  final String id, namespace, principal;
  final DiscoveryState state;
}

class DiscoveryFailure implements Exception {
  const DiscoveryFailure(this.message);
  final String message;
}

abstract interface class ProfileDiscoveryRepository {
  Future<DiscoverySession> open(
    String session,
    String accessToken,
    String namespace,
    String? expectedPrincipal,
  );
  Future<DiscoveryState> state(String session);
  Future<DiscoveryState> advance(String session);
  Future<DiscoveryState> retry(String session, int revision);
  Future<DiscoveryState> refresh(
    String session,
    int revision, {
    required bool full,
  });
  Future<List<DiscoveredProfile>> profiles(String session, String? after);
  Future<void> close(String session);
}
