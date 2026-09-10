import 'dart:async';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Fictional connection metadata only, separate from the production secure key.
class PreviewGoogleStore implements GoogleConnectionStore {
  final _preferences = SharedPreferencesAsync();
  static const key = 'shep.preview.google.v1';
  @override
  Future<GoogleConnectionState> read() async =>
      GoogleConnectionState.decode(await _preferences.getString(key));
  @override
  Future<void> write(GoogleConnectionState value) =>
      _preferences.setString(key, value.encode());
}

class MemoryGoogleStore implements GoogleConnectionStore {
  MemoryGoogleStore([this.value = const GoogleConnectionState()]);
  GoogleConnectionState value;
  bool failRead = false, failWrite = false;
  bool unconfirmedWrite = false;
  int writes = 0, activeWrites = 0, maxWrites = 0;
  Completer<void>? holdWrite;
  @override
  Future<GoogleConnectionState> read() async {
    if (failRead) throw StateError('fixture read refused');
    return GoogleConnectionState.decode(value.encode());
  }

  @override
  Future<void> write(GoogleConnectionState next) async {
    writes++;
    activeWrites++;
    if (activeWrites > maxWrites) maxWrites = activeWrites;
    try {
      await holdWrite?.future;
      if (failWrite) throw StateError('fixture write refused');
      value = GoogleConnectionState.decode(next.encode());
      if (unconfirmedWrite) throw const GoogleStorageUnconfirmed();
    } finally {
      activeWrites--;
    }
  }
}

class FixtureGoogleAuthorization implements GoogleAuthorization {
  static const subject = 'fixture-google-user';
  static const email = 'alex@example.test';
  static const application = 'fixture-shep-application';
  int connects = 0, signOuts = 0, tokens = 0;
  bool failSignOut = false, failEditingOnce = false;
  String? failure;
  String returnedSubject = subject, returnedApplication = application;
  Completer<void>? hold;
  Duration delay = Duration.zero;
  @override
  Future<GoogleConnectionRecord> connect(
    GooglePermissions requested,
    GoogleConnectionRecord? active,
  ) async {
    connects++;
    await hold?.future;
    if (delay != Duration.zero) await Future<void>.delayed(delay);
    if (failure case final String message) {
      throw GoogleConnectionFailure(message);
    }
    if (failEditingOnce &&
        requested.calendar == GoogleCalendarPermission.edit) {
      failEditingOnce = false;
      throw const GoogleConnectionFailure(
        'Google sign-in was cancelled. Your saved connection was kept.',
      );
    }
    return GoogleConnectionRecord(
      returnedSubject,
      email,
      requested,
      returnedApplication,
    );
  }

  @override
  Future<void> signOut() async {
    signOuts++;
    if (failSignOut) throw StateError('fixture sign-out refused');
  }

  @override
  Future<String> accessToken(
    GoogleConnectionRecord active,
    List<String> scopes,
  ) async {
    tokens++;
    await hold?.future;
    return 'fixture-access-token';
  }
}
