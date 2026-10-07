import '../model/preferences.dart';
import '../model/remote_images.dart';

/// Portable fields added after the first release, in release order. Older
/// snapshots, requests and receipts hold the fields before a suffix of these.
const _addedFields = ['reply_include_original', 'image_policy'];

bool validProfileFieldSet(Iterable<String> fields) {
  final current = const Preferences().profileSettings().keys.toSet();
  final selected = fields.toSet();
  for (var dropped = 0; dropped <= _addedFields.length; dropped++) {
    final expected = {...current}
      ..removeAll(_addedFields.sublist(_addedFields.length - dropped));
    if (selected.length == expected.length && selected.containsAll(expected)) {
      return true;
    }
  }
  return false;
}

class ProfileSettingsSnapshot {
  ProfileSettingsSnapshot(this.preferences, Map<String, int> revisions)
    : revisions = Map.unmodifiable(revisions);
  final Preferences preferences;
  final Map<String, int> revisions;
  Map<String, Object?> get values {
    final result = preferences.profileSettings();
    if (validProfileFieldSet(revisions.keys)) {
      result.removeWhere((key, _) => !revisions.containsKey(key));
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

  /// `imageTrust` replaces the saved device-local image exceptions.
  Future<Preferences> saveLocal(
    Map<String, Object?> changes, {
    ImageTrust? imageTrust,
  });
  Future<ProfileSettingsReceipt> applyProfile({
    required String id,
    required ProfileSettingsSnapshot baseline,
    required Map<String, Object?> changes,
  });
}
