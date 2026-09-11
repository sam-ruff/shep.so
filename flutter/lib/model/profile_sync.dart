part of 'profile_discovery.dart';

/// Ongoing preference reconciliation. Cycles run on the foreground check and on
/// demand, one owned task at a time, through the same verified Drive session
/// as discovery. Device writes go through the enrollment application path so
/// one platform receipt serialises enrollment and sync.
extension ProfileSyncActions on ProfileDiscovery {
  bool get supportsSync => repository is ProfileSyncRepository;
  bool get canSubscribe =>
      supportsSync &&
      connected &&
      !busy &&
      syncChecked &&
      sync == null &&
      enrollment?.complete == true;
  Future<dynamic> _sync(String session, Map<String, Object?> command) =>
      (repository as ProfileSyncRepository).sync(session, command);

  Future<void> _readSync(int generation, String session) async {
    if (!supportsSync) return;
    final value = await _sync(session, {'kind': 'current'});
    if (!_current(generation, session)) return;
    sync = value == null
        ? null
        : ProfileSyncStatus.fromJson(value as Map<String, dynamic>);
    syncChecked = true;
    _changed();
  }

  Future<void> _readReviews(int generation, String session) async {
    if (sync == null) {
      syncReviews = [];
      return;
    }
    final rows = await _sync(session, {'kind': 'reviews'}) as List;
    if (!_current(generation, session)) return;
    if (rows.length > 50) {
      throw const DiscoveryFailure(
        'Too many preference reviews were returned. Update Shep and retry.',
      );
    }
    syncReviews = rows
        .map((r) => ProfileSyncReview.fromJson(r as Map<String, dynamic>))
        .toList();
    if (syncReview case final open?) {
      syncReview = syncReviews.where((r) => r.id == open.id).firstOrNull;
      if (syncReview == null) {
        syncVersions = [];
        syncSeen = 0;
      }
    }
    _changed();
  }

  Future<void> _syncTask(
    Future<void> Function(int, String) action, {
    bool quiet = false,
  }) {
    if (_work case final Future<void> pending) return pending;
    if (_disposed || busy || !connected || !configured || !supportsSync) {
      return Future.value();
    }
    final generation = _generation, grant = google.grantGeneration;
    busy = true;
    syncing = true;
    if (!quiet) syncError = null;
    _changed();
    final work = Future<void>.microtask(() async {
      try {
        var session = _session;
        if (session == null) {
          session = await _open(generation, grant);
          if (session == null) return;
          // Enrollment state is session-scoped; sync seeds from its receipt.
          await _readEnrollment(generation, session);
        }
        if (!_current(generation, session)) return;
        await action(generation, session);
      } catch (failure) {
        if (!_disposed && generation == _generation) {
          syncError = failure is FormatException
              ? failure.message
              : _message(failure);
        }
      } finally {
        if (_disposed || generation != _generation) await _retireQuietly();
        busy = false;
        syncing = false;
        _work = null;
        _changed();
      }
    });
    _work = work;
    return work;
  }

  /// Read the durable subscription and its open reviews without a cycle.
  Future<void> refreshSync() => _syncTask((generation, session) async {
    await _readSync(generation, session);
    if (_current(generation, session)) {
      await _readReviews(generation, session);
    }
  });

  Future<void> _subscribe(
    int generation,
    String session,
    ProfileEnrollmentDevice device,
  ) async {
    final current = enrollment;
    if (current == null || !current.complete) return;
    final snapshot = await device.captureProfilePreferences();
    if (!_current(generation, session)) return;
    final value = await _sync(session, {
      'kind': 'subscribe',
      'enrollment': current.id,
      'snapshot': snapshot.toJson(),
    });
    if (!_current(generation, session)) return;
    sync = ProfileSyncStatus.fromJson(value as Map<String, dynamic>);
    syncChecked = true;
    _changed();
  }

  /// Seed sync from the completed enrollment. It starts paused: the master
  /// switch in Preferences turns cycles on.
  Future<void> subscribeSync(ProfileEnrollmentDevice device) => _syncTask(
    (generation, session) => _subscribe(generation, session, device),
  );

  Future<void> configureSync({bool? enabled, String? field, bool? selected}) =>
      _syncTask((generation, session) async {
        final current = sync;
        if (current == null) return;
        final value = await _sync(session, {
          'kind': 'configure',
          'expected_revision': current.revision,
          'enabled': enabled,
          'field': field,
          'selected': selected,
        });
        if (!_current(generation, session)) return;
        sync = ProfileSyncStatus.fromJson(value as Map<String, dynamic>);
        _changed();
      });

  Future<void> _applyPending(
    int generation,
    String session,
    ProfileEnrollmentDevice device,
  ) async {
    final request = await _sync(session, {'kind': 'application'});
    if (!_current(generation, session) || request == null) return;
    final receipt = await device.applyProfilePreferences(
      request as Map<String, dynamic>,
    );
    if (receipt.id != request['id'] || receipt.revisions.isEmpty) {
      throw const DiscoveryFailure(
        'The saved preference receipt belongs to another application. Sync again.',
      );
    }
    if (!_current(generation, session)) return;
    final value = await _sync(session, {
      'kind': 'confirm_application',
      'id': receipt.id,
      'applied': receipt.applied,
      'kept': receipt.kept,
      'revisions': receipt.revisions,
    });
    if (!_current(generation, session)) return;
    sync = ProfileSyncStatus.fromJson(value as Map<String, dynamic>);
    _changed();
  }

  Future<void> _cycle(
    int generation,
    String session,
    ProfileEnrollmentDevice device,
  ) async {
    syncApplied = 0;
    syncPublished = 0;
    syncRan = true;
    // A lost device receipt is retried before any new remote application.
    await _applyPending(generation, session, device);
    for (var i = 0; i < 40 && _current(generation, session); i++) {
      final snapshot = await device.captureProfilePreferences();
      if (!_current(generation, session)) return;
      final value = await _sync(session, {
        'kind': 'cycle',
        'snapshot': snapshot.toJson(),
      });
      if (!_current(generation, session)) return;
      final status = ProfileSyncStatus.fromJson(value as Map<String, dynamic>);
      sync = status;
      if (status.last case final last?) {
        syncApplied += last.applied;
        syncPublished += last.published;
      }
      _changed();
      if (status.applications > 0) {
        await _applyPending(generation, session, device);
      }
      if (status.last?.remaining != true && status.applications == 0) break;
    }
    if (_current(generation, session)) {
      await _readReviews(generation, session);
    }
  }

  Future<void> syncNow(ProfileEnrollmentDevice device) =>
      _syncTask((generation, session) async {
        if (!syncChecked) await _readSync(generation, session);
        if (!_current(generation, session) || sync == null) return;
        await _cycle(generation, session, device);
      });

  /// Foreground check: silent, bounded, and skipped while any profile work,
  /// unsaved preferences or a missing grant would make it unsafe.
  Future<void> syncTick(ProfileEnrollmentDevice? device) {
    if (device == null ||
        _work != null ||
        _disposed ||
        busy ||
        !connected ||
        !configured ||
        !supportsSync ||
        (syncChecked && sync?.enabled != true)) {
      return Future.value();
    }
    return _syncTask(quiet: true, (generation, session) async {
      if (!syncChecked) await _readSync(generation, session);
      if (!_current(generation, session) || sync?.enabled != true) return;
      await _cycle(generation, session, device);
    });
  }

  Future<void> openSyncReview(ProfileSyncReview review) =>
      _syncTask((generation, session) async {
        syncReview = review;
        syncVersions = [];
        syncSeen = 0;
        _syncOffset = 0;
        _changed();
        await _versionsPage(generation, session, review, null);
      });

  Future<void> _versionsPage(
    int generation,
    String session,
    ProfileSyncReview review,
    String? after,
  ) async {
    final rows =
        await _sync(session, {
              'kind': 'review_versions',
              'id': review.id,
              'after': after,
            })
            as List;
    if (!_current(generation, session) || syncReview?.id != review.id) return;
    if (rows.length > 50) {
      throw const DiscoveryFailure(
        'Too many preference versions were returned. Update Shep and retry.',
      );
    }
    _syncOffset = after == null ? 0 : _syncOffset + syncVersions.length;
    syncVersions = rows
        .map((r) => ProfileSyncVersion.fromJson(r as Map<String, dynamic>))
        .toList();
    syncSeen = max(syncSeen, _syncOffset + syncVersions.length);
    _changed();
  }

  Future<void> syncVersionsPage({required bool first}) =>
      _syncTask((generation, session) async {
        final review = syncReview;
        if (review == null) return;
        await _versionsPage(
          generation,
          session,
          review,
          first ? null : syncVersions.lastOrNull?.operation,
        );
      });

  /// Keep mine (shared == null) or use one exact shared version. The device
  /// snapshot proves the local intent is still the reviewed one.
  Future<void> decideSync(
    ProfileSyncReview review,
    ProfileEnrollmentDevice device, {
    String? shared,
  }) => _syncTask((generation, session) async {
    final snapshot = await device.captureProfilePreferences();
    if (!_current(generation, session)) return;
    final value = await _sync(session, {
      'kind': 'decide',
      'id': review.id,
      'choice': shared == null
          ? {'kind': 'local'}
          : {'kind': 'shared', 'operation': shared},
      'seen': syncSeen,
      'snapshot': snapshot.toJson(),
    });
    if (!_current(generation, session)) return;
    sync = ProfileSyncStatus.fromJson(value as Map<String, dynamic>);
    syncReview = null;
    syncVersions = [];
    syncSeen = 0;
    _changed();
    if (sync!.applications > 0) {
      await _applyPending(generation, session, device);
    }
    if (_current(generation, session)) {
      await _readReviews(generation, session);
    }
  });

  /// Leave the open review. Unmounting screens must not notify during dispose.
  void closeSyncReview({bool notify = true}) {
    syncReview = null;
    syncVersions = [];
    syncSeen = 0;
    if (notify) _changed();
  }
}
