part of 'profile_discovery.dart';

extension ProfileEnrollmentActions on ProfileDiscovery {
  bool get supportsEnrollment => repository is ProfileEnrollmentRepository;
  bool get canEnroll =>
      supportsEnrollment &&
      connected &&
      !busy &&
      state?.complete == true &&
      error == null;
  Future<dynamic> _enrollment(String session, Map<String, Object?> command) =>
      (repository as ProfileEnrollmentRepository).enrollment(session, command);
  Future<void> _readEnrollment(int generation, String session) async {
    if (!supportsEnrollment) return;
    final value = await _enrollment(session, {'kind': 'current'});
    if (!_current(generation, session)) return;
    final next = value == null
        ? null
        : ProfileEnrollment.fromJson(value as Map<String, dynamic>);
    if (next?.id != enrollment?.id) {
      enrollmentRows = [];
      enrollmentAfter = 0;
    }
    enrollment = next;
    _changed();
  }

  Future<void> _enrollmentPage(
    int generation,
    String session,
    int after,
  ) async {
    final current = enrollment;
    if (current == null) return;
    final value =
        await _enrollment(session, {
              'kind': 'rows',
              'id': current.id,
              'after': after,
            })
            as List;
    if (!_current(generation, session) || enrollment?.id != current.id) return;
    if (value.length > 50) {
      throw const DiscoveryFailure(
        'Too many review rows. Reopen this enrollment.',
      );
    }
    enrollmentRows = value
        .map((v) => EnrollmentRow.fromJson(v as Map<String, dynamic>))
        .toList();
    enrollmentAfter = after;
    _changed();
  }

  Future<void> _enrollmentTask(Future<void> Function(int, String) action) {
    if (_work case final Future<void> pending) return pending;
    if (_disposed ||
        busy ||
        !connected ||
        !supportsEnrollment ||
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
          error = failure is FormatException
              ? failure.message
              : _message(failure);
          try {
            await _readEnrollment(generation, session);
          } catch (_) {
            /* Retain the application failure. */
          }
        }
      } finally {
        if (_disposed || generation != _generation) await _retireQuietly();
        busy = false;
        enrolling = false;
        _work = null;
        _changed();
      }
    });
    _work = work;
    return work;
  }

  Future<void> prepareEnrollment(
    DiscoveredProfile profile,
    ProfileEnrollmentDevice device,
  ) => _enrollmentTask((generation, session) async {
    if (state?.complete != true ||
        profile.removed ||
        !profile.initialized ||
        profile.waiting > 0 ||
        profile.ready > 0) {
      throw const DiscoveryFailure(
        'Finish discovery and profile setup before importing accounts.',
      );
    }
    final baseline = await device.captureProfilePreferences();
    if (!_current(generation, session)) return;
    final value = await _enrollment(session, {
      'kind': 'prepare',
      'id': _identity(),
      'profile': profile.profile,
      'generation': profile.generation,
      'revision': profile.revision,
      'preferences': baseline.toJson(),
    });
    if (!_current(generation, session)) return;
    enrollment = ProfileEnrollment.fromJson(value as Map<String, dynamic>);
    enrollmentRows = [];
    enrollmentAfter = 0;
    await _continueEnrollment(generation, session, device);
  });
  Future<void> enrollmentPage({required bool first}) => _enrollmentTask(
    (generation, session) => _enrollmentPage(
      generation,
      session,
      first ? 0 : enrollmentRows.lastOrNull?.position ?? 0,
    ),
  );
  Future<void> chooseEnrollment(EnrollmentRow row, bool selected) =>
      _enrollmentTask((generation, session) async {
        final current = enrollment;
        if (current == null) return;
        await _enrollment(session, {
          'kind': 'choose',
          'id': current.id,
          'position': row.position,
          'selected': selected,
        });
        if (_current(generation, session)) {
          await _enrollmentPage(generation, session, enrollmentAfter);
        }
      });
  Future<void> cancelEnrollment() =>
      _enrollmentTask((generation, session) async {
        final current = enrollment;
        if (current == null) return;
        await _enrollment(session, {'kind': 'cancel', 'id': current.id});
        if (!_current(generation, session)) return;
        enrollment = null;
        enrollmentRows = [];
        enrollmentAfter = 0;
        _changed();
      });
  Future<void> approveEnrollment(
    ProfileEnrollmentDevice device, {
    required bool accounts,
    required bool settings,
  }) => _enrollmentTask((generation, session) async {
    final current = enrollment;
    if (current == null) return;
    final value = await _enrollment(session, {
      'kind': 'approve',
      'id': current.id,
      'accounts': accounts,
      'settings': settings,
    });
    if (!_current(generation, session)) return;
    enrollment = ProfileEnrollment.fromJson(value as Map<String, dynamic>);
    await _continueEnrollment(generation, session, device);
  });
  Future<void> resumeEnrollment(ProfileEnrollmentDevice device) async {
    if (busy || !connected || !supportsEnrollment) return;
    await discover();
    if (error != null || busy || enrollment == null) return;
    await _enrollmentTask(
      (generation, session) => _continueEnrollment(generation, session, device),
    );
  }

  Future<void> _continueEnrollment(
    int generation,
    String session,
    ProfileEnrollmentDevice device,
  ) async {
    enrolling = true;
    _changed();
    while (_current(generation, session) &&
        !paused &&
        enrollment != null &&
        !enrollment!.needsReview &&
        !enrollment!.complete) {
      final current = enrollment!;
      dynamic value;
      if (current.phase == 'settings') {
        final request =
            await _enrollment(session, {'kind': 'settings', 'id': current.id})
                as Map<String, dynamic>;
        if (!_current(generation, session) || paused) return;
        if (request['id'] != current.id) {
          throw const DiscoveryFailure(
            'The preference review changed. Reopen this profile.',
          );
        }
        final receipt = await device.applyProfilePreferences(request);
        if (receipt.id != current.id) {
          throw const DiscoveryFailure(
            'The saved preference receipt belongs to another review. Reopen this profile.',
          );
        }
        if (!_current(generation, session)) return;
        value = await _enrollment(session, {
          'kind': 'confirm_settings',
          'id': current.id,
          'applied': receipt.applied,
          'kept': receipt.kept,
        });
      } else {
        value = await _enrollment(session, {'kind': 'step', 'id': current.id});
      }
      if (!_current(generation, session)) return;
      enrollment = ProfileEnrollment.fromJson(value as Map<String, dynamic>);
      if (enrollment!.applied != current.applied || enrollment!.complete) {
        await device.refreshProfileAccounts();
      }
      _changed();
    }
    if (_current(generation, session) && enrollment?.needsReview == true) {
      await _enrollmentPage(generation, session, 0);
    }
    if (_current(generation, session) && enrollment?.complete == true) {
      await device.refreshProfileAccounts();
    }
  }
}
