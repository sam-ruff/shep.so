import 'native_repository.dart';
import 'accounts.dart';
import 'profile_discovery.dart';
import 'profile_creation.dart';

class NativeProfileDiscovery
    implements ProfileDiscoveryRepository, ProfileCreationRepository {
  @override
  Future<dynamic> creation(String session, Map<String, Object?> command) =>
      _call({'op': 'profile_creation', 'session': session, 'command': command});
  NativeProfileDiscovery(this.repository);
  final NativeRepository repository;
  Future<dynamic> _call(Map<String, Object?> data) async {
    try {
      return await repository.callBackground(data);
    } on MailOperationFailure catch (error) {
      throw DiscoveryFailure(error.message);
    }
  }

  @override
  Future<DiscoverySession> open(
    String session,
    String accessToken,
    String namespace,
    String? expectedPrincipal,
  ) async => DiscoverySession.fromJson(
    await _call({
          'op': 'open_profile_discovery',
          'session': session,
          'access_token': accessToken,
          'namespace': namespace,
          'expected_principal': expectedPrincipal,
        })
        as Map<String, dynamic>,
  );
  Future<dynamic> _command(String session, Map<String, Object?> command) =>
      _call({
        'op': 'profile_discovery',
        'session': session,
        'command': command,
      });
  Future<DiscoveryState> _state(
    String session,
    Map<String, Object?> command,
  ) async => DiscoveryState.fromJson(
    await _command(session, command) as Map<String, dynamic>,
  );
  @override
  Future<DiscoveryState> state(String session) =>
      _state(session, {'kind': 'state'});
  @override
  Future<DiscoveryState> advance(String session) =>
      _state(session, {'kind': 'advance'});
  @override
  Future<DiscoveryState> retry(String session, int revision) =>
      _state(session, {'kind': 'retry', 'expected_revision': revision});
  @override
  Future<DiscoveryState> refresh(
    String session,
    int revision, {
    required bool full,
  }) => _state(session, {
    'kind': 'refresh',
    'expected_revision': revision,
    'full': full,
  });
  @override
  Future<List<DiscoveredProfile>> profiles(
    String session,
    String? after,
  ) async {
    final rows =
        await _command(session, {'kind': 'profiles', 'after': after}) as List;
    if (rows.length > 50) {
      throw const DiscoveryFailure(
        'Too many profile summaries were returned. Update Shep and retry.',
      );
    }
    return rows
        .map((r) => DiscoveredProfile.fromJson(r as Map<String, dynamic>))
        .toList();
  }

  @override
  Future<void> close(String session) async {
    await _call({'op': 'close_profile_discovery', 'session': session});
  }
}
