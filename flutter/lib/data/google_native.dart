import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:google_sign_in_platform_interface/google_sign_in_platform_interface.dart';
import '../model/google_connection.dart';

class DeviceGoogleConnectionStore implements GoogleConnectionStore {
  static const _storage = FlutterSecureStorage(
    iOptions: IOSOptions(
      accessibility: KeychainAccessibility.first_unlock_this_device,
    ),
  );
  static Future<void> _writes = Future.value();
  static const _key = 'so.shep.google.connection.v1';
  @override
  Future<GoogleConnectionState> read() async {
    await _writes;
    return GoogleConnectionState.decode(await _storage.read(key: _key));
  }

  @override
  Future<void> write(GoogleConnectionState value) {
    final encoded = value.encode();
    final next = _writes.then((_) async {
      try {
        await _storage.write(key: _key, value: encoded);
      } catch (_) {
        // An acknowledged value may survive a lost plugin reply. Reconcile it
        // without initiating another authorization or overwriting old state.
        String? observed;
        try {
          observed = await _storage.read(key: _key);
        } catch (_) {
          throw const GoogleStorageUnconfirmed();
        }
        if (observed != encoded) rethrow;
      }
    });
    _writes = next.then<void>((_) {}, onError: (Object _, StackTrace _) {});
    return next;
  }
}

class NativeGoogleAuthorization implements GoogleAuthorization {
  static const _application = String.fromEnvironment(
    'SHEP_GOOGLE_SERVER_CLIENT_ID',
  );
  static Future<void>? _initialization;
  GoogleSignInUserData? _user;
  GoogleSignInPlatform get _sdk => GoogleSignInPlatform.instance;
  Future<void> _initialize() async {
    if (kIsWeb ||
        ![
          TargetPlatform.android,
          TargetPlatform.iOS,
        ].contains(defaultTargetPlatform)) {
      throw const GoogleConnectionFailure(
        'Google sign-in is available in the Android and iOS clients.',
      );
    }
    const server = String.fromEnvironment('SHEP_GOOGLE_SERVER_CLIENT_ID');
    const ios = String.fromEnvironment('SHEP_GOOGLE_IOS_CLIENT_ID');
    if (server.isEmpty ||
        (defaultTargetPlatform == TargetPlatform.iOS && ios.isEmpty)) {
      throw const GoogleConnectionFailure(
        'Google sign-in is not configured in this build. Install a build configured for Google sign-in.',
      );
    }
    await (_initialization ??= _sdk.init(
      InitParameters(
        serverClientId: server,
        clientId: defaultTargetPlatform == TargetPlatform.iOS ? ios : null,
      ),
    ));
  }

  Future<GoogleSignInUserData?> _restore() async {
    if (_user != null) return _user;
    try {
      final result = await _sdk.attemptLightweightAuthentication(
        const AttemptLightweightAuthenticationParameters(),
      );
      return _user = result?.user;
    } on GoogleSignInException catch (failure) {
      if ([
        GoogleSignInExceptionCode.canceled,
        GoogleSignInExceptionCode.interrupted,
        GoogleSignInExceptionCode.uiUnavailable,
      ].contains(failure.code)) {
        return null;
      }
      rethrow;
    }
  }

  Future<ClientAuthorizationTokenData?> _authorize(
    GoogleSignInUserData user,
    List<String> scopes, {
    required bool prompt,
  }) => _sdk.clientAuthorizationTokensForScopes(
    ClientAuthorizationTokensForScopesParameters(
      request: AuthorizationRequestDetails(
        scopes: scopes,
        userId: user.id,
        email: user.email,
        promptIfUnauthorized: prompt,
      ),
    ),
  );

  @override
  Future<GoogleConnectionRecord> connect(
    GooglePermissions requested,
    GoogleConnectionRecord? active,
  ) async {
    try {
      await _initialize();
      if (active != null && active.application != _application) {
        throw const GoogleConnectionFailure(
          'This build uses a different Google application. Your saved connection was kept. Disconnect here before setting up the new application.',
        );
      }
      var user = await _restore();
      if (active != null && user != null && user.id != active.subject) {
        throw const GoogleConnectionFailure(
          'The device Google account differs from the saved connection. Disconnect here before choosing another account.',
        );
      }
      // Never sign out the committed account simply to display an account picker.
      // Additional consent is requested against the same SDK identity.
      user ??= (await _sdk.authenticate(
        AuthenticateParameters(scopeHint: requested.scopes),
      )).user;
      _user = user;
      if (active != null && user.id != active.subject) {
        throw const GoogleConnectionFailure(
          'A different Google account was selected. The saved connection was kept. Reconnect its account or disconnect here first.',
        );
      }
      if (requested.scopes.isNotEmpty) {
        final token =
            await _authorize(user, requested.scopes, prompt: false) ??
            await _authorize(user, requested.scopes, prompt: true);
        if (token == null) {
          throw const GoogleConnectionFailure(
            'Google did not approve the requested access. Try signing in again.',
          );
        }
      }
      return GoogleConnectionRecord(
        user.id,
        user.email,
        requested,
        _application,
      );
    } on GoogleSignInException catch (failure) {
      throw _friendly(failure);
    }
  }

  @override
  Future<void> signOut() async {
    // No initialization means this process may still have a remembered native
    // connection from an earlier run; initialize before actually clearing it.
    await _initialize();
    await _sdk.signOut(const SignOutParams());
    _user = null;
  }

  @override
  Future<String> accessToken(
    GoogleConnectionRecord active,
    List<String> scopes,
  ) async {
    try {
      await _initialize();
      if (active.application != _application) {
        throw const GoogleConnectionFailure(
          'This build uses a different Google application. Reconnect in Preferences.',
        );
      }
      // Lightweight authentication may display account UI. Restore through
      // explicit Connect; background work only refreshes an existing session.
      final user = _user;
      if (user == null || user.id != active.subject) {
        throw const GoogleConnectionFailure(
          'Reconnect the saved Google account in Preferences.',
        );
      }
      final token = await _authorize(user, scopes, prompt: false);
      if (token == null) {
        throw const GoogleConnectionFailure(
          'Google access needs approval. Reconnect in Preferences.',
        );
      }
      return token.accessToken;
    } on GoogleSignInException catch (failure) {
      throw _friendly(failure);
    }
  }

  GoogleConnectionFailure _friendly(
    GoogleSignInException failure,
  ) => GoogleConnectionFailure(switch (failure.code) {
    GoogleSignInExceptionCode.canceled =>
      'Google sign-in was cancelled. Your saved connection was kept.',
    GoogleSignInExceptionCode.clientConfigurationError =>
      'Google sign-in is not configured correctly in this build. Install a configured build and retry.',
    _ =>
      'Google could not complete sign-in. Check connectivity and try again. Your saved connection was kept.',
  });
}
