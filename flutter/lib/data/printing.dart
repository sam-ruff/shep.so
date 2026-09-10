import 'package:flutter/services.dart';
import 'accounts.dart';

class PreparedPrint {
  const PreparedPrint({
    required this.document,
    required this.title,
    required this.signature,
    required this.accountId,
    this.issues = const [],
  });
  final String document, title, signature, accountId;
  final List<String> issues;
  factory PreparedPrint.fromJson(Map<String, dynamic> value) => PreparedPrint(
    document: value['document'] as String,
    title: value['title'] as String,
    signature: value['signature'] as String,
    accountId: value['account_id'] as String,
    issues: (value['issues'] as List).cast<String>(),
  );
}

abstract interface class PrintRepository {
  Future<PreparedPrint> preparePrint(
    String id, {
    required String generation,
    required bool plain,
  });
}

abstract interface class MessagePrinter {
  Future<void> open(PreparedPrint prepared, {required String generation});
}

class SystemMessagePrinter implements MessagePrinter {
  const SystemMessagePrinter();
  static const _channel = MethodChannel('so.shep/message-print');
  @override
  Future<void> open(
    PreparedPrint prepared, {
    required String generation,
  }) async {
    try {
      await _channel.invokeMethod<void>('print', {
        'document': prepared.document,
        'title': prepared.title,
        'generation': generation,
      });
    } on PlatformException catch (e) {
      throw MailOperationFailure(
        e.message ?? 'Could not open the print dialog. Retry.',
      );
    } on MissingPluginException {
      throw const MailOperationFailure(
        'Printing is unavailable on this device. Use a supported native client.',
      );
    }
  }
}
