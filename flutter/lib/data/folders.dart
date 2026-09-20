class FolderAccount {
  const FolderAccount(this.data);
  final Map<String, dynamic> data;
  String get id => data['account'] as String;
  String get connection => data['connection'] as String;
  String get label => data['label'] as String;
  String get email => data['email'] as String;
  String parentLabel(String wire) =>
      (data['parent_labels'] as Map?)?[wire] as String? ?? wire;
  List<String> get parents {
    final catalogue = (data['catalogue'] as List? ?? const []).cast<Map>();
    final known = {
      for (final entry in catalogue) entry['name'] as String: entry,
    };
    return {
      for (final name in (data['names'] as List).cast<String>())
        if (known[name]?['no_inferiors'] != true &&
            known[name]?['non_existent'] != true)
          name,
      for (final entry in catalogue)
        if (entry['no_inferiors'] != true && entry['non_existent'] != true)
          entry['name'] as String,
    }.toList();
  }
}

class FolderCreation {
  const FolderCreation(this.data);
  final Map<String, dynamic> data;
  String get id => data['id'] as String;
  String get account => data['account'] as String;
  String get name => data['name'] as String;
  String? get parent => data['parent'] as String?;
  String get status => data['status'] as String;
  int get revision => data['revision'] as int;
  String? get error => data['error'] as String?;
  bool get acknowledged => data['acknowledged'] == true;
  bool get hasReceipt => data['receipt'] != null;
  bool get pending =>
      !const {'succeeded', 'cancelled', 'dismissed'}.contains(status);
  bool get runnable =>
      const {'queued', 'waiting', 'checking', 'repair'}.contains(status);
  bool get canCancel =>
      !acknowledged &&
      const {'queued', 'waiting', 'planning', 'rejected'}.contains(status);
  bool get canRetry => !acknowledged && status == 'rejected';
  bool get canCheck =>
      const {'repair', 'uncertain', 'running'}.contains(status);
  bool get canDismiss => const {
    'repair',
    'uncertain',
    'rejected',
    'succeeded',
    'cancelled',
  }.contains(status);
  String get label => switch (status) {
    'queued' => 'Queued',
    'waiting' => 'Waiting to connect',
    'planning' => 'Preparing folder',
    'running' => 'Creating folder',
    'checking' => 'Checking server',
    'repair' =>
      hasReceipt ? 'Saving on this device' : 'Created, checking server',
    'uncertain' => 'Needs checking',
    'rejected' => 'Could not create folder',
    'succeeded' => 'Folder ready',
    'cancelled' => 'Cancelled',
    _ => 'Tracking stopped',
  };
}

abstract interface class FolderCreationRepository {
  Future<List<FolderAccount>> folderOptions();
  Future<List<FolderCreation>> folderCreations();
  Future<FolderCreation> admitFolder(
    String id,
    FolderAccount account,
    String? parent,
    String name,
  );
  Future<FolderCreation> executeFolder(FolderCreation request);
  Future<FolderCreation> decideFolder(FolderCreation request, String decision);
}
