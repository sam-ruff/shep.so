import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:google_sign_in_platform_interface/google_sign_in_platform_interface.dart';
import 'package:shep_mobile/data/google_native.dart';
import 'package:shep_mobile/model/google_connection.dart';

class GooglePlatformFixture extends GoogleSignInPlatform {
  int initializations = 0, authentications = 0, signOuts = 0, revocations = 0;
  AuthenticationResults? restored;
  bool silentDenied = false, cancelConsent = false;
  final requests = <AuthorizationRequestDetails>[];
  InitParameters? initialized;
  static const identity = AuthenticationResults(
    user: GoogleSignInUserData(
      email: 'alex@example.test',
      id: 'verified-sdk-subject',
    ),
    authenticationTokens: AuthenticationTokenData(
      idToken: 'fixture-id-token-never-stored',
    ),
  );
  @override
  Future<void> init(InitParameters params) async {
    initializations++;
    initialized = params;
  }

  @override
  Future<AuthenticationResults?> attemptLightweightAuthentication(
    AttemptLightweightAuthenticationParameters params,
  ) async {
    if (restored == null) {
      throw const GoogleSignInException(
        code: GoogleSignInExceptionCode.canceled,
      );
    }
    return restored;
  }

  @override
  bool supportsAuthenticate() => true;
  @override
  Future<AuthenticationResults> authenticate(
    AuthenticateParameters params,
  ) async {
    authentications++;
    return restored = identity;
  }

  @override
  bool authorizationRequiresUserInteraction() => false;
  @override
  Future<ClientAuthorizationTokenData?> clientAuthorizationTokensForScopes(
    ClientAuthorizationTokensForScopesParameters params,
  ) async {
    requests.add(params.request);
    if (params.request.promptIfUnauthorized && cancelConsent) {
      throw const GoogleSignInException(
        code: GoogleSignInExceptionCode.canceled,
        description: 'sensitive fixture diagnostic',
      );
    }
    if (!params.request.promptIfUnauthorized && silentDenied) return null;
    return const ClientAuthorizationTokenData(
      accessToken: 'fixture-provider-access-token',
    );
  }

  @override
  Future<ServerAuthorizationTokenData?> serverAuthorizationTokensForScopes(
    ServerAuthorizationTokensForScopesParameters params,
  ) => throw StateError('No server token exchange is allowed');
  @override
  Future<void> signOut(SignOutParams params) async {
    signOuts++;
    restored = null;
  }

  @override
  Future<void> disconnect(DisconnectParams params) async {
    revocations++;
    throw StateError('No project-wide revocation');
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'production adapter binds SDK identity, exact consent and local-only sign-out',
    () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final previous = GoogleSignInPlatform.instance;
      final platform = GooglePlatformFixture();
      GoogleSignInPlatform.instance = platform;
      addTearDown(() => GoogleSignInPlatform.instance = previous);
      final sdk = NativeGoogleAuthorization();
      const configured = String.fromEnvironment('SHEP_GOOGLE_SERVER_CLIENT_ID');
      if (configured.isEmpty) {
        await expectLater(
          sdk.connect(const GooglePermissions(), null),
          throwsA(isA<GoogleConnectionFailure>()),
        );
        expect(platform.initializations, 0);
        return;
      }
      const read = GooglePermissions(calendar: GoogleCalendarPermission.read);
      final grant = await sdk.connect(read, null);
      expect(platform.initialized!.serverClientId, configured);
      expect(platform.initialized!.clientId, isNull);
      expect(grant.subject, 'verified-sdk-subject');
      expect(grant.application, configured);
      expect(platform.authentications, 1);
      expect(platform.signOuts, 0);
      expect(platform.requests.single.scopes, read.scopes);
      expect(platform.requests.single.userId, grant.subject);
      expect(platform.requests.single.email, grant.email);
      expect(grant.toJson().toString(), isNot(contains('token')));
      platform.silentDenied = true;
      platform.cancelConsent = true;
      await expectLater(
        sdk.connect(const GooglePermissions(drive: true), grant),
        throwsA(
          isA<GoogleConnectionFailure>().having(
            (e) => e.message,
            'sanitized cancellation',
            allOf(contains('cancelled'), isNot(contains('sensitive'))),
          ),
        ),
      );
      expect(platform.authentications, 1);
      expect(platform.signOuts, 0);
      expect(platform.requests.last.promptIfUnauthorized, true);
      final before = platform.requests.length;
      await expectLater(
        sdk.accessToken(grant, read.scopes),
        throwsA(isA<GoogleConnectionFailure>()),
      );
      expect(platform.requests.length, before + 1);
      expect(platform.requests.last.promptIfUnauthorized, false);
      platform.silentDenied = false;
      expect(
        await sdk.accessToken(grant, read.scopes),
        'fixture-provider-access-token',
      );
      await expectLater(
        sdk.connect(
          read,
          GoogleConnectionRecord(
            grant.subject,
            grant.email,
            read,
            'another-project',
          ),
        ),
        throwsA(isA<GoogleConnectionFailure>()),
      );
      final reopened = NativeGoogleAuthorization();
      await expectLater(
        reopened.accessToken(grant, read.scopes),
        throwsA(isA<GoogleConnectionFailure>()),
      );
      await reopened.connect(read, grant);
      expect(platform.initializations, 1);
      expect(platform.authentications, 1);
      await reopened.signOut();
      expect(platform.signOuts, 1);
      expect(platform.revocations, 0);
    },
  );
}
