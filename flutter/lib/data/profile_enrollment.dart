import 'accounts.dart';
import 'profile_settings.dart';

class ProfileEnrollment {
  ProfileEnrollment.fromJson(Map<String, dynamic> data)
    : baselineValues = Map<String, Object?>.from(
        (data['baseline'] as Map)['values'] as Map,
      ),
      settingsReceipt = data['settings_receipt'] as Map?,
      id = data['id'] as String,
      name = data['name'] as String?,
      phase = data['phase'] as String,
      copied = data['copied'] as int,
      total = data['total'] as int,
      rows = data['rows'] as int,
      applied = data['applied'] as int,
      kept = data['kept'] as int,
      includeAccounts = data['include_accounts'] as bool,
      includeSettings = data['include_settings'] as bool,
      error = data['error'] as String?;
  final Map<String, Object?> baselineValues;
  final Map? settingsReceipt;
  final String id, phase;
  final String? name, error;
  final int copied, total, rows, applied, kept;
  final bool includeAccounts, includeSettings;
  bool get complete => phase == 'complete';
  bool get needsReview => phase == 'review';
  bool get preparing =>
      const ['copying', 'draining', 'fields', 'planning'].contains(phase);
}

class EnrollmentRow {
  EnrollmentRow.fromJson(Map<String, dynamic> data)
    : position = data['position'] as int,
      target = data['target'] as String,
      kind = data['kind'] as String,
      account = data['account'] == null
          ? null
          : MailAccount.fromJson(data['account']),
      local = data['local'] == null
          ? null
          : MailAccount.fromJson(data['local']),
      value = data['value'],
      selected = data['selected'] as bool,
      available = data['available'] as bool,
      reason = data['reason'] as String?,
      receipt = data['receipt'] as String?;
  final int position;
  final String target, kind;
  final MailAccount? account, local;
  final Object? value;
  final bool selected, available;
  final String? reason, receipt;
}

abstract interface class ProfileEnrollmentRepository {
  Future<dynamic> enrollment(String session, Map<String, Object?> command);
}

abstract interface class ProfileEnrollmentDevice {
  Future<ProfileSettingsSnapshot> captureProfilePreferences();
  Future<ProfileSettingsReceipt> applyProfilePreferences(
    Map<String, dynamic> request,
  );
  Future<void> refreshProfileAccounts();
}

abstract interface class ProfileAccountRepository {
  Set<String> get reconnectAccounts;
  Future<void> refreshProfileAccounts();
}
