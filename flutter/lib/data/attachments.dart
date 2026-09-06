import 'package:flutter/services.dart';
import '../model/mail.dart';

abstract interface class AttachmentRepository {
  Future<Uint8List> attachment(String message, ReceivedAttachment file);
}

/// The platform document picker owns the user's destination. Cancellation is
/// distinct from a successful write and never changes the cached message.
class AttachmentSaver {
  const AttachmentSaver();
  static const channel = MethodChannel('so.shep/attachment-save');
  Future<bool> save(ReceivedAttachment file, Uint8List bytes) async =>
      await channel.invokeMethod<bool>('save', {
        'name': file.name,
        'type': file.mediaType,
        'bytes': bytes,
      }) ??
      false;
}
