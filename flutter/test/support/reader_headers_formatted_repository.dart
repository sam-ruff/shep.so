import 'dart:convert';
import 'package:shep_mobile/data/formatted_message.dart';
import 'package:shep_mobile/model/mail.dart';
import 'formatted_fixture.dart';
import 'reader_headers_repository.dart';

class HeaderFormattedRepository extends HeaderRepository
    implements FormattedMessageRepository {
  HeaderFormattedRepository() {
    metadata = headerMail(id: '1');
  }
  int refreshes = 0;
  @override
  Future<List<Mail>> refresh() async {
    final older = metadata;
    refreshes++;
    metadata = headerMail(
      id: '1',
      recipient: 'Refreshed $refreshes <refresh-$refreshes@example.test>',
    );
    if (!body.isCompleted) {
      body.complete(
        older.withDetail(
          headerMail(body: 'Complete prepared source', loaded: true),
        ),
      );
    }
    return cached;
  }

  @override
  Future<PreparedMessage> formattedMessage(
    String id, {
    required String generation,
    required bool dark,
    required bool quotes,
  }) async {
    await body.future;
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
