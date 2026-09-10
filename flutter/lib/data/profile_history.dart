import 'native_repository.dart';

/// Local journal addressing. These strings do not prove Google ownership:
/// authenticated discovery must supply the verified principal and namespace.
class ProfileHistoryBinding {
  const ProfileHistoryBinding({
    required this.namespace,
    required this.principal,
    required this.profile,
    required this.generation,
  });
  final String namespace, principal, profile, generation;
  Map<String, Object> toJson() => {
    'namespace': namespace,
    'principal': principal,
    'profile': profile,
    'generation': generation,
  };
}

class ProfileHistoryState {
  ProfileHistoryState.fromJson(Map<String, dynamic> data)
    : device = data['device'] as String,
      revision = data['revision'] as int,
      operations = data['operations'] as int,
      waiting = data['waiting'] as int,
      ready = data['ready'] as int,
      queued = data['queued'] as int,
      fields = data['fields'] as int,
      conflicts = data['conflicts'] as int,
      removed = data['removed'] as bool;
  final String device;
  final int revision, operations, waiting, ready, queued, fields, conflicts;
  final bool removed;
}

/// Native SQLite history behind the production Rust bridge. No mail account,
/// credential or remote provider is mutated through these staging operations.
class NativeProfileHistory {
  NativeProfileHistory(this.repository, this.binding);
  final NativeRepository repository;
  final ProfileHistoryBinding binding;

  Future<dynamic> _command(
    String expected,
    Map<String, Object?> command,
  ) async {
    final reply =
        await repository.callBackground({
              'op': 'profile_history',
              'binding': binding.toJson(),
              'command': command,
            })
            as Map<String, dynamic>;
    if (reply['kind'] != expected) {
      throw StateError(
        'Profile history returned an unexpected response. Update Shep and retry.',
      );
    }
    return reply['value'];
  }

  Future<ProfileHistoryState> _state(Map<String, Object?> command) async =>
      ProfileHistoryState.fromJson(
        await _command('state', command) as Map<String, dynamic>,
      );
  Future<ProfileHistoryState> state() => _state({'kind': 'state'});
  Future<ProfileHistoryState> importRecord(String record) =>
      _state({'kind': 'import', 'record': record});
  Future<ProfileHistoryState> drain() => _state({'kind': 'drain'});

  /// Freeze the operation UUID and complete edit before calling. Retry those
  /// same bytes/values after a lost reply; a new UUID would be a different edit.
  Future<ProfileHistoryState> edit(Map<String, Object?> frozenEdit) =>
      _state({'kind': 'edit', 'edit': frozenEdit});
  Future<List<Map<String, dynamic>>> fields({String? after}) async =>
      (await _command('fields', {'kind': 'fields', 'after': after}) as List)
          .cast<Map<String, dynamic>>();
  Future<List<Map<String, dynamic>>> versions(
    String target, {
    String? after,
  }) async =>
      (await _command('versions', {
                'kind': 'versions',
                'target': target,
                'after': after,
              })
              as List)
          .cast<Map<String, dynamic>>();
  Future<Map<String, dynamic>> value(String target, String operation) async =>
      await _command('value', {
            'kind': 'value',
            'target': target,
            'operation': operation,
          })
          as Map<String, dynamic>;
  Future<Map<String, dynamic>?> nextUpload() async =>
      await _command('upload', {'kind': 'next_upload'})
          as Map<String, dynamic>?;
  Future<ProfileHistoryState> reserve(String operation, String fileId) =>
      _state({'kind': 'reserve', 'operation': operation, 'file_id': fileId});
  Future<ProfileHistoryState> confirm(
    String operation,
    String fileId,
    String sha256,
  ) => _state({
    'kind': 'confirm',
    'operation': operation,
    'file_id': fileId,
    'sha256': sha256,
  });
  Future<void> close() async {
    await repository.callBackground({
      'op': 'close_profile_history',
      'binding': binding.toJson(),
    });
  }
}
