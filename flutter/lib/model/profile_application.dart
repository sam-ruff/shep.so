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
  ProfileSettingsStore get store => workspace.settings as ProfileSettingsStore;
  @override
  Future<ProfileSettingsSnapshot> captureProfilePreferences() async {
    await workspace._settingsQueue;
    final snapshot = await store.profileSnapshot();
    if (!mapEquals(
      snapshot.preferences.profileSettings(),
      workspace.preferences.profileSettings(),
    )) {
      throw const DiscoveryFailure(
        'Save your current preferences before preparing a profile review.',
      );
    }
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
    workspace.preferences = prior.applyProfile({
      for (final entry in changes.entries)
        if (current[entry.key] == original[entry.key]) entry.key: entry.value,
    });
    workspace._changed();
    final result = workspace._settingsQueue.then((_) async {
      try {
        final receipt = await store.applyProfile(
          id: id,
          baseline: baseline,
          changes: changes,
        );
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
