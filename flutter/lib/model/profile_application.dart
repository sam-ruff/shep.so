part of 'workspace.dart';

extension _ProfilePreferenceMerge on Workspace {
  void _mergeProfilePreferences(
    Preferences saved,
    Map<String, int> generations,
  ) {
    final values = saved.profileSettings();
    preferences = preferences.applyProfile({
      for (final key in values.keys)
        if ((_preferenceFields[key] ?? 0) == (generations[key] ?? 0))
          key: values[key],
    });
  }
}

class WorkspaceProfileApplication implements ProfileEnrollmentDevice {
  WorkspaceProfileApplication(this.workspace);
  final Workspace workspace;
  ProfileSettingsSnapshot? _reviewed;
  Map<String, int> _reviewedGenerations = const {};
  String? _receiptId;
  ProfileSettingsStore get store => workspace.settings as ProfileSettingsStore;
  @override
  Future<ProfileSettingsSnapshot> captureProfilePreferences() async {
    await workspace._settingsQueue;
    final generations = Map<String, int>.of(workspace._preferenceFields);
    final snapshot = await store.profileSnapshot();
    if (!mapEquals(generations, workspace._preferenceFields) ||
        !mapEquals(
          snapshot.preferences.profileSettings(),
          workspace.preferences.profileSettings(),
        )) {
      throw const DiscoveryFailure(
        'Save your current preferences before preparing a profile review.',
      );
    }
    _reviewed = snapshot;
    _reviewedGenerations = generations;
    return snapshot;
  }

  @override
  Future<ProfileSettingsReceipt> applyProfilePreferences(
    Map<String, dynamic> request,
  ) {
    final id = request['id'] as String;
    final baseline = ProfileSettingsSnapshot.fromJson(
      request['baseline'] as Map<String, dynamic>,
    );
    final changes = Map<String, Object?>.unmodifiable(
      request['changes'] as Map,
    );
    final prior = workspace.preferences;
    final before = Map<String, int>.of(workspace._preferenceFields);
    final current = prior.profileSettings(),
        original = baseline.preferences.profileSettings();
    // Immediate projection requires the same captured local intent. Values alone
    // cannot distinguish an untouched field from an edit changed back. A resumed
    // review after restart has no UI proof: show progress until storage checks its
    // durable revisions/receipt, keeping navigation and local editing available.
    final reviewed = _reviewed;
    final canProject =
        _receiptId != id &&
        reviewed != null &&
        mapEquals(reviewed.revisions, baseline.revisions) &&
        mapEquals(reviewed.preferences.profileSettings(), original);
    workspace.preferences = prior.applyProfile({
      for (final entry in changes.entries)
        if (canProject &&
            current[entry.key] == original[entry.key] &&
            (before[entry.key] ?? 0) == (_reviewedGenerations[entry.key] ?? 0))
          entry.key: entry.value,
    });
    workspace._changed();
    final result = workspace._settingsQueue.then((_) async {
      try {
        final receipt = await store.applyProfile(
          id: id,
          baseline: baseline,
          changes: changes,
        );
        _receiptId = receipt.id;
        workspace._mergeProfilePreferences(receipt.preferences, before);
        workspace._changed();
        return receipt;
      } catch (_) {
        workspace._mergeProfilePreferences(prior, before);
        workspace._changed();
        rethrow;
      }
    });
    workspace._settingsQueue = result.then<void>(
      (_) {},
      onError: (Object _, StackTrace _) {},
    );
    return result;
  }

  @override
  Future<void> refreshProfileAccounts() async {
    await (workspace.repository as ProfileAccountRepository)
        .refreshProfileAccounts();
    // Refresh metadata only: unsaved editors, cached mail and reader stay owned.
    workspace._changed();
  }
}
