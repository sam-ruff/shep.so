import 'dart:async';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/profile_discovery.dart';
import 'package:shep_mobile/data/profile_enrollment.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'paged_repository.dart';
import 'profile_discovery_fixture.dart';

class EnrollmentPreferences implements PreferenceStorage {
  String? bytes;
  @override
  Future<String?> read() async => bytes;
  @override
  Future<void> write(String value) async {
    bytes = value;
  }
}

class EnrollmentMail extends PagedRepository
    implements ProfileAccountRepository {
  final imported = <MailAccount>[];
  @override
  List<MailAccount> get mailAccounts => List.unmodifiable(imported);
  @override
  Set<String> get reconnectAccounts => imported.map((a) => a.id).toSet();
  int refreshes = 0;
  @override
  Future<void> refreshProfileAccounts() async {
    refreshes++;
  }
}

class FixtureProfileEnrollment extends FixtureProfileDiscovery
    implements ProfileEnrollmentRepository {
  FixtureProfileEnrollment(this.mail, {this.accounts = 2}) : super(count: 1);
  final EnrollmentMail mail;
  final int accounts;
  Map<String, dynamic>? job;
  final rows = <Map<String, dynamic>>[];
  int prepared = 0, steps = 0, settingsCalls = 0;
  bool lostAccountReplyOnce = false, lostSettingsReplyOnce = false;
  Completer<void>? holdStep;
  Map<String, dynamic>? copy() => job == null ? null : {...job!};
  @override
  Future<dynamic> enrollment(
    String session,
    Map<String, Object?> command,
  ) async {
    if (active != session) {
      throw const DiscoveryFailure('Profile discovery changed.');
    }
    switch (command['kind']) {
      case 'current':
        return copy();
      case 'prepare':
        if (job != null && job!['phase'] != 'complete') {
          throw const DiscoveryFailure('Resume this enrollment first.');
        }
        prepared++;
        job = {
          'id': command['id'],
          'name': 'Personal',
          'phase': 'copying',
          'copied': 0,
          'total': 3,
          'rows': accounts + 2,
          'applied': 0,
          'kept': 0,
          'include_accounts': true,
          'include_settings': true,
          'baseline': command['preferences'],
          'settings_receipt': null,
          'error': null,
        };
        rows.clear();
        for (var i = 0; i < accounts; i++) {
          rows.add({
            'position': i + 1,
            'target': 'account:${fixtureUuid(i + 20)}:connection',
            'kind': 'account',
            'account': MailAccount(
              id: 'profile-account-${i + 20}',
              name: 'Saved account ${i + 1}',
              email: 'alex${i + 1}@example.test',
              host: 'mail.example.test',
              port: 993,
              username: 'alex${i + 1}',
              smtpHost: 'smtp.example.test',
              smtpPort: 465,
            ).toJson(),
            'local': null,
            'value': null,
            'reason': null,
            'available': true,
            'selected': true,
            'receipt': null,
          });
        }
        for (final entry in {
          'appearance': 'Dark',
          'preview_lines': null,
        }.entries) {
          rows.add({
            'position': rows.length + 1,
            'target': 'setting:${entry.key}',
            'kind': 'setting',
            'account': null,
            'local': null,
            'value': entry.value,
            'reason': null,
            'available': true,
            'selected': true,
            'receipt': null,
          });
        }
        return copy();
      case 'rows':
        return rows
            .where((r) => (r['position'] as int) > (command['after'] as int))
            .take(50)
            .map((r) => {...r})
            .toList();
      case 'choose':
        if (job!['phase'] != 'review') {
          throw const DiscoveryFailure('This enrollment was already approved.');
        }
        rows.firstWhere(
          (r) => r['position'] == command['position'],
        )['selected'] = command['selected'];
        return copy();
      case 'cancel':
        job = null;
        rows.clear();
        return null;
      case 'approve':
        job!['include_accounts'] = command['accounts'];
        job!['include_settings'] = command['settings'];
        job!['phase'] = 'applying';
        return copy();
      case 'step':
        steps++;
        await holdStep?.future;
        if (job!['phase'] == 'copying') {
          job!['copied'] = (job!['copied'] as int) + 1;
          if (job!['copied'] == job!['total']) job!['phase'] = 'review';
        } else if (job!['phase'] == 'applying') {
          final pending = rows
              .where((r) => r['kind'] == 'account' && r['receipt'] == null)
              .firstOrNull;
          if (pending == null) {
            job!['phase'] = 'settings';
          } else if (pending['selected'] == true &&
              job!['include_accounts'] == true) {
            mail.imported.add(
              MailAccount.fromJson(pending['account'] as Map<String, dynamic>),
            );
            pending['receipt'] = 'applied';
            job!['applied'] = (job!['applied'] as int) + 1;
            if (lostAccountReplyOnce) {
              lostAccountReplyOnce = false;
              throw const DiscoveryFailure(
                'The account was saved but its reply was lost. Resume enrollment to confirm progress.',
              );
            }
          } else {
            pending['receipt'] = 'kept';
            job!['kept'] = (job!['kept'] as int) + 1;
          }
        }
        return copy();
      case 'settings':
        return {
          'id': job!['id'],
          'baseline': job!['baseline'],
          'changes': {
            if (job!['include_settings'] == true)
              for (final row in rows.where(
                (r) => r['kind'] == 'setting' && r['selected'] == true,
              ))
                (row['target'] as String).replaceFirst('setting:', ''):
                    row['value'],
          },
        };
      case 'confirm_settings':
        settingsCalls++;
        if (lostSettingsReplyOnce) {
          lostSettingsReplyOnce = false;
          throw const DiscoveryFailure(
            'Preferences were saved. Resume enrollment to confirm the receipt.',
          );
        }
        job!['settings_receipt'] = {
          'applied': command['applied'],
          'kept': command['kept'],
        };
        job!['phase'] = 'complete';
        return copy();
      default:
        throw const DiscoveryFailure('Unknown enrollment fixture command.');
    }
  }
}
