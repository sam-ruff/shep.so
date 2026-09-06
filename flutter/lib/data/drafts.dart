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
