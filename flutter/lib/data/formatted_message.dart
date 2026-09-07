class PreparedMessage {
  const PreparedMessage({
    required this.signature,
    required this.text,
    this.document,
    this.remoteImages = const [],
    this.issues = const [],
  });
  final String signature, text;
  final String? document;
  final List<RemoteImage> remoteImages;
  final List<String> issues;
  factory PreparedMessage.fromJson(Map<String, dynamic> json) =>
      PreparedMessage(
        signature: json['signature'] as String,
        text: json['text'] as String,
        document: json['document'] as String?,
        remoteImages: (json['remote_images'] as List)
            .map((v) => RemoteImage(v['url'] as String, v['alt'] as String))
            .toList(),
        issues: (json['issues'] as List).cast<String>(),
      );
}

class RemoteImage {
  const RemoteImage(this.url, this.alt);
  final String url, alt;
}

abstract interface class FormattedMessageRepository {
  Future<PreparedMessage> formattedMessage(
    String id, {
    required String generation,
    required bool dark,
    required bool quotes,
  });
}
