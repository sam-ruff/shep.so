import 'dart:async';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/model/mail.dart';
import 'paged_repository.dart';

const headerSubject = 'Café plans - ガ and 한글';
const headerSender = r'"Robin \"RJ\" Field" <sender@example.test>';
const headerRecipient =
    'Alias <alias@example.test>, "Café Team" <team@example.test>';

Mail headerMail({
  String id = 'header-message',
  String recipient = headerRecipient,
  String subject = headerSubject,
  String? senderHeader = headerSender,
  String body = '',
  bool loaded = false,
}) => Mail(
  id: id,
  sender: 'Robin "RJ" Field',
  senderHeader: senderHeader,
  recipient: recipient,
  address: 'sender@example.test',
  subject: subject,
  preview: 'Header fixture',
  body: body,
  date: DateTime(2026, 10, 6, 10, 30),
  account: 'Receiving account',
  accountId: 'account',
  unread: false,
  bodyLoaded: loaded,
);

class HeaderRepository extends PagedRepository {
  Mail metadata = headerMail();
  Completer<Mail> body = Completer<Mail>();
  Map<String, String> aliases = {};
  @override
  List<Mail> get cached => [metadata];
  @override
  Future<Mail> detail(String id) => body.future;
  @override
  Future<MailPage> page({
    required String folder,
    String? account,
    required String query,
    required String filter,
    required bool oldest,
    required int offset,
    Map<String, Map<String, Object>> projection = const {},
  }) async => MailPage([metadata], 1, 0, aliases: aliases);
}
