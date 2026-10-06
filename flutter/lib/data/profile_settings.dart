import '../model/preferences.dart';

bool validProfileFieldSet(Iterable<String> fields) {
  final current = const Preferences().profileSettings().keys.toSet();
  final legacy = {...current}..remove('reply_include_original');
  final selected = fields.toSet();
  return (selected.length == current.length && selected.containsAll(current)) ||
      (selected.length == legacy.length && selected.containsAll(legacy));
}

class ProfileSettingsSnapshot {
  ProfileSettingsSnapshot(this.preferences, Map<String, int> revisions)
    : revisions = Map.unmodifiable(revisions);
  final Preferences preferences;
  final Map<String, int> revisions;
  Map<String, Object?> get values {
    final result = preferences.profileSettings();
    if (validProfileFieldSet(revisions.keys) &&
        !revisions.containsKey('reply_include_original')) {
      result.remove('reply_include_original');
    }
    return result;
  }

  Map<String, Object?> toJson() => {'values': values, 'revisions': revisions};
  factory ProfileSettingsSnapshot.fromJson(Map<String, dynamic> data) {
    final values = Map<String, Object?>.from(data['values'] as Map);
    final revisions = Map<String, int>.from(data['revisions'] as Map);
    if (!validProfileFieldSet(values.keys) ||
        values.length != revisions.length ||
        !values.keys.every(revisions.containsKey)) {
      throw const FormatException(
        'Invalid settings snapshot. Reopen the profile.',
      );
    }
    return ProfileSettingsSnapshot(
      const Preferences().applyProfile(values),
      revisions,
    );
  }
}

class ProfileSettingsReceipt {
  const ProfileSettingsReceipt({
    required this.id,
    required this.preferences,
    required this.applied,
    required this.kept,
    this.revisions = const {},
  });
  final String id;
  final Preferences preferences;
  final List<String> applied, kept;

  /// Revisions at the original application, never a later retry's current state.
  /// Empty for legacy receipts whose exact applied revisions are unknown.
  final Map<String, int> revisions;
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
