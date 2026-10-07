import '../model/remote_images.dart';

class PreparedMessage {
  const PreparedMessage({
    required this.signature,
    required this.text,
    this.document,
    this.remoteImages = const [],
    this.issues = const [],
    this.senderAddress,
    this.senderDomain,
  });
  final String signature, text;
  final String? document;
  final List<RemoteImage> remoteImages;
  final List<String> issues;

  /// The native parse of the From header that image exceptions match.
  final String? senderAddress, senderDomain;
  factory PreparedMessage.fromJson(Map<String, dynamic> json) =>
      PreparedMessage(
        signature: json['signature'] as String,
        text: json['text'] as String,
        document: json['document'] as String?,
        remoteImages: (json['remote_images'] as List)
            .map(
              (v) => RemoteImage(
                v['url'] as String,
                v['alt'] as String,
                key: v['key'] as String? ?? '',
              ),
            )
            .toList(),
        issues: (json['issues'] as List).cast<String>(),
        senderAddress: json['sender_address'] as String?,
        senderDomain: json['sender_domain'] as String?,
      );
}

class RemoteImage {
  const RemoteImage(this.url, this.alt, {this.key = ''});
  final String url, alt;

  /// Names this image's placeholder in the confined document.
  final String key;
}

abstract interface class FormattedMessageRepository {
  Future<PreparedMessage> formattedMessage(
    String id, {
    required String generation,
    required bool dark,
    required bool quotes,
  });
}

/// Converted WebP bytes, still base64 encoded, for the confined runtime.
class RemoteImageBytes {
  const RemoteImageBytes(this.bytes, this.width, this.height);
  final String bytes;
  final int width, height;
  Map<String, Object?> toJson() => {
    'bytes': bytes,
    'width': width,
    'height': height,
  };
}

class RemoteImageBatch {
  const RemoteImageBatch({this.images = const {}, this.failed = const {}});
  final Map<String, RemoteImageBytes> images;
  final Map<String, String> failed;
  factory RemoteImageBatch.fromJson(Map<String, dynamic> json) =>
      RemoteImageBatch(
        images: {
          for (final entry in (json['images'] as Map).entries)
            entry.key as String: RemoteImageBytes(
              entry.value['bytes'] as String,
              entry.value['width'] as int,
              entry.value['height'] as int,
            ),
        },
        failed: (json['failed'] as Map).cast<String, String>(),
      );
}

/// Native fetches permitted images; the reader never gives the document a URL.
abstract interface class RemoteImageRepository {
  /// At most [batch] keys from one prepared message.
  Future<RemoteImageBatch> remoteImages(
    String id, {
    required List<String> keys,
    required ImageRules rules,
  });

  /// Drops every cached image after a permission is narrowed.
  Future<void> forgetRemoteImages();

  static const batch = 8;
}
