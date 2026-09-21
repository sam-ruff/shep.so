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
  Map<String, dynamic>? get mutation =>
      (data['mutation'] as Map?)?.cast<String, dynamic>();
  Map<String, dynamic>? get plan =>
      (mutation?['review'] as Map?)?['plan'] as Map<String, dynamic>?;
  String get name {
    final review = mutation?['review'] as Map?;
    if (const {'rejected', 'cancelled'}.contains(status)) {
      return review?['source_label'] as String? ?? data['name'] as String;
    }
    if (review?['destination_label'] case final String label) return label;
    if (review?['source_label'] case final String label) return label;
    final members = (plan?['members'] as List?)?.cast<Map>() ?? const [];
    final root = members
        .where((member) => member['path'] == plan?['source'])
        .firstOrNull;
    return root?['destination'] as String? ?? data['name'] as String;
  }

  String? get parent => data['parent'] as String?;
  String get status => data['status'] as String;
  int get revision => data['revision'] as int;
  String? get error => data['error'] as String?;
  bool get acknowledged => data['acknowledged'] == true;
  bool get hasReceipt => mutation == null
      ? data['receipt'] != null
      : mutation?['receipt'] != null && mutation?['observed'] == true;
  bool get pending =>
      !const {'succeeded', 'cancelled', 'dismissed'}.contains(status);
  bool get runnable =>
      const {'queued', 'waiting', 'checking', 'repair'}.contains(status);
  bool get canCancel =>
      (mutation == null
          ? !acknowledged
          : mutation?['completed'] == 0 && mutation?['receipt'] == null) &&
      const {'queued', 'waiting', 'planning', 'rejected'}.contains(status);
  bool get canRetry =>
      (mutation == null ? !acknowledged : mutation?['receipt'] == null) &&
      status == 'rejected';
  bool get canCheck =>
      const {'repair', 'uncertain', 'running'}.contains(status) ||
      mutation != null && status == 'rejected';
  bool get canDismiss =>
      (mutation == null ||
          mutation?['checked'] == true ||
          const {'succeeded', 'cancelled'}.contains(status)) &&
      const {
        'repair',
        'uncertain',
        'rejected',
        'succeeded',
        'cancelled',
      }.contains(status);
  String? get changeVerb {
    final action = plan?['action'];
    if (action == 'Delete') return 'Delete';
    if (action is Map && action.containsKey('Rename')) return 'Rename';
    if (action is Map && action.containsKey('Move')) return 'Move';
    return null;
  }

  String get label => switch (status) {
    'queued' => changeVerb == null ? 'Queued' : '$changeVerb queued',
    'waiting' => 'Waiting to connect',
    'planning' => 'Preparing folder',
    'running' => mutation == null ? 'Creating folder' : 'Changing folder',
    'checking' => 'Checking server',
    'repair' =>
      hasReceipt
          ? 'Saving on this device'
          : mutation == null
          ? 'Created, checking server'
          : 'Changed, checking server',
    'uncertain' => 'Needs checking',
    'rejected' =>
      mutation == null
          ? 'Could not create folder'
          : 'Folder change needs review',
    'succeeded' =>
      changeVerb == 'Delete'
          ? 'Folder deleted'
          : mutation == null
          ? 'Folder ready'
          : 'Folder changed',
    'cancelled' => 'Cancelled',
    _ => 'Tracking stopped',
  };
}

abstract interface class FolderChangeRepository
    implements FolderCreationRepository {
  Future<Map<String, dynamic>> reviewFolderChange(
    FolderAccount account,
    String source,
    Object action,
  );
  Future<FolderCreation> admitFolderChange(
    String id,
    Map<String, dynamic> review,
  );
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
