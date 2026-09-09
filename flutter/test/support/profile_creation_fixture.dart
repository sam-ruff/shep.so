import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/profile_creation.dart';
import 'package:shep_mobile/data/profile_discovery.dart';
import 'profile_discovery_fixture.dart';

class FixtureProfileCreation extends FixtureProfileDiscovery
    implements ProfileCreationRepository {
  FixtureProfileCreation({this.accountCount = 2}) : super(count: 0);
  final int accountCount;
  Map<String, dynamic>? job;
  int steps = 0, prepared = 0;
  bool failUploadOnce = false, lostApprovalOnce = false;
  Completer<void>? holdStep;
  Map<String, dynamic>? copy() => job == null
      ? null
      : {
          ...job!,
          'setting_values': Map<String, Object?>.from(
            job!['setting_values'] as Map,
          ),
        };
  @override
  Future<dynamic> creation(String session, Map<String, Object?> command) async {
    if (active != session) {
      throw const DiscoveryFailure('Profile discovery changed.');
    }
    switch (command['kind']) {
      case 'current':
        return copy();
      case 'prepare':
        if (job != null && job!['phase'] != 'complete') {
          throw const DiscoveryFailure('Resume the saved publication first.');
        }
        final spec = command['specification'] as Map;
        final settings = Map<String, Object?>.from(spec['settings'] as Map);
        final n = spec['include_accounts'] == true ? accountCount : 0;
        prepared++;
        job = {
          'id': command['id'],
          'name': spec['name'],
          'accounts': n,
          'settings': settings.length,
          'setting_values': settings,
          'total': n + 3,
          'staged': 0,
          'uploaded': 0,
          'phase': 'review',
          'error': null,
        };
        return copy();
      case 'accounts':
        final after = command['after'] as int;
        return [
          for (var i = 0; i < (job!['accounts'] as int); i++)
            if (i + 2 > after)
              {
                'position': i + 2,
                'account': MailAccount(
                  id: fixtureUuid(i + 20),
                  name: 'Saved account ${i + 1}',
                  email: 'alex${i + 1}@example.test',
                  host: 'mail.example.test',
                  port: 993,
                  username: 'alex${i + 1}',
                  smtpHost: 'smtp.example.test',
                  smtpPort: 465,
                ).toJson(),
              },
        ].take(50).toList();
      case 'cancel':
        job = null;
        return null;
      case 'approve':
        if (!mapEquals(
          job!['setting_values'] as Map,
          command['settings'] as Map,
        )) {
          throw const DiscoveryFailure(
            'Preferences changed. Cancel this review and prepare another.',
          );
        }
        job!['phase'] = 'staging';
        if (lostApprovalOnce) {
          lostApprovalOnce = false;
          throw const DiscoveryFailure(
            'Could not confirm profile approval. Resume its saved publication.',
          );
        }
        return copy();
      case 'step':
        steps++;
        await holdStep?.future;
        if (job!['phase'] == 'staging') {
          job!['staged'] = (job!['staged'] as int) + 1;
          if (job!['staged'] == job!['total']) job!['phase'] = 'uploading';
        } else if (job!['phase'] == 'uploading') {
          if (failUploadOnce) {
            failUploadOnce = false;
            job!['error'] =
                'Profile upload could not be confirmed. Resume this publication.';
            throw DiscoveryFailure(job!['error'] as String);
          }
          job!['uploaded'] = (job!['uploaded'] as int) + 1;
          job!['error'] = null;
          if (job!['uploaded'] == job!['total']) job!['phase'] = 'complete';
        }
        if (job!['phase'] == 'complete') {
          saved = discoveryState(
            revision: saved.revision + 1,
            phase: 'complete',
            files: job!['total'] as int,
            profiles: 1,
          );
        }
        return copy();
      default:
        throw const DiscoveryFailure('Unknown fixture publication command.');
    }
  }

  @override
  Future<List<DiscoveredProfile>> profiles(
    String session,
    String? after,
  ) async {
    if (job == null || (job!['uploaded'] as int) == 0 || after != null) {
      return [];
    }
    return [
      DiscoveredProfile.fromJson({
        'profile': job!['id'],
        'generation': fixtureUuid(999),
        'name': job!['name'],
        'name_conflict': false,
        'accounts': job!['accounts'],
        'settings': job!['settings'],
        'waiting': 0,
        'ready': 0,
        'conflicts': 0,
        'removed': false,
        'initialized': job!['phase'] == 'complete',
      }),
    ];
  }
}
