import 'accounts.dart';

class ProfileCreation {
  ProfileCreation.fromJson(Map<String, dynamic> value)
    : id = value['id'] as String,
      name = value['name'] as String,
      accounts = value['accounts'] as int,
      settings = value['settings'] as int,
      total = value['total'] as int,
      staged = value['staged'] as int,
      uploaded = value['uploaded'] as int,
      phase = value['phase'] as String,
      error = value['error'] as String?,
      settingValues = Map<String, Object?>.from(value['setting_values'] as Map);
  final String id, name, phase;
  final int accounts, settings, total, staged, uploaded;
  final String? error;
  final Map<String, Object?> settingValues;
  bool get complete => phase == 'complete';
  bool get needsReview => phase == 'review';
}

class ProfileAccountReview {
  ProfileAccountReview.fromJson(Map<String, dynamic> value)
    : position = value['position'] as int,
      account = MailAccount.fromJson(value['account'] as Map<String, dynamic>);
  final int position;
  final MailAccount account;
}

abstract interface class ProfileCreationRepository {
  Future<dynamic> creation(String session, Map<String, Object?> command);
}
