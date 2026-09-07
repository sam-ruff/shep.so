import 'dart:convert';
import 'package:shep_mobile/data/formatted_message.dart';
import 'formatted_fixture.dart';
import 'preview_repository.dart';

class FormattedPreviewRepository extends PreviewRepository
    implements FormattedMessageRepository {
  FormattedPreviewRepository({this.failOnce = false})
    : super(
        firstBody: (jsonDecode(formattedFixtureJson) as Map)['text'] as String,
      );
  bool failOnce;
  @override
  Future<PreparedMessage> formattedMessage(
    String id, {
    required String generation,
    required bool dark,
    required bool quotes,
  }) async {
    await Future<void>.delayed(const Duration(milliseconds: 150));
    if (failOnce) {
      failOnce = false;
      throw StateError('Synthetic first preparation failure');
    }
    if (id != '1') {
      return PreparedMessage(
        signature: id,
        text: cached.firstWhere((m) => m.id == id).body,
      );
    }
    return PreparedMessage.fromJson(
      jsonDecode(
            formattedFixtureJson.replaceAll(
              'SHEP-FIXTURE-GENERATION',
              generation,
            ),
          )
          as Map<String, dynamic>,
    );
  }
}
