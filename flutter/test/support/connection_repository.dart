import 'dart:async';
import 'package:shep_mobile/data/native_repository.dart';

/// Object-scoped connection/protocol failure injection. All cache, journal and
/// credential work still uses NativeRepository and the real Rust bridge.
class ConnectionFixtureRepository extends NativeRepository {
  ConnectionFixtureRepository(super.profile, super.credentials);
  bool refuseActivation = false,
      loseActivationResponse = false,
      losePrepareResponse = false,
      captureSend = false;
  Completer<void>? probeStarted, probeRelease;
  int probes = 0;
  bool forwardProbes = false;
  Map<String, Object?>? submitted;
  final sendCaptured = Completer<void>();
  final sendRelease = Completer<void>();
  final sendSettled = Completer<void>();
  @override
  Future<dynamic> call(Map<String, Object?> request) async {
    if (request['op'] == 'send' && captureSend) {
      submitted = request;
      sendCaptured.complete();
      await sendRelease.future;
      throw StateError('Synthetic SMTP refusal');
    }
    if (request['op'] == 'probe_account_connection') {
      probes++;
      if (probeStarted case final started? when !started.isCompleted) {
        started.complete();
      }
      if (probeRelease != null) await probeRelease!.future;
      if (forwardProbes) return super.call(request);
      return {'connected': true, 'sent': false};
    }
    if (request['op'] == 'activate_account' && refuseActivation) {
      throw StateError(
        'Synthetic activation failure; previous connection kept.',
      );
    }
    final result = await super.call(request);
    if (request['op'] == 'wait_outgoing' &&
        captureSend &&
        !sendSettled.isCompleted) {
      sendSettled.complete();
    }
    if (request['op'] == 'prepare_account' && losePrepareResponse) {
      throw StateError('Synthetic lost preparation response.');
    }
    if (request['op'] == 'activate_account' && loseActivationResponse) {
      throw StateError('Synthetic lost activation response.');
    }
    return result;
  }
}
