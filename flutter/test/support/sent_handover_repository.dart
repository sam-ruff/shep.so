import 'dart:async';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/model/mail.dart';
import 'paged_repository.dart';

// Synthetic transport checkpoints, never a running-application action API.
class SentHandoverRepository extends PagedRepository {
  static const providerId = 'fixture:Sent Mail:91.4';
  static const localId = 'fixture:Sent:local-sent-copy';
  static const subject = 'Sent handover fixture';
  final syncGate = Completer<void>();
  final bodyGate = Completer<void>();
  bool syncStarted = false, bodyStarted = false, adopted = false;
  final changes = <(String, Map<String, Object>)>[];
  Mail message = Mail(
    id: providerId,
    sender: 'Alex',
    address: 'alex@example.test',
    subject: subject,
    preview: 'Exact saved body',
    body: 'Exact saved body after Sent handover.',
    date: DateTime(2026, 9, 6),
    accountId: 'fixture',
    folder: 'Sent Mail',
    unread: false,
  );
  @override
  List<Mail> get cached => [message];
  @override
  Future<List<Mail>> refresh() async {
    syncStarted = true;
    await syncGate.future;
    adopted = true;
    message = Mail(
      id: localId,
      sender: message.sender,
      address: message.address,
      subject: message.subject,
      preview: message.preview,
      body: message.body,
      date: message.date,
      accountId: message.accountId,
      folder: message.folder,
      unread: message.unread,
      starred: message.starred,
    );
    return cached;
  }

  @override
  Future<MailPage> page({
    required String folder,
    String? account,
    required String query,
    required String filter,
    required bool oldest,
    required int offset,
  }) async {
    final rows =
        folder == message.folder ||
            (folder == 'Sent' && message.folder == 'Sent Mail')
        ? [message.withoutBody()]
        : <Mail>[];
    return MailPage(
      rows,
      rows.length,
      0,
      aliases: adopted && rows.isNotEmpty ? {providerId: localId} : {},
      folderMembership: folder == 'Sent'
          ? {
              'fixture': {'Sent Mail'},
            }
          : {},
    );
  }

  @override
  Future<Mail> detail(String id) async {
    final old = message;
    bodyStarted = true;
    await bodyGate.future;
    return old;
  }

  @override
  Future<void> mutate(String id, Map<String, Object> fields) async {
    if (id != message.id) {
      throw StateError('A stale UI identity reached this transport');
    }
    changes.add((id, Map.of(fields)));
    message = message.patch(fields);
  }
}
