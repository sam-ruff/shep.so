import 'dart:async';
import 'package:shep_mobile/data/profile_discovery.dart';
import 'package:shep_mobile/data/profile_sync.dart';
import 'profile_discovery_fixture.dart';
import 'profile_enrollment_fixture.dart';

/// Isolated stand-in for the native sync ledger. It follows the same rules as
/// Rust: local intent is measured by device revisions, one application waits
/// for its receipt at a time, and concurrent changes become reviews.
class FixtureProfileSync extends FixtureProfileEnrollment
    implements ProfileSyncRepository {
  FixtureProfileSync(super.mail, {super.accounts, bool completed = false}) {
    if (completed) {
      job = {
        'id': fixtureUuid(900),
        'name': 'Personal',
        'phase': 'complete',
        'copied': 3,
        'total': 3,
        'rows': 2,
        'applied': accounts,
        'kept': 0,
        'include_accounts': true,
        'include_settings': true,
        'baseline': {
          'values': {
            'appearance': 'Light',
            'left_swipe': 'archive',
            'right_swipe': 'read',
            'preview_lines': 4,
            'sender_pictures': true,
            'unified_inbox': true,
            'reply_display': 'Collapsed',
            'tooltips': true,
          },
          'revisions': {for (final key in _fields) key: 0},
        },
        'settings_receipt': {
          'applied': ['appearance', 'preview_lines'],
          'kept': [],
          'revisions': {for (final key in _fields) key: 0},
        },
        'error': null,
      };
    }
  }
  static const _fields = [
    'appearance',
    'left_swipe',
    'right_swipe',
    'preview_lines',
    'sender_pictures',
    'unified_inbox',
    'reply_display',
    'tooltips',
  ];
  Map<String, dynamic>? subscription;
  final bases = <String, Map<String, dynamic>>{};

  /// Shared profile values as the other device published them.
  final remote = <String, Object?>{'appearance': 'Dark', 'preview_lines': null};

  /// Remote edits not yet pulled by this device: field -> value.
  final pendingRemote = <String, Object?>{};

  /// Remote edits another device makes concurrently with this device's next
  /// local edit of the same field; saved flows use it to provoke conflicts
  /// regardless of how many foreground ticks ran in between.
  final concurrent = <String, List<Object?>>{};
  final published = <Map<String, Object?>>[];
  final reviews = <Map<String, dynamic>>[];
  Map<String, dynamic>? application;
  Map<String, dynamic>? receipt;
  int cycles = 0, confirms = 0, subscribes = 0, operations = 0;
  int conflictVersions = 2;
  bool lostConfirmOnce = false, failCycleOnce = false;
  Completer<void>? holdCycle;

  Map<String, dynamic> _status() => {
    ...subscription!,
    'bases': {for (final e in bases.entries) e.key: {...e.value}},
    'staged': 0,
    'deferred': 0,
    'reviews': reviews.length,
    'applications': application == null ? 0 : 1,
    'unproven': bases.values
        .where((b) => b['native_revision'] == null)
        .length,
  };
  String _operation() => fixtureUuid(1000 + operations++);

  @override
  Future<dynamic> sync(String session, Map<String, Object?> command) async {
    if (active != session) {
      throw const DiscoveryFailure('Profile discovery changed.');
    }
    switch (command['kind']) {
      case 'current':
        return subscription == null ? null : _status();
      case 'subscribe':
        if (job?['phase'] != 'complete') {
          throw const DiscoveryFailure('Finish applying this profile first.');
        }
        if (subscription != null) return _status();
        subscribes++;
        final snapshot = command['snapshot'] as Map<String, dynamic>;
        final revisions = (snapshot['revisions'] as Map).cast<String, int>();
        final proof =
            (job!['settings_receipt']['revisions'] as Map?)?.cast<String, int>();
        for (final field in _fields) {
          final proven = proof?[field];
          bases[field] = {
            'operation': remote.containsKey(field) ? _operation() : null,
            'value': remote[field],
            'native_revision': proven,
            'observed': proven ?? revisions[field],
          };
        }
        subscription = {
          'id': fixtureUuid(700),
          'name': job!['name'],
          'enabled': false,
          'revision': 0,
          'fields': {for (final field in _fields) field: true},
          'last': null,
          'error': null,
        };
        return _status();
      case 'configure':
        if (command['expected_revision'] != subscription!['revision']) {
          throw const DiscoveryFailure(
            'Profile sync settings changed. Reopen Preferences.',
          );
        }
        if (command['enabled'] case final bool enabled) {
          subscription!['enabled'] = enabled;
        }
        if (command['field'] case final String field) {
          (subscription!['fields'] as Map)[field] = command['selected'];
        }
        subscription!['revision'] = (subscription!['revision'] as int) + 1;
        return _status();
      case 'cycle':
        cycles++;
        await holdCycle?.future;
        if (failCycleOnce) {
          failCycleOnce = false;
          subscription!['error'] = 'Could not reach Google Drive. Kept local changes.';
          throw const DiscoveryFailure(
            'Could not reach Google Drive. Kept local changes.',
          );
        }
        if (subscription!['enabled'] != true) {
          throw const DiscoveryFailure('Profile sync is turned off.');
        }
        subscription!['error'] = null;
        final snapshot = command['snapshot'] as Map<String, dynamic>;
        final values = snapshot['values'] as Map<String, dynamic>;
        final revisions = (snapshot['revisions'] as Map).cast<String, int>();
        var admitted = 0, published = 0, applied = 0, deferred = 0;
        bool enabled(String field) =>
            (subscription!['fields'] as Map)[field] != false;
        for (final field in _fields) {
          final basis = bases[field]!;
          if (!enabled(field) || revisions[field]! <= (basis['observed'] as int)) {
            continue;
          }
          // An open review follows the current local intent, like Rust's upsert.
          final open = reviews.where((r) => r['field'] == field).firstOrNull;
          if (open != null) {
            open['local'] = values[field];
            open['native_revision'] = revisions[field];
            continue;
          }
          if (concurrent[field]?.isNotEmpty == true) {
            pendingRemote[field] = concurrent[field]!.removeAt(0);
          }
          if (pendingRemote.containsKey(field)) {
            _review(field, 'conflict', values[field], revisions[field]!);
            deferred++;
            continue;
          }
          remote[field] = values[field];
          this.published.add({field: values[field]});
          bases[field] = {
            'operation': _operation(),
            'value': values[field],
            'native_revision': revisions[field],
            'observed': revisions[field],
          };
          admitted++;
          published++;
        }
        var remaining = false;
        for (final field in pendingRemote.keys.toList()) {
          if (!enabled(field) || reviews.any((r) => r['field'] == field)) {
            continue;
          }
          final basis = bases[field]!;
          if (basis['native_revision'] != revisions[field]) {
            _review(field, 'unproven', values[field], revisions[field]!);
            continue;
          }
          if (application != null) {
            remaining = true;
            continue;
          }
          final value = pendingRemote.remove(field);
          application = {
            'id': fixtureUuid(800 + cycles),
            'field': field,
            'operation': _operation(),
            'baseline': snapshot,
            'changes': {field: value},
          };
          applied++;
        }
        subscription!['last'] = {
          'admitted': admitted,
          'imported': 0,
          'applied': applied,
          'published': published,
          'deferred': deferred,
          'remaining': remaining,
        };
        return _status();
      case 'application':
        if (application == null) return null;
        return {
          'id': application!['id'],
          'baseline': application!['baseline'],
          'changes': application!['changes'],
        };
      case 'confirm_application':
        confirms++;
        if (application?['id'] != command['id']) {
          throw const DiscoveryFailure(
            'This preference application is not part of the current sync.',
          );
        }
        if (lostConfirmOnce) {
          lostConfirmOnce = false;
          throw const DiscoveryFailure(
            'The receipt was saved but its reply was lost. Sync again.',
          );
        }
        final field = application!['field'] as String;
        final revisions = (command['revisions'] as Map).cast<String, int>();
        if ((command['applied'] as List).contains(field)) {
          bases[field] = {
            'operation': application!['operation'],
            'value': (application!['changes'] as Map)[field],
            'native_revision': revisions[field],
            'observed': revisions[field],
          };
          remote[field] = (application!['changes'] as Map)[field];
        }
        receipt = {...command};
        application = null;
        return _status();
      case 'reviews':
        return reviews
            .map(
              (r) => {
                'id': r['id'],
                'field': r['field'],
                'kind': r['kind'],
                'local': r['local'],
                'native_revision': r['native_revision'],
                'history_revision': 0,
                'versions': r['versions'],
                'total': (r['versions'] as List).length,
                'deciding': false,
              },
            )
            .toList();
      case 'review_versions':
        final review = reviews.firstWhere((r) => r['id'] == command['id']);
        final versions = review['versions'] as List<String>;
        final after = command['after'] as String?;
        final start = after == null ? 0 : versions.indexOf(after) + 1;
        return versions
            .skip(start)
            .take(50)
            .map(
              (operation) => {
                'operation': operation,
                'value': review['remote'],
                'reset': review['remote'] == null,
              },
            )
            .toList();
      case 'decide':
        final review = reviews.firstWhere((r) => r['id'] == command['id']);
        final field = review['field'] as String;
        if (command['seen'] != (review['versions'] as List).length) {
          throw const DiscoveryFailure(
            'Open every version page before choosing.',
          );
        }
        final snapshot = command['snapshot'] as Map<String, dynamic>;
        if ((snapshot['revisions'] as Map)[field] != review['native_revision']) {
          throw const DiscoveryFailure(
            'This preference changed on the device. Refresh the review before choosing.',
          );
        }
        final choice = command['choice'] as Map;
        reviews.remove(review);
        pendingRemote.remove(field);
        if (choice['kind'] == 'local') {
          remote[field] = review['local'];
          published.add({field: review['local']});
          bases[field] = {
            'operation': _operation(),
            'value': review['local'],
            'native_revision': review['native_revision'],
            'observed': review['native_revision'],
          };
        } else {
          if (!(review['versions'] as List).contains(choice['operation'])) {
            throw const DiscoveryFailure('Choose a version from this review.');
          }
          application = {
            'id': fixtureUuid(850 + operations),
            'field': field,
            'operation': choice['operation'],
            'baseline': snapshot,
            'changes': {field: review['remote']},
          };
        }
        return _status();
      default:
        throw const DiscoveryFailure('Unknown sync fixture command.');
    }
  }

  void _review(String field, String kind, Object? local, int revision) {
    reviews.add({
      'id': fixtureUuid(600 + reviews.length),
      'field': field,
      'kind': kind,
      'local': local,
      'remote': pendingRemote[field],
      'native_revision': revision,
      'versions': List.generate(
        kind == 'conflict' ? conflictVersions : 1,
        (i) => fixtureUuid(2000 + reviews.length * 100 + i),
      ),
    });
  }
}
