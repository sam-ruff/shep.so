part of 'profile_discovery.dart';

extension ProfileCreationActions on ProfileDiscovery {
  bool get supportsCreation => repository is ProfileCreationRepository;
  bool get canCreate =>
      supportsCreation &&
      connected &&
      !busy &&
      state?.complete == true &&
      error == null;
  Future<dynamic> _creation(String session, Map<String, Object?> command) =>
      (repository as ProfileCreationRepository).creation(session, command);
  Future<void> _readCreation(int generation, String session) async {
    if (!supportsCreation) return;
    final value = await _creation(session, {'kind': 'current'});
    if (!_current(generation, session)) return;
    final next = value == null
        ? null
        : ProfileCreation.fromJson(value as Map<String, dynamic>);
    if (next?.id != creation?.id) {
      creationAccounts = [];
      creationAfter = 0;
    }
    creation = next;
    _changed();
  }

  Future<void> _creationPage(int generation, String session, int after) async {
    final current = creation;
    if (current == null) return;
    final value =
        await _creation(session, {
              'kind': 'accounts',
              'id': current.id,
              'after': after,
            })
            as List;
    if (!_current(generation, session) || creation?.id != current.id) return;
    if (value.length > 50) {
      throw const DiscoveryFailure(
        'Too many accounts were returned. Reopen the profile review.',
      );
    }
    creationAccounts = value
        .map((v) => ProfileAccountReview.fromJson(v as Map<String, dynamic>))
        .toList();
    creationAfter = after;
    _changed();
  }

  Future<void> _creationTask(Future<void> Function(int, String) action) {
    if (_work case final Future<void> pending) return pending;
    if (_disposed ||
        busy ||
        !connected ||
        !supportsCreation ||
        _session == null) {
      return Future.value();
    }
    final generation = _generation, session = _session!;
    busy = true;
    paused = false;
    error = null;
    _changed();
    final work = Future<void>.microtask(() async {
      try {
        await action(generation, session);
      } catch (failure) {
        if (_current(generation, session)) {
          error = _message(failure);
          try {
            await _readCreation(generation, session);
          } catch (_) {
            /* Retain the original failure. */
          }
        }
      } finally {
        if (_disposed || generation != _generation) await _retireQuietly();
        busy = false;
        publishing = false;
        _work = null;
        _changed();
      }
    });
    _work = work;
    return work;
  }

  Future<void> prepareCreation(
    String name, {
    required bool accounts,
    required Map<String, Object?> settings,
  }) => _creationTask((generation, session) async {
    final value = await _creation(session, {
      'kind': 'prepare',
      'id': _identity(),
      'specification': {
        'name': name.trim(),
        'include_accounts': accounts,
        'settings': settings,
      },
    });
    if (!_current(generation, session)) return;
    creation = ProfileCreation.fromJson(value as Map<String, dynamic>);
    creationAccounts = [];
    creationAfter = 0;
    await _creationPage(generation, session, 0);
  });
  Future<void> reviewCreationAccounts({required bool first}) => _creationTask(
    (generation, session) => _creationPage(
      generation,
      session,
      first ? 0 : creationAccounts.lastOrNull?.position ?? 0,
    ),
  );
  Future<void> cancelCreation() => _creationTask((generation, session) async {
    final current = creation;
    if (current == null) return;
    await _creation(session, {'kind': 'cancel', 'id': current.id});
    if (!_current(generation, session)) return;
    creation = null;
    creationAccounts = [];
    creationAfter = 0;
  });
  Future<void> approveCreation(Map<String, Object?> settings) =>
      _creationTask((generation, session) async {
        final current = creation;
        if (current == null) return;
        final value = await _creation(session, {
          'kind': 'approve',
          'id': current.id,
          'settings': settings,
        });
        if (!_current(generation, session)) return;
        creation = ProfileCreation.fromJson(value as Map<String, dynamic>);
        await _publishCreation(generation, session);
      });
  Future<void> resumeCreation() async {
    if (busy || !connected || !supportsCreation) return;
    // Refresh the SDK token and finish saved discovery before another write.
    await discover();
    if (error != null ||
        busy ||
        creation == null ||
        creation!.needsReview ||
        creation!.complete) {
      return;
    }
    await _creationTask(_publishCreation);
  }

  Future<void> _publishCreation(int generation, String session) async {
    publishing = true;
    _changed();
    while (_current(generation, session) &&
        !paused &&
        creation != null &&
        !creation!.complete) {
      final value = await _creation(session, {
        'kind': 'step',
        'id': creation!.id,
      });
      if (!_current(generation, session)) return;
      creation = ProfileCreation.fromJson(value as Map<String, dynamic>);
      _changed();
    }
    if (_current(generation, session)) {
      final observed = await repository.state(session);
      if (!_current(generation, session)) return;
      state = observed;
      await _page(generation, session, null);
    }
  }
}
