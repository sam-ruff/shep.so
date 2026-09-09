import '../model/preferences.dart';

class ProfileSettingsSnapshot {
  ProfileSettingsSnapshot(this.preferences, Map<String, int> revisions)
    : revisions = Map.unmodifiable(revisions);
  final Preferences preferences;
  final Map<String, int> revisions;
  Map<String, Object?> toJson() => {
    'values': preferences.profileSettings(),
    'revisions': revisions,
  };
  factory ProfileSettingsSnapshot.fromJson(Map<String, dynamic> data) =>
      ProfileSettingsSnapshot(
        const Preferences().applyProfile(
          Map<String, Object?>.from(data['values'] as Map),
        ),
        Map<String, int>.from(data['revisions'] as Map),
      );
}

class ProfileSettingsReceipt {
  const ProfileSettingsReceipt({
    required this.id,
    required this.preferences,
    required this.applied,
    required this.kept,
  });
  final String id;
  final Preferences preferences;
  final List<String> applied, kept;
}

/// Device-local receipts protect newer edits when native enrollment resumes
/// after a lost acknowledgment. No receipt or revision is exported to Google.
abstract interface class ProfileSettingsStore {
  Future<ProfileSettingsSnapshot> profileSnapshot();
  Future<Preferences> saveLocal(Map<String, Object?> changes);
  Future<ProfileSettingsReceipt> applyProfile({
    required String id,
    required ProfileSettingsSnapshot baseline,
    required Map<String, Object?> changes,
  });
}
