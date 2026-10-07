import 'package:flutter/services.dart';
import '../model/mail.dart';

abstract interface class AttachmentRepository {
  Future<Uint8List> attachment(String message, ReceivedAttachment file);
}

/// The exact cached MIME bytes of a message, never re-encoded.
abstract interface class OriginalMessageRepository {
  Future<Uint8List> originalMessage(String id);
}

/// The platform document picker owns the user's destination. Cancellation is
/// distinct from a successful write and never changes the cached message.
class AttachmentSaver {
  const AttachmentSaver();
  static const channel = MethodChannel('so.shep/attachment-save');
  Future<bool> save(ReceivedAttachment file, Uint8List bytes) =>
      saveFile(file.name, file.mediaType, bytes);

  /// Desktop names an exported original `message.eml`; mobile matches it.
  Future<bool> saveOriginal(Uint8List bytes) =>
      saveFile('message.eml', 'message/rfc822', bytes);

  Future<bool> saveFile(String name, String type, Uint8List bytes) async =>
      await channel.invokeMethod<bool>('save', {
        'name': name,
        'type': type,
        'bytes': bytes,
      }) ??
      false;
}
