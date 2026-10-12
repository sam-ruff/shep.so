import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/mail.dart';
import 'support/preview_repository.dart';

Mail mail({
  String id = 'message',
  String subject = 'Café',
  String recipient = '',
  String? senderHeader,
}) => Mail(
  id: id,
  sender: 'Sender',
  senderHeader: senderHeader,
  recipient: recipient,
  address: 'sender@example.test',
  subject: subject,
  preview: 'Preview',
  body: 'Body',
  date: DateTime(2026, 10, 6),
  account: 'Receiving account',
  accountId: 'receiver',
);

void main() {
  test('metadata survives aliases, projections and body eviction', () {
    const recipient =
        'Alias <alias@example.test>, "Café Team" <team@example.test>';
    const sender = r' "Robin \"RJ\" Field" <sender@example.test> ';
    final source = mail(recipient: recipient, senderHeader: sender);
    for (final copy in [
      source.patch({
        'id': 'provider-alias',
        'folder': 'Archive',
        'unread': false,
      }),
      source.withoutBody(),
      source.withoutBody().withDetail(
        mail(recipient: 'Older To', senderHeader: 'Older From'),
      ),
    ]) {
      expect(copy.recipient, recipient);
      expect(copy.senderHeader, sender);
      expect(copy.account, 'Receiving account');
    }
  });
  test(
    'missing and empty cached headers remain explicit without account fallback',
    () {
      final source = mail(senderHeader: '');
      expect(source.recipient, '');
      expect(source.senderHeader, '');
      expect(
        source
            .withoutBody()
            .withDetail(mail(recipient: 'Older hidden recipient'))
            .recipient,
        '',
      );
    },
  );
  test(
    'preview adapter carries provided header metadata exactly and preserves absence',
    () {
      const recipient =
          ' Team <group@example.test>, Álice <alice@example.test> ';
      const sender = r'"Robin \"RJ\" Field" <sender@example.test>';
      final supplied = PreviewRepository(
        firstHeaders: {'recipient': recipient, 'senderHeader': sender},
      ).cached.first;
      expect(supplied.recipient, recipient);
      expect(supplied.senderHeader, sender);
      final absent = PreviewRepository().cached.first;
      expect(absent.recipient, '');
      expect(absent.senderHeader, isNull);
    },
  );
}
