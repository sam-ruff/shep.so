import 'dart:async';
import 'dart:math';
import 'package:flutter/foundation.dart';
import '../data/profile_discovery.dart';
import '../data/profile_creation.dart';
import 'google_connection.dart';
part 'profile_creation.dart';

class ProfileDiscovery extends ChangeNotifier {
  ProfileDiscovery(this.google, this.repository, {required this.namespace}) {
    _grant = google.grantGeneration;
    google.addListener(_googleChanged);
  }
  final GoogleConnection google;
  final ProfileDiscoveryRepository repository;
  final String namespace;
  DiscoveryState? state;
  ProfileCreation? creation;
  List<ProfileAccountReview> creationAccounts = [];
  int creationAfter = 0;
  bool publishing = false;
  List<DiscoveredProfile> profiles = [];
  String? error, after;
  bool busy = false, paused = false;
  bool _disposed = false;
  int _generation = 0, _grant = 0;
  String? _session;
  Future<void>? _work;
  Future<void> _retiring = Future.value();
  bool get connected =>
      google.loaded &&
      !google.cleanupPending &&
      google.active?.permissions.drive == true;
  bool get configured => namespace.isNotEmpty;
  bool get canPage => !busy && _session != null;
  void _changed() {
    if (!_disposed) notifyListeners();
  }

  void _googleChanged() {
    if (_grant != google.grantGeneration || !connected) {
      _grant = google.grantGeneration;
      _generation++;
      paused = true;
      state = null;
      creation = null;
      creationAccounts = [];
      creationAfter = 0;
      profiles = [];
      after = null;
      error = null;
      if (!busy) unawaited(_retireQuietly());
    }
    _changed();
  }

  bool _current(int generation, String session) =>
      !_disposed &&
      generation == _generation &&
      connected &&
      _session == session;
  String _identity() {
    final random = Random.secure();
    final bytes = List.generate(16, (_) => random.nextInt(256));
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    final hex = bytes.map((b) => b.toRadixString(16).padLeft(2, '0')).join();
    return '${hex.substring(0, 8)}-${hex.substring(8, 12)}-${hex.substring(12, 16)}-${hex.substring(16, 20)}-${hex.substring(20)}';
  }

  Future<void> _retire() {
    final next = _retiring.then((_) async {
      final session = _session;
      if (session == null) return;
      await repository.close(session);
      if (_session == session) _session = null;
    });
    _retiring = next.then<void>((_) {}, onError: (Object _, StackTrace _) {});
    return next;
  }

  Future<void> _retireQuietly() async {
    try {
      await _retire();
    } catch (_) {
      if (!_disposed) {
        error =
            'Could not finish closing profile discovery. Retry discovery before continuing.';
      }
      _changed();
    }
  }

  String _message(Object failure) => switch (failure) {
    GoogleConnectionFailure(:final message) => message,
    DiscoveryFailure(:final message) => message,
    _ => 'Could not discover saved profiles. Check your connection and retry.',
  };

  /// The task stays owned through native replies and session cleanup even if the
  /// screen closes. One step at a time, with no next GET after pause/disconnect.
  Future<void> discover({bool full = false}) {
    if (_work case final Future<void> pending) return pending;
    if (_disposed || busy || !connected || !configured) return Future.value();
    busy = true;
    paused = false;
    error = null;
    after = null;
    final generation = _generation, grant = google.grantGeneration;
    _changed();
    final next = Future<void>.microtask(() async {
      String? id;
      var bound = false;
      try {
        await _retire();
        if (_disposed || generation != _generation || !connected) return;
        final token = await google.accessToken(const [
          'https://www.googleapis.com/auth/drive.appdata',
        ]);
        if (_disposed || generation != _generation || !connected) return;
        id = _identity();
        _session = id;
        final opened = await repository.open(
          id,
          token,
          namespace,
          google.active!.drivePrincipal,
        );
        if (!_current(generation, id)) return;
        if (opened.id != id || opened.namespace != namespace) {
          throw const DiscoveryFailure(
            'Profile discovery returned a different session. Reconnect Google and retry.',
          );
        }
        await google.bindDrivePrincipal(grant, opened.principal);
        if (!_current(generation, id)) return;
        bound = true;
        state = opened.state;
        await _readCreation(generation, id);
        if (!_current(generation, id)) return;
        if (creation?.needsReview == true) {
          await _creationPage(generation, id, 0);
        }
        await _page(generation, id, null);
        if (!_current(generation, id)) return;
        DiscoveryState? refreshed;
        if (full || state!.complete) {
          refreshed = await repository.refresh(id, state!.revision, full: full);
        } else if (state!.error != null) {
          refreshed = await repository.retry(id, state!.revision);
        }
        if (!_current(generation, id)) return;
        if (refreshed != null) state = refreshed;
        _changed();
        while (_current(generation, id) && !paused && !state!.complete) {
          final next = await repository.advance(id);
          if (!_current(generation, id)) return;
          state = next;
          _changed();
        }
        if (_current(generation, id)) await _page(generation, id, null);
      } catch (failure) {
        if (!_disposed && generation == _generation) {
          error = _message(failure);
          if (bound && id != null && _session == id) {
            try {
              final observed = await repository.state(id);
              if (_current(generation, id)) {
                state = observed;
                await _page(generation, id, null);
              }
            } catch (_) {
              /* Keep the original failure and earlier observations. */
            }
          }
        }
      } finally {
        if (_disposed || generation != _generation) await _retireQuietly();
        busy = false;
        _work = null;
        _changed();
      }
    });
    _work = next;
    return next;
  }

  void pause() {
    if (busy) {
      paused = true;
      _changed();
    }
  }

  Future<void> _page(int generation, String session, String? cursor) async {
    final rows = await repository.profiles(session, cursor);
    if (!_current(generation, session)) return;
    if (rows.length > 50) {
      throw const DiscoveryFailure(
        'Too many profiles were returned. Update Shep and retry.',
      );
    }
    profiles = rows;
    after = cursor;
    _changed();
  }

  Future<void> page({required bool first}) async {
    if (!canPage || _disposed || !connected) return;
    final id = _session!, generation = _generation;
    final cursor = first ? null : profiles.lastOrNull?.cursor;
    if (!first && cursor == null) return;
    busy = true;
    error = null;
    _changed();
    try {
      await _page(generation, id, cursor);
    } catch (failure) {
      if (_current(generation, id)) error = _message(failure);
    } finally {
      if (_disposed || generation != _generation) await _retireQuietly();
      busy = false;
      _changed();
    }
  }

  @override
  void dispose() {
    google.removeListener(_googleChanged);
    _disposed = true;
    _generation++;
    paused = true;
    if (!busy) unawaited(_retireQuietly());
    super.dispose();
  }
}
