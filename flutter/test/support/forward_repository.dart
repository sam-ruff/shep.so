import 'dart:async';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/model/mail.dart';

// Controlled failures wrap the actual FFI/cache operation. Never use personal
// profiles or credentials with this test-only repository.
class ForwardFixtureRepository extends NativeRepository {
  ForwardFixtureRepository(super.profile, super.credentials);
  bool loseAcknowledgment = false;
  Completer<void>? release, started;
  @override
  Future<Draft> forward(String id, String draftId) async {
    started?.complete();
    if (release != null) await release!.future;
    final draft = await super.forward(id, draftId);
    if (loseAcknowledgment) {
      loseAcknowledgment = false;
      throw StateError('Synthetic lost forward acknowledgment. Retry Forward.');
    }
    return draft;
  }
}
