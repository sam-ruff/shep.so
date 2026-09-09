import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';

enum GoogleCalendarPermission { off, read, edit }

@immutable
class GooglePermissions {
  const GooglePermissions({
    this.drive = false,
    this.calendar = GoogleCalendarPermission.off,
  });
  final bool drive;
  final GoogleCalendarPermission calendar;
  List<String> get scopes => [
    if (drive) 'https://www.googleapis.com/auth/drive.appdata',
    if (calendar != GoogleCalendarPermission.off) ...[
      'https://www.googleapis.com/auth/calendar.events${calendar == GoogleCalendarPermission.read ? '.readonly' : ''}',
      'https://www.googleapis.com/auth/calendar.calendarlist.readonly',
    ],
  ];
  Map<String, Object> toJson() => {'drive': drive, 'calendar': calendar.name};
  factory GooglePermissions.fromJson(Map<String, dynamic> json) {
    if (json['drive'] is! bool || json['calendar'] is! String) {
      throw const FormatException();
    }
    return GooglePermissions(
      drive: json['drive'] as bool,
      calendar: GoogleCalendarPermission.values.byName(
        json['calendar'] as String,
      ),
    );
  }
  @override
  bool operator ==(Object other) =>
      other is GooglePermissions &&
      drive == other.drive &&
      calendar == other.calendar;
  @override
  int get hashCode => Object.hash(drive, calendar);
}

/// Native SDK identity and the services explicitly enabled by this connection.
/// Tokens belong to the SDK, never this record or portable profile metadata.
@immutable
class GoogleConnectionRecord {
  const GoogleConnectionRecord(
    this.subject,
    this.email,
    this.permissions,
    this.application,
  );
  final String subject, email, application;
  final GooglePermissions permissions;
  Map<String, Object> toJson() => {
    'subject': subject,
    'email': email,
    'application': application,
    'permissions': permissions.toJson(),
  };
  factory GoogleConnectionRecord.fromJson(Map<String, dynamic> json) {
    final subject = json['subject'],
        email = json['email'],
        application = json['application'];
    if (subject is! String ||
        subject.isEmpty ||
        subject.length > 255 ||
        email is! String ||
        email.isEmpty ||
        email.length > 320 ||
        application is! String ||
        application.isEmpty ||
        application.length > 255 ||
        RegExp(r'[\x00-\x1f\x7f]').hasMatch('$subject$email$application')) {
      throw const FormatException();
    }
    return GoogleConnectionRecord(
      subject,
      email,
      GooglePermissions.fromJson(json['permissions'] as Map<String, dynamic>),
      application,
    );
  }
}

class GoogleConnectionState {
  const GoogleConnectionState({
    this.requested = const GooglePermissions(),
    this.active,
    this.cleanupPending = false,
  });
  final GooglePermissions requested;
  final GoogleConnectionRecord? active;
  final bool cleanupPending;
  String encode() => jsonEncode({
    'version': 1,
    'requested': requested.toJson(),
    'active': active?.toJson(),
    'cleanup_pending': cleanupPending,
  });
  factory GoogleConnectionState.decode(String? source) {
    if (source == null) return const GoogleConnectionState();
    if (source.length > 16384) throw const FormatException();
    final data = jsonDecode(source) as Map<String, dynamic>;
    if (data['version'] != 1 || data['cleanup_pending'] is! bool) {
      throw const FormatException();
    }
    return GoogleConnectionState(
      requested: GooglePermissions.fromJson(
        data['requested'] as Map<String, dynamic>,
      ),
      active: data['active'] == null
          ? null
          : GoogleConnectionRecord.fromJson(
              data['active'] as Map<String, dynamic>,
            ),
      cleanupPending: data['cleanup_pending'] as bool,
    );
  }
}

abstract interface class GoogleConnectionStore {
  Future<GoogleConnectionState> read();
  Future<void> write(GoogleConnectionState value);
}

/// A platform write lost its reply and the saved value could not be read back.
/// Re-read before permitting another operation; neither success nor rollback is
/// established by this outcome.
class GoogleStorageUnconfirmed implements Exception {
  const GoogleStorageUnconfirmed();
}

class GoogleConnectionFailure implements Exception {
  const GoogleConnectionFailure(this.message);
  final String message;
  @override
  String toString() => message;
}

abstract interface class GoogleAuthorization {
  /// Reauthorize the committed account without signing it out to show a picker.
  Future<GoogleConnectionRecord> connect(
    GooglePermissions requested,
    GoogleConnectionRecord? active,
  );

  /// Local SDK sign-out only: must not revoke other devices' project grants.
  Future<void> signOut();
  Future<String> accessToken(
    GoogleConnectionRecord active,
    List<String> scopes,
  );
}

/// Owns one SDK operation and one coalescing preference writer. No UI await is
/// required for browsing; pending SDK results cannot replace newer choices.
class GoogleConnection extends ChangeNotifier {
  GoogleConnection(this.authorization, this.store);
  final GoogleAuthorization authorization;
  final GoogleConnectionStore store;
  GoogleConnectionState _saved = const GoogleConnectionState();
  GooglePermissions requested = const GooglePermissions();
  GoogleConnectionRecord? get active => _saved.active;
  bool get cleanupPending => _saved.cleanupPending;
  bool get choicesUnsaved => requested != _saved.requested;
  bool loaded = false, busy = false, committing = false, saving = false;
  bool _disposed = false;
  bool _preserveRequestedOnLoad = false;
  int _revision = 0;
  String? error, notice;
  Future<void>? _writer;

  void _changed() {
    if (!_disposed) notifyListeners();
  }

  Future<void> load() async {
    if (busy || loaded || _disposed) return;
    busy = true;
    error = null;
    _changed();
    try {
      final saved = await store.read();
      if (_disposed) return;
      _saved = saved;
      if (!_preserveRequestedOnLoad) requested = saved.requested;
      _preserveRequestedOnLoad = false;
      loaded = true;
      if (choicesUnsaved) unawaited(saveChoices());
    } catch (_) {
      error =
          'Could not read the saved Google connection. Unlock device storage, then retry.';
    } finally {
      busy = false;
      _changed();
    }
  }

  void choose(GooglePermissions value) {
    if (!loaded || committing || _disposed || requested == value) return;
    requested = value;
    _revision++;
    error = null;
    notice = null;
    unawaited(saveChoices());
    _changed();
  }

  Future<void> saveChoices() {
    if (_writer case final Future<void> pending) return pending;
    if (!loaded || committing || _disposed) return Future.value();
    // Defer the runner so the handle is installed even if no write is needed.
    final next = Future<void>.microtask(() async {
      saving = true;
      _changed();
      try {
        while (!_disposed && requested != _saved.requested) {
          final next = GoogleConnectionState(
            requested: requested,
            active: active,
            cleanupPending: cleanupPending,
          );
          await store.write(next);
          _saved = next;
        }
      } on GoogleStorageUnconfirmed {
        _unconfirmedStorage();
      } catch (_) {
        error =
            'Google choices could not be saved. Unlock device storage, then retry saving.';
      } finally {
        saving = false;
        _writer = null;
        _changed();
      }
    });
    _writer = next;
    return next;
  }

  Future<void> connect() async {
    if (!loaded || busy || cleanupPending || _disposed) return;
    busy = true;
    error = null;
    notice = null;
    _changed();
    final revision = _revision, permissions = requested;
    try {
      await saveChoices();
      if (_disposed || revision != _revision || error != null) return;
      final candidate = await authorization.connect(permissions, active);
      if (_disposed) return;
      await saveChoices();
      if (revision != _revision || permissions != requested) {
        throw const GoogleConnectionFailure(
          'Google choices changed during sign-in. Sign in again with the current permissions.',
        );
      }
      if (error != null) return;
      if (candidate.permissions != permissions ||
          (active != null &&
              (candidate.subject != active!.subject ||
                  candidate.application != active!.application))) {
        throw const GoogleConnectionFailure(
          'Google returned a different connection. Your saved connection was kept.',
        );
      }
      // Choices are disabled only for this durable commit, not while Google is
      // awaiting user consent. Navigation remains independent throughout.
      committing = true;
      _changed();
      final next = GoogleConnectionState(
        requested: requested,
        active: candidate,
      );
      GoogleConnectionState.decode(next.encode());
      await store.write(next);
      _saved = next;
      notice = 'Google connection saved on this device.';
    } on GoogleStorageUnconfirmed {
      _unconfirmedStorage();
    } on GoogleConnectionFailure catch (failure) {
      error = failure.message;
    } catch (_) {
      error =
          'Google sign-in could not be saved. Your previous connection was kept. Unlock device storage and retry.';
    } finally {
      busy = false;
      committing = false;
      _changed();
    }
  }

  Future<void> disconnect() async {
    if (!loaded || busy || _disposed) return;
    busy = true;
    error = null;
    notice = null;
    _revision++;
    _changed();
    try {
      await saveChoices();
      if (_disposed || error != null) return;
      committing = true;
      _changed();
      // Persist local disconnection before SDK cleanup. A crash or cleanup
      // failure cannot silently reconnect from the SDK's remembered account.
      final removed = GoogleConnectionState(
        requested: requested,
        cleanupPending: true,
      );
      await store.write(removed);
      _saved = removed;
      _changed();
      await authorization.signOut();
      final cleared = GoogleConnectionState(requested: requested);
      await store.write(cleared);
      _saved = cleared;
      notice = 'Google disconnected on this device. Cached mail was kept.';
    } on GoogleStorageUnconfirmed {
      _unconfirmedStorage();
    } catch (_) {
      error = cleanupPending
          ? 'Google is disconnected here, but device sign-out needs cleanup. Retry cleanup.'
          : 'Could not save Google disconnection. Unlock device storage, then retry.';
    } finally {
      busy = false;
      committing = false;
      _changed();
    }
  }

  /// Provider callers obtain a token only for enabled services and this exact
  /// committed identity. Silent token refresh never opens consent UI.
  Future<String> accessToken(List<String> scopes) async {
    final expected = active;
    if (!loaded ||
        expected == null ||
        cleanupPending ||
        scopes.isEmpty ||
        !scopes.every(expected.permissions.scopes.contains)) {
      throw const GoogleConnectionFailure(
        'Connect Google and enable this service in Preferences.',
      );
    }
    if (busy || _disposed) {
      throw const GoogleConnectionFailure(
        'Google is busy with another request. Try again shortly.',
      );
    }
    busy = true;
    _changed();
    try {
      final token = await authorization.accessToken(expected, scopes);
      if (_disposed || !identical(active, expected) || cleanupPending) {
        throw const GoogleConnectionFailure(
          'Google changed while this request was starting. Try again.',
        );
      }
      return token;
    } finally {
      busy = false;
      _changed();
    }
  }

  void _unconfirmedStorage() {
    loaded = false;
    _preserveRequestedOnLoad = true;
    error =
        'Could not confirm the saved Google connection. Unlock device storage, then retry reading it before continuing.';
  }

  @override
  void dispose() {
    _disposed = true;
    _revision++;
    super.dispose();
  }
}
