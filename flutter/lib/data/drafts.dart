import '../model/mail.dart';

class SelectedAttachment {
  const SelectedAttachment(this.path, this.name);
  final String path, name;
  Map<String, Object> toJson() => {'path': path, 'name': name};
}

class DraftFiles {
  const DraftFiles(this.attachments, this.revision);
  final List<DraftAttachment> attachments;
  final int revision;
  factory DraftFiles.fromJson(Map<String, dynamic> value) => DraftFiles(
    (value['attachments'] as List)
        .map((a) => DraftAttachment.fromJson(a))
        .toList(),
    value['file_revision'],
  );
}

abstract interface class DraftRepository {
  Future<DraftFiles> files(String id);
  Future<DraftFiles> addFiles(String id, List<SelectedAttachment> selected);
  Future<DraftFiles> removeFile(String id, String file);
  Future<Draft> reply(String id, bool all);
}

abstract interface class ForwardRepository {
  Future<Draft> forward(String id, String draftId);
}

/// Saves an unsent draft from a `mailto:` link through the shared parser.
/// [message] links come from received mail and keep only their address.
abstract interface class MailtoRepository {
  Future<Draft> mailtoDraft(
    String draftId,
    String link, {
    required String accountId,
    required bool message,
  });
}
