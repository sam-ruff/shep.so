import 'package:shep_mobile/data/native_repository.dart';

/// Object-scoped connection/protocol failure injection. All cache, journal and
/// credential work still uses NativeRepository and the real Rust bridge.
class ConnectionFixtureRepository extends NativeRepository {
  ConnectionFixtureRepository(super.profile, super.credentials);
  bool refuseActivation = false,
      loseActivationResponse = false,
      captureSend = false;
  Map<String, Object?>? submitted;
  @override
  Future<dynamic> call(Map<String, Object?> request) async {
    if (request['op'] == 'send' && captureSend) {
      submitted = request;
      throw StateError('Synthetic SMTP refusal');
    }
    if (request['op'] == 'probe') return {'connected': true, 'sent': false};
    if (request['op'] == 'activate_account' && refuseActivation) {
      throw StateError(
        'Synthetic activation failure; previous connection kept.',
      );
    }
    final result = await super.call(request);
    if (request['op'] == 'activate_account' && loseActivationResponse) {
      throw StateError('Synthetic lost activation response.');
    }
    return result;
  }
}
