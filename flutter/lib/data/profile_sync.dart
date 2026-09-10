/// Small native observations of ongoing preference sync. Values here describe
/// the device ledger; they never carry credentials, tokens or mail.
class ProfileSyncStatus {
  ProfileSyncStatus.fromJson(Map<String, dynamic> data)
    : id = data['id'] as String,
      name = data['name'] as String?,
      enabled = data['enabled'] as bool,
      revision = data['revision'] as int,
      fields = Map<String, bool>.unmodifiable(
        (data['fields'] as Map).cast<String, bool>(),
      ),
      staged = data['staged'] as int,
      deferred = data['deferred'] as int,
      reviews = data['reviews'] as int,
      applications = data['applications'] as int,
      unproven = data['unproven'] as int,
      error = data['error'] as String?,
      last = data['last'] == null
          ? null
          : ProfileSyncReport.fromJson(data['last'] as Map<String, dynamic>);
  final String id;
  final String? name, error;
  final bool enabled;
  final int revision, staged, deferred, reviews, applications, unproven;
  final Map<String, bool> fields;
  final ProfileSyncReport? last;
  bool fieldEnabled(String field) => fields[field] ?? true;
}

class ProfileSyncReport {
  ProfileSyncReport.fromJson(Map<String, dynamic> data)
    : admitted = data['admitted'] as int,
      imported = data['imported'] as int,
      applied = data['applied'] as int,
      published = data['published'] as int,
      deferred = data['deferred'] as int,
      remaining = data['remaining'] as bool;
  final int admitted, imported, applied, published, deferred;
  final bool remaining;
}

class ProfileSyncReview {
  ProfileSyncReview.fromJson(Map<String, dynamic> data)
    : id = data['id'] as String,
      field = data['field'] as String,
      kind = data['kind'] as String,
      local = data['local'],
      total = data['total'] as int,
      deciding = data['deciding'] as bool;
  final String id, field, kind;
  final Object? local;
  final int total;
  final bool deciding;
}

class ProfileSyncVersion {
  ProfileSyncVersion.fromJson(Map<String, dynamic> data)
    : operation = data['operation'] as String,
      value = data['value'],
      reset = data['reset'] as bool;
  final String operation;
  final Object? value;
  final bool reset;
}

abstract interface class ProfileSyncRepository {
  Future<dynamic> sync(String session, Map<String, Object?> command);
}
