import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/repository.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/google_connection.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/ui/calendar.dart';
import 'package:shep_mobile/ui/cal_dav_connections.dart';
import 'package:shep_mobile/ui/theme.dart';

import 'support/preview_repository.dart';
import 'support/google_fixture.dart';

class MemorySettings implements SettingsStore {
  Preferences value = const Preferences();
  @override
  Future<Preferences> read() async => value;
  @override
  Future<void> write(Preferences preferences) async => value = preferences;
}

class CalendarRepository extends PreviewRepository
    implements DurableCalendarRepository {
  CalendarRepository() : super(delay: Duration.zero);
  final admitted = <String>[];
  final admittedSubjects = <String>[];
  final executedSubjects = <String>[];
  final saved = <(CalendarEntry?, CalendarEntry)>[];
  final admissions = <String, CalendarAdmission>{};
  final executed = <String>[];
  final waited = <String>[];
  final cancelled = <String>[];
  final acceptedCurrent = <String>[];
  List<CalendarActivity> activity = const [];
  List<CalendarEntry> calendarCache = const [];
  String calendarSubject = FixtureGoogleAuthorization.subject;
  Future<CalendarSnapshot> Function()? snapshotOverride;
  Future<CalendarAdmission> Function(
    String,
    CalendarEntry,
    CalendarEntry?,
    String,
  )?
  admitOverride;
  Future<CalendarAdmission> Function(String, CalendarEntry, String)?
  deleteOverride;
  Future<CalendarAdmission?> Function(String)? admissionOverride;

  @override
  Future<CalendarAdmission> admitCalendarAction(
    String actionId,
    CalendarEntry entry,
    CalendarEntry? before, {
    required String subject,
  }) async {
    final override = admitOverride;
    if (override != null) {
      return override(actionId, entry, before, subject);
    }
    admitted.add(actionId);
    admittedSubjects.add(subject);
    saved.add((before, entry));
    return admissions[actionId] = CalendarAdmission({
      'id': actionId,
      'status': 'queued',
      'subject': subject,
      'mutation': {
        'save': {
          'before': before?.toCalendarJson(),
          'after': entry.toCalendarJson(),
        },
      },
      'receipt': null,
    });
  }

  @override
  Future<CalendarAdmission> admitCalendarDelete(
    String actionId,
    CalendarEntry entry, {
    required String subject,
  }) async {
    final override = deleteOverride;
    if (override != null) return override(actionId, entry, subject);
    admitted.add(actionId);
    admittedSubjects.add(subject);
    return admissions[actionId] = CalendarAdmission({
      'id': actionId,
      'status': 'queued',
      'subject': subject,
      'mutation': {
        'delete': {'before': entry.toCalendarJson()},
      },
      'receipt': null,
    });
  }

  @override
  Future<CalendarAdmission?> calendarActionAdmission(String actionId) async =>
      admissionOverride?.call(actionId) ?? admissions[actionId];

  @override
  Future<List<CalendarActivity>> calendarActions({int offset = 0}) async =>
      activity;

  @override
  Future<List<CalendarEntry>> calendarEvents() async => calendarCache;
  @override
  Future<CalendarSnapshot> calendarSnapshot() async =>
      await snapshotOverride?.call() ??
      CalendarSnapshot(const [], calendarCache, subject: calendarSubject);

  @override
  Future<void> cancelCalendarAction(String actionId) async {
    cancelled.add(actionId);
  }

  @override
  Future<void> acceptCalendarCurrentState(String actionId) async {
    acceptedCurrent.add(actionId);
    activity = activity.where((action) => action.id != actionId).toList();
  }

  @override
  Future<void> executeCalendarAction(
    String actionId,
    String accessToken, {
    required String subject,
  }) async {
    executed.add(actionId);
    executedSubjects.add(subject);
  }

  @override
  Future<void> inspectCalendarAction(
    String actionId,
    String accessToken, {
    required String subject,
  }) async {}

  @override
  Future<void> repairCalendarAction(String actionId) async {}

  @override
  Future<CalendarSnapshot> syncCalendar(
    String accessToken,
    DateTime start,
    DateTime end, {
    required String subject,
  }) async => const CalendarSnapshot([], []);

  @override
  Future<void> waitCalendarAction(String actionId, String error) async {
    waited.add(actionId);
  }
}

class CalDavCalendarRepository extends CalendarRepository
    implements DurableCalDavRepository {
  final connection = CalDavConnection({
    'connection': {
      'id': 'caldav-home',
      'url': 'https://calendar.example.test/home/',
      'username': 'sam',
    },
    'credential_slot': 'calendar-slot',
    'revision': 3,
  });
  final calDavAdmissions = <String, CalDavAdmission>{};
  final calDavExecuted = <String>[];
  final calDavInspected = <String>[];
  final setupAttempts = <CalDavAttempt>[];
  final savedPasswords = <String>[];
  final activatedAttempts = <String>[];
  Completer<void>? passwordRelease;
  bool failCleanup = false;
  bool failRemovalCleanup = false;
  bool refuseRemoval = false;
  int cleanupCalls = 0;
  Future<List<CalDavConnection>> Function()? connectionsOverride;
  Future<List<CalDavAttempt>> Function(bool pending)? attemptsOverride;

  CalDavAdmission _admit(String id) => calDavAdmissions[id] = CalDavAdmission({
    'id': id,
    'status': 'queued',
    'connection_id': connection.id,
    'connection_revision': connection.revision,
    'credential_slot': connection.credentialSlot,
  });

  @override
  Future<CalDavAdmission> admitCalDavAction(
    String actionId,
    CalendarEntry entry,
    CalendarEntry? before,
    CalDavConnection connection,
  ) async => _admit(actionId);

  @override
  Future<CalDavAdmission> admitCalDavDelete(
    String actionId,
    CalendarEntry entry,
    CalDavConnection connection,
  ) async => _admit(actionId);

  @override
  Future<CalDavAdmission?> calDavActionAdmission(String actionId) async =>
      calDavAdmissions[actionId];

  @override
  Future<void> executeCalDavAction(
    String actionId,
    String credentialSlot, {
    bool Function()? canDispatch,
  }) async {
    calDavExecuted.add(actionId);
  }

  @override
  Future<void> inspectCalDavAction(
    String actionId,
    String credentialSlot,
  ) async {
    calDavInspected.add(actionId);
  }

  @override
  Future<List<CalDavConnection>> calDavConnections() async =>
      await connectionsOverride?.call() ?? [connection];

  @override
  Future<List<CalDavAttempt>> calDavAttempts({bool pending = false}) async =>
      await attemptsOverride?.call(pending) ?? setupAttempts;

  @override
  Future<CalDavAttempt?> calDavAttempt(String id) async =>
      setupAttempts.where((attempt) => attempt.id == id).firstOrNull;

  @override
  Future<CalDavAttempt> admitCalDavConnection({
    required String attemptId,
    required String connectionId,
    required String url,
    required String username,
    CalDavConnection? observed,
  }) async {
    final attempt = CalDavAttempt({
      'id': attemptId,
      'status': 'prepared',
      'error': null,
      'request': {
        'connection': {'id': connectionId, 'url': url, 'username': username},
        'credential_slot': 'calendar-$attemptId',
      },
    });
    setupAttempts.add(attempt);
    return attempt;
  }

  @override
  Future<void> activateCalDavConnection(
    CalDavAttempt attempt, {
    bool Function()? canDispatch,
  }) async {
    activatedAttempts.add(attempt.id);
  }

  @override
  Future<void> cancelCalDavConnection(CalDavAttempt attempt) async {}
  @override
  Future<void> cleanupCalDavCredentials() async {
    cleanupCalls++;
    if (failCleanup) throw StateError('private cleanup detail');
  }

  @override
  Future<void> removeCalDavConnection(CalDavConnection connection) async {
    if (refuseRemoval) throw StateError('stale revision');
    if (failRemovalCleanup) {
      throw const CalendarCredentialCleanupFailure('private removal detail');
    }
  }

  @override
  Future<void> saveCalDavPassword(
    CalDavAttempt attempt,
    String password,
  ) async {
    savedPasswords.add(password);
    if (passwordRelease != null) await passwordRelease!.future;
  }
}

CalendarEntry entry(String id) => CalendarEntry(
  id,
  'Review',
  DateTime(2027, 1, 4, 10),
  DateTime(2027, 1, 4, 11),
  sourceId: 'primary',
);

Map<String, Object?> activityData(String status) => {
  'id': 'action',
  'status': status,
  'error': status == 'waiting' ? 'Connect Google Calendar' : null,
  'created': 1,
  'mutation': {
    'save': {'before': null, 'after': entry('local').toCalendarJson()},
  },
  'receipt': null,
};

Map<String, Object?> deleteActivity(CalendarEntry value, {int created = 1}) => {
  'id': 'delete-$created',
  'status': 'waiting',
  'error': null,
  'created': created,
  'mutation': {
    'delete': {'before': value.toCalendarJson()},
  },
  'receipt': null,
};

CalendarAdmission admission(
  String id,
  CalendarEntry requested, {
  String status = 'queued',
  CalendarEntry? saved,
}) => CalendarAdmission({
  'id': id,
  'status': status,
  'subject': FixtureGoogleAuthorization.subject,
  'mutation': {
    'save': {'before': null, 'after': requested.toCalendarJson()},
  },
  'receipt': saved == null
      ? null
      : {'request_id': id, 'before': null, 'after': saved.toCalendarJson()},
});

void main() {
  test('lost admission reply reconciles its saved provider identity', () async {
    final repository = CalendarRepository();
    var lookups = 0;
    final local = entry('local-create');
    final remote = entry('remote-create');
    repository.admissionOverride = (id) async {
      lookups++;
      return lookups == 1
          ? null
          : admission(id, local, status: 'succeeded', saved: remote);
    };
    repository.admitOverride = (id, value, before, subject) async {
      throw StateError('lost local reply');
    };
    final workspace = Workspace(repository, MemorySettings());
    await workspace.refreshCalendarActivity();
    expect(await workspace.saveEvent(local), isTrue);
    expect(workspace.events.map((value) => value.id), ['remote-create']);
    expect(repository.executed, isEmpty);
  });

  test(
    'changed edits wait when an earlier admission cannot be confirmed',
    () async {
      final repository = CalendarRepository();
      var admissions = 0;
      var lookups = 0;
      repository.admissionOverride = (_) async {
        lookups++;
        if (lookups == 1) return null;
        throw StateError('lookup unavailable');
      };
      repository.admitOverride = (id, value, before, subject) async {
        admissions++;
        throw StateError('lost local reply');
      };
      final workspace = Workspace(repository, MemorySettings());
      await workspace.refreshCalendarActivity();
      final original = entry('stable-create');
      expect(await workspace.saveEvent(original), isFalse);
      final changed = CalendarEntry(
        original.id,
        'Changed while uncertain',
        original.start,
        original.end,
        sourceId: original.sourceId,
      );
      expect(await workspace.saveEvent(changed), isFalse);
      expect(admissions, 1);
      expect(workspace.error, contains('previous save'));
    },
  );

  test(
    'definitive absent lookup retries the same admission identity',
    () async {
      final repository = CalendarRepository();
      final ids = <String>[];
      repository.admissionOverride = (_) async => null;
      repository.admitOverride = (id, value, before, subject) async {
        ids.add(id);
        if (ids.length == 1) throw StateError('lost before durable write');
        return admission(id, value);
      };
      final workspace = Workspace(repository, MemorySettings());
      await workspace.refreshCalendarActivity();
      final value = entry('stable-create');
      expect(await workspace.saveEvent(value), isFalse);
      expect(await workspace.saveEvent(value), isTrue);
      expect(ids, hasLength(2));
      expect(ids.toSet(), hasLength(1));
    },
  );

  test('rejected admission keeps the editor data and reloads cache', () async {
    final repository = CalendarRepository();
    final cached = entry('cached');
    repository.calendarCache = [cached];
    repository.admitOverride = (id, value, before, subject) async =>
        admission(id, value, status: 'rejected');
    final workspace = Workspace(repository, MemorySettings());
    await workspace.refreshCalendarActivity();
    expect(await workspace.saveEvent(entry('new-local')), isFalse);
    expect(workspace.events.map((value) => value.id), ['cached']);
    expect(repository.executed, isEmpty);
    expect(workspace.error, contains('not saved'));
  });

  test(
    'lost delete admission reconciles terminal success without dispatch',
    () async {
      final repository = CalendarRepository();
      final value = entry('delete-me');
      repository.calendarCache = [value];
      var lookups = 0;
      repository.admissionOverride = (id) async {
        lookups++;
        return lookups == 1
            ? null
            : CalendarAdmission({
                'id': id,
                'status': 'succeeded',
                'subject': FixtureGoogleAuthorization.subject,
                'mutation': {
                  'delete': {'before': value.toCalendarJson()},
                },
                'receipt': {
                  'request_id': id,
                  'before': value.toCalendarJson(),
                  'after': null,
                },
              });
      };
      repository.deleteOverride = (id, event, subject) async =>
          throw StateError('lost local reply');
      final workspace = Workspace(repository, MemorySettings());
      await workspace.refreshCalendarActivity();
      expect(await workspace.deleteEvent(value), isTrue);
      expect(workspace.events, isEmpty);
      expect(repository.executed, isEmpty);
    },
  );

  test(
    'completed prior create adopts remote identity before changed edits',
    () async {
      final repository = CalendarRepository();
      final local = entry('local-create');
      final remote = entry('remote-create');
      var lookups = 0;
      repository.admissionOverride = (id) async {
        lookups++;
        if (lookups == 1) return null;
        if (lookups == 2) throw StateError('lookup unavailable');
        return admission(id, local, status: 'succeeded', saved: remote);
      };
      repository.admitOverride = (id, value, before, subject) async =>
          throw StateError('lost local reply');
      final workspace = Workspace(repository, MemorySettings());
      await workspace.refreshCalendarActivity();
      expect(await workspace.saveEvent(local), isFalse);
      final changed = CalendarEntry(
        local.id,
        'Changed',
        local.start,
        local.end,
        sourceId: local.sourceId,
      );
      expect(await workspace.saveEvent(changed), isFalse);
      expect(workspace.events.map((value) => value.id), ['remote-create']);
      expect(workspace.error, contains('Reopen'));
      expect(await workspace.saveEvent(changed), isFalse);
      expect(workspace.events.map((value) => value.id), ['remote-create']);
    },
  );

  test(
    'snapshot generation change does not strand an admitted action',
    () async {
      final repository = CalendarRepository();
      final release = Completer<void>();
      repository.admitOverride = (id, value, before, subject) async {
        await release.future;
        return admission(id, value);
      };
      final authorisation = FixtureGoogleAuthorization();
      const permissions = GooglePermissions(
        calendar: GoogleCalendarPermission.edit,
      );
      final google = GoogleConnection(
        authorisation,
        MemoryGoogleStore(
          const GoogleConnectionState(
            requested: permissions,
            active: GoogleConnectionRecord(
              FixtureGoogleAuthorization.subject,
              FixtureGoogleAuthorization.email,
              permissions,
              FixtureGoogleAuthorization.application,
            ),
          ),
        ),
      );
      await google.load();
      final workspace = Workspace(repository, MemorySettings(), google: google);
      await workspace.refreshCalendarActivity();
      final saving = workspace.saveEvent(entry('generation'));
      await Future<void>.delayed(Duration.zero);
      await workspace.refreshCalendarActivity();
      release.complete();
      expect(await saving, isTrue);
      await Future<void>.delayed(Duration.zero);
      expect(repository.executed, hasLength(1));
    },
  );

  test('changed delete cannot reuse an uncertain admission request', () async {
    final repository = CalendarRepository();
    final original = entry('delete-stable');
    var admissions = 0;
    var lookups = 0;
    repository.admissionOverride = (_) async {
      lookups++;
      if (lookups == 1) return null;
      throw StateError('lookup unavailable');
    };
    repository.deleteOverride = (id, event, subject) async {
      admissions++;
      throw StateError('lost local reply');
    };
    final workspace = Workspace(repository, MemorySettings());
    expect(await workspace.deleteEvent(original), isFalse);
    final changed = CalendarEntry(
      original.id,
      'Changed',
      original.start,
      original.end,
      sourceId: original.sourceId,
    );
    expect(await workspace.deleteEvent(changed), isFalse);
    expect(admissions, 1);
    expect(workspace.error, contains('previous deletion'));
  });

  test('changed edit wakes its recovered queued admission', () async {
    final repository = CalendarRepository();
    final local = entry('wake-create');
    var lookups = 0;
    repository.admissionOverride = (id) async {
      lookups++;
      if (lookups == 1) return null;
      if (lookups == 2) throw StateError('lookup unavailable');
      return admission(id, local);
    };
    repository.admitOverride = (id, value, before, subject) async =>
        throw StateError('lost local reply');
    final authorisation = FixtureGoogleAuthorization();
    const permissions = GooglePermissions(
      calendar: GoogleCalendarPermission.edit,
    );
    final google = GoogleConnection(
      authorisation,
      MemoryGoogleStore(
        const GoogleConnectionState(
          requested: permissions,
          active: GoogleConnectionRecord(
            FixtureGoogleAuthorization.subject,
            FixtureGoogleAuthorization.email,
            permissions,
            FixtureGoogleAuthorization.application,
          ),
        ),
      ),
    );
    await google.load();
    final workspace = Workspace(repository, MemorySettings(), google: google);
    await workspace.refreshCalendarActivity();
    expect(await workspace.saveEvent(local), isFalse);
    final changed = CalendarEntry(
      local.id,
      'Changed',
      local.start,
      local.end,
      sourceId: local.sourceId,
    );
    expect(await workspace.saveEvent(changed), isFalse);
    await Future<void>.delayed(Duration.zero);
    expect(repository.executed, hasLength(1));
  });

  test(
    'held CalDAV credential save cannot activate after foreground changes',
    () async {
      final repository = CalDavCalendarRepository()
        ..passwordRelease = Completer<void>();
      final workspace = Workspace(repository, MemorySettings());
      final connecting = workspace.connectCalDav(
        url: 'https://calendar.example.test/home/',
        username: 'sam',
        password: 'secret',
      );
      await Future<void>.delayed(Duration.zero);
      expect(repository.savedPasswords, hasLength(1));
      workspace.setForeground(false);
      workspace.setForeground(true);
      repository.passwordRelease!.complete();
      expect(await connecting, false);
      expect(repository.activatedAttempts, isEmpty);
      expect(repository.setupAttempts, hasLength(1));
      workspace.dispose();
    },
  );

  test('disposed workspace leaves admitted action for restart', () async {
    final repository = CalendarRepository();
    final release = Completer<void>();
    repository.admitOverride = (id, value, before, subject) async {
      await release.future;
      return admission(id, value);
    };
    final workspace = Workspace(repository, MemorySettings());
    await workspace.refreshCalendarActivity();
    final saving = workspace.saveEvent(entry('disposed'));
    await Future<void>.delayed(Duration.zero);
    workspace.dispose();
    release.complete();
    expect(await saving, isTrue);
    await Future<void>.delayed(Duration.zero);
    expect(repository.executed, isEmpty);
  });

  test(
    'closing editor retains an admission whose outcome is still unknown',
    () async {
      final repository = CalendarRepository();
      var admissions = 0;
      var lookups = 0;
      repository.admissionOverride = (_) async {
        lookups++;
        if (lookups == 1) return null;
        throw StateError('lookup unavailable');
      };
      repository.admitOverride = (id, value, before, subject) async {
        admissions++;
        throw StateError('lost local reply');
      };
      final workspace = Workspace(repository, MemorySettings());
      final original = entry('closed');
      expect(await workspace.saveEvent(original), isFalse);
      workspace.releaseCalendarEditor(original.sourceId, original.id);
      await Future<void>.delayed(Duration.zero);
      final changed = CalendarEntry(
        original.id,
        'Changed',
        original.start,
        original.end,
        sourceId: original.sourceId,
      );
      expect(await workspace.saveEvent(changed), isFalse);
      expect(admissions, 1);
      expect(workspace.error, contains('previous save'));
    },
  );

  test(
    'unknown local admissions remain bounded before durable confirmation',
    () async {
      final repository = CalendarRepository();
      var attempts = 0;
      var lookups = 0;
      repository.admissionOverride = (_) async {
        if ((++lookups).isEven) throw StateError('lookup unavailable');
        return null;
      };
      repository.admitOverride = (id, value, before, subject) async {
        attempts++;
        throw StateError('local reply unavailable');
      };
      final workspace = Workspace(repository, MemorySettings());
      for (var i = 0; i < 32; i++) {
        expect(await workspace.saveEvent(entry('unknown-$i')), isFalse);
      }
      expect(await workspace.saveEvent(entry('overflow')), isFalse);
      expect(await workspace.deleteEvent(entry('overflow-delete')), isFalse);
      expect(attempts, 32);
      expect(workspace.error, contains('unresolved calendar changes'));
      repository.admissionOverride = (_) async =>
          throw StateError('still offline');
      for (var i = 0; i < 32; i++) {
        final value = entry('unknown-$i');
        workspace.releaseCalendarEditor(value.sourceId, value.id);
      }
      await Future<void>.delayed(Duration.zero);
      repository.admissionOverride = (_) async => null;
      await workspace.refreshCalendarActivity();
      expect(await workspace.saveEvent(entry('after-recovery')), isFalse);
      expect(attempts, 33);
    },
  );

  test(
    'closing a completed create keeps identity until cache is observed',
    () async {
      final repository = CalendarRepository();
      final original = entry('local-close');
      final saved = entry('remote-close');
      var lookups = 0;
      var attempts = 0;
      repository.admissionOverride = (id) async {
        lookups++;
        if (lookups == 1) return null;
        if (lookups == 2) throw StateError('lookup unavailable');
        return admission(id, original, status: 'succeeded', saved: saved);
      };
      repository.admitOverride = (id, value, before, subject) async {
        attempts++;
        throw StateError('local reply unavailable');
      };
      final workspace = Workspace(repository, MemorySettings());
      expect(await workspace.saveEvent(original), isFalse);
      repository.snapshotOverride = () async =>
          throw StateError('cache unavailable');
      workspace.releaseCalendarEditor(original.sourceId, original.id);
      await Future<void>.delayed(Duration.zero);
      final changed = CalendarEntry(
        original.id,
        'Changed',
        original.start,
        original.end,
        sourceId: original.sourceId,
      );
      expect(await workspace.saveEvent(changed), isFalse);
      expect(attempts, 1);
      repository.snapshotOverride = null;
      repository.calendarCache = [saved];
      workspace.releaseCalendarEditor(original.sourceId, original.id);
      await Future<void>.delayed(Duration.zero);
      expect(workspace.events.map((event) => event.id), [saved.id]);
    },
  );

  test(
    'offline admission retains the subject observed with cached events',
    () async {
      final repository = CalendarRepository();
      final workspace = Workspace(repository, MemorySettings());
      await workspace.refreshCalendarActivity();
      expect(await workspace.saveEvent(entry('offline')), isTrue);
      await Future<void>.delayed(Duration.zero);
      expect(repository.admittedSubjects, [FixtureGoogleAuthorization.subject]);
      expect(repository.executed, isEmpty);
      expect(repository.waited, repository.admitted);
    },
  );

  test(
    'replacement Google account cannot run a cached calendar deletion',
    () async {
      const permissions = GooglePermissions(
        calendar: GoogleCalendarPermission.edit,
      );
      final sdk = FixtureGoogleAuthorization();
      final google = GoogleConnection(
        sdk,
        MemoryGoogleStore(
          const GoogleConnectionState(
            requested: permissions,
            active: GoogleConnectionRecord(
              'replacement-subject',
              'other@example.test',
              permissions,
              FixtureGoogleAuthorization.application,
            ),
          ),
        ),
      );
      addTearDown(google.dispose);
      await google.load();
      final remote = entry('remote');
      final repository = CalendarRepository()..calendarCache = [remote];
      final workspace = Workspace(repository, MemorySettings(), google: google);
      await workspace.refreshCalendarActivity();
      expect(await workspace.deleteEvent(remote), isTrue);
      await Future<void>.delayed(Duration.zero);
      expect(repository.admittedSubjects, [FixtureGoogleAuthorization.subject]);
      expect(sdk.tokens, 0);
      expect(repository.executed, isEmpty);
      expect(repository.waited, repository.admitted);
    },
  );

  for (final dispose in [false, true]) {
    test(
      'held Calendar token cannot dispatch after ${dispose ? 'dispose' : 'background'}',
      () async {
        const permissions = GooglePermissions(
          calendar: GoogleCalendarPermission.edit,
        );
        final sdk = FixtureGoogleAuthorization()..hold = Completer<void>();
        final google = GoogleConnection(
          sdk,
          MemoryGoogleStore(
            const GoogleConnectionState(
              requested: permissions,
              active: GoogleConnectionRecord(
                FixtureGoogleAuthorization.subject,
                FixtureGoogleAuthorization.email,
                permissions,
                FixtureGoogleAuthorization.application,
              ),
            ),
          ),
        );
        await google.load();
        final repository = CalendarRepository();
        final workspace = Workspace(
          repository,
          MemorySettings(),
          google: google,
        );
        final running = workspace.retryCalendarActivity(
          CalendarActivity({
            ...activityData('waiting'),
            'subject': FixtureGoogleAuthorization.subject,
          }),
        );
        await Future<void>.delayed(Duration.zero);
        expect(sdk.tokens, 1);
        if (dispose) {
          workspace.dispose();
        } else {
          workspace.setForeground(false);
          workspace.setForeground(true);
        }
        sdk.hold!.complete();
        await running;
        expect(repository.executed, isEmpty);
        if (!dispose) workspace.dispose();
      },
    );
  }

  test('retry uses its saved subject with the matching SDK grant', () async {
    const permissions = GooglePermissions(
      calendar: GoogleCalendarPermission.edit,
    );
    final sdk = FixtureGoogleAuthorization();
    final google = GoogleConnection(
      sdk,
      MemoryGoogleStore(
        const GoogleConnectionState(
          requested: permissions,
          active: GoogleConnectionRecord(
            FixtureGoogleAuthorization.subject,
            FixtureGoogleAuthorization.email,
            permissions,
            FixtureGoogleAuthorization.application,
          ),
        ),
      ),
    );
    addTearDown(google.dispose);
    await google.load();
    final repository = CalendarRepository();
    final workspace = Workspace(repository, MemorySettings(), google: google);
    await workspace.retryCalendarActivity(
      CalendarActivity({
        ...activityData('waiting'),
        'subject': FixtureGoogleAuthorization.subject,
      }),
    );
    expect(sdk.tokens, 1);
    expect(repository.executedSubjects, [FixtureGoogleAuthorization.subject]);
    expect(repository.executed, ['action']);
  });

  test('late calendar snapshot cannot undo a newly admitted delete', () async {
    final remote = entry('remote');
    final held = Completer<CalendarSnapshot>();
    final started = Completer<void>();
    final repository = CalendarRepository();
    repository.snapshotOverride = () {
      repository.snapshotOverride = null;
      started.complete();
      return held.future;
    };
    final workspace = Workspace(repository, MemorySettings())
      ..events = [remote];
    final refresh = workspace.refreshCalendarActivity();
    await started.future;
    expect(await workspace.deleteEvent(remote), isTrue);
    held.complete(CalendarSnapshot(const [], [remote]));
    await refresh;
    expect(workspace.events, isEmpty);
  });

  test('superseded calendar refresh cannot publish its error', () async {
    final held = Completer<CalendarSnapshot>();
    final started = Completer<void>();
    final repository = CalendarRepository();
    repository.snapshotOverride = () {
      repository.snapshotOverride = null;
      started.complete();
      return held.future;
    };
    final workspace = Workspace(repository, MemorySettings());
    final oldRefresh = workspace.refreshCalendarActivity();
    await started.future;
    await workspace.refreshCalendarActivity();
    held.completeError(StateError('Outdated snapshot failed'));
    await oldRefresh;
    expect(workspace.error, isNull);
  });

  test(
    'save returns after durable admission and exposes waiting recovery',
    () async {
      final repository = CalendarRepository();
      final workspace = Workspace(repository, MemorySettings());

      expect(await workspace.saveEvent(entry('local')), isTrue);
      expect(repository.admitted, hasLength(1));
      expect(workspace.events.any((event) => event.id == 'local'), isTrue);
      await Future<void>.delayed(Duration.zero);
      expect(repository.executed, isEmpty);
      expect(repository.waited, repository.admitted);
    },
  );

  testWidgets(
    'calendar shows real Retry and Cancel controls for waiting work',
    (tester) async {
      final repository = CalendarRepository()
        ..activity = [CalendarActivity(activityData('waiting'))];
      final workspace = Workspace(repository, MemorySettings());
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(body: CalendarView(workspace: workspace)),
        ),
      );
      await tester.pump();

      expect(find.text('Event waiting'), findsOneWidget);
      expect(find.text('Review'), findsOneWidget);
      expect(find.text('Connect Google Calendar'), findsOneWidget);
      expect(find.widgetWithText(TextButton, 'Retry'), findsOneWidget);
      expect(find.widgetWithText(TextButton, 'Cancel'), findsOneWidget);

      await tester.tap(find.widgetWithText(TextButton, 'Cancel'));
      await tester.pump();
      expect(repository.cancelled, ['action']);
    },
  );

  test(
    'delete is admitted before token access and paints immediately',
    () async {
      final repository = CalendarRepository();
      final workspace = Workspace(repository, MemorySettings());
      final remote = CalendarEntry(
        'remote',
        'Review',
        DateTime(2027, 1, 4, 10),
        DateTime(2027, 1, 4, 11),
        sourceId: 'primary',
        etag: 'v1',
        remoteUrl: 'remote',
      );
      workspace.events = [remote];

      expect(await workspace.deleteEvent(remote), isTrue);
      expect(repository.admitted, hasLength(1));
      expect(workspace.events, isEmpty);
      await Future<void>.delayed(Duration.zero);
      expect(repository.executed, isEmpty);
      expect(repository.waited, repository.admitted);
    },
  );

  testWidgets('failed deletion identifies its event and offers cancellation', (
    tester,
  ) async {
    final repository = CalendarRepository()
      ..activity = [
        CalendarActivity({
          ...deleteActivity(entry('remote')),
          'status': 'rejected',
          'error': 'The event changed on the server.',
        }),
      ];
    final workspace = Workspace(repository, MemorySettings());
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(body: CalendarView(workspace: workspace)),
      ),
    );
    await tester.pump();
    expect(find.text('Event was not deleted'), findsOneWidget);
    expect(find.text('Review'), findsOneWidget);
    expect(find.text('The event changed on the server.'), findsOneWidget);
    expect(find.text('Event was not saved'), findsNothing);
    await tester.tap(find.widgetWithText(TextButton, 'Cancel'));
    await tester.pump();
    expect(repository.cancelled, ['delete-1']);
  });

  testWidgets('editing preserves provider identity and retained metadata', (
    tester,
  ) async {
    final repository = CalendarRepository();
    final workspace = Workspace(repository, MemorySettings());
    final remote = CalendarEntry(
      'remote',
      'Old',
      DateTime(2027, 1, 4, 10),
      DateTime(2027, 1, 4, 11),
      sourceId: 'primary',
      description: 'Retain',
      allDay: true,
      etag: '"v1"',
      remoteUrl: 'remote',
    );
    workspace.events = [remote];
    await tester.pumpWidget(
      MaterialApp(
        home: EventEditor(
          workspace: workspace,
          date: remote.start,
          entry: remote,
        ),
      ),
    );
    await tester.enterText(find.byType(TextField).first, 'Updated');
    await tester.tap(find.widgetWithText(FilledButton, 'Save event'));
    await tester.pump();
    expect(repository.saved.single.$1, remote);
    final after = repository.saved.single.$2;
    expect(
      (after.etag, after.remoteUrl, after.description, after.start),
      (remote.etag, remote.remoteUrl, remote.description, remote.start),
    );
    expect(after.allDay, isTrue);
  });

  testWidgets('new event saves the all-day choice shown in the editor', (
    tester,
  ) async {
    final repository = CalendarRepository();
    final workspace = Workspace(repository, MemorySettings());
    final date = DateTime(2027, 1, 4);
    await tester.pumpWidget(
      MaterialApp(
        home: EventEditor(workspace: workspace, date: date),
      ),
    );
    expect(find.text('4/1/2027 · All day'), findsOneWidget);
    await tester.enterText(find.byType(TextField).first, 'Day off');
    await tester.tap(find.widgetWithText(FilledButton, 'Save event'));
    await tester.pump();
    final after = repository.saved.single.$2;
    expect(after.allDay, isTrue);
    expect(after.start, date);
    expect(after.end, date.add(const Duration(days: 1)));
    expect(after.etag, isNull);
    expect(after.remoteUrl, isNull);
  });

  test(
    'projection keeps shared IDs separate and newest intent owns one event',
    () async {
      final first = entry('shared');
      final second = CalendarEntry(
        'shared',
        'Other source',
        first.start,
        first.end,
        sourceId: 'team',
      );
      final newer = CalendarEntry(
        'shared',
        'Newest',
        first.start,
        first.end,
        sourceId: 'primary',
      );
      final repository = CalendarRepository()
        ..calendarCache = [newer, second]
        ..activity = [
          CalendarActivity({
            'id': 'new',
            'status': 'waiting',
            'error': null,
            'created': 2,
            'mutation': {
              'save': {
                'before': first.toCalendarJson(),
                'after': newer.toCalendarJson(),
              },
            },
            'receipt': null,
          }),
          CalendarActivity(deleteActivity(first, created: 1)),
        ];
      final workspace = Workspace(repository, MemorySettings());
      await workspace.refreshCalendarActivity();
      expect(
        workspace.events.map((event) => '${event.sourceId}:${event.title}'),
        containsAll(['primary:Newest', 'team:Other source']),
      );
    },
  );

  test(
    'pending delete stays absent after reopen and empty cache clears stale event',
    () async {
      final remote = entry('remote');
      final repository = CalendarRepository()
        ..activity = [CalendarActivity(deleteActivity(remote))];
      final workspace = Workspace(repository, MemorySettings())
        ..events = [remote];
      await workspace.refreshCalendarActivity();
      expect(workspace.events, isEmpty);
      repository.activity = const [];
      workspace.events = [remote];
      await workspace.refreshCalendarActivity();
      expect(workspace.events, isEmpty);
    },
  );

  testWidgets(
    'recurring CalDAV event is view-only while a new event remains editable',
    (tester) async {
      final workspace = Workspace(CalendarRepository(), MemorySettings());
      addTearDown(workspace.dispose);
      final recurring = CalendarEntry(
        'series-instance',
        'Weekly review',
        DateTime(2026, 9, 21, 10),
        DateTime(2026, 9, 21, 11),
        sourceId: 'caldav-source',
        etag: '"strong-etag"',
        description: 'Recurring event',
      );

      await tester.pumpWidget(
        MaterialApp(
          home: EventEditor(
            workspace: workspace,
            date: recurring.start,
            entry: recurring,
          ),
        ),
      );

      expect(find.text('View event'), findsOneWidget);
      expect(
        find.textContaining('Recurring CalDAV events are view-only'),
        findsOneWidget,
      );
      expect(find.widgetWithText(FilledButton, 'Save event'), findsNothing);
      expect(find.widgetWithText(TextButton, 'Delete'), findsNothing);
      expect(
        tester.widget<TextField>(find.byType(TextField).first).readOnly,
        isTrue,
      );

      await tester.pumpWidget(
        MaterialApp(
          home: EventEditor(workspace: workspace, date: DateTime(2026, 9, 22)),
        ),
      );

      expect(find.text('New event'), findsOneWidget);
      expect(find.widgetWithText(FilledButton, 'Save event'), findsOneWidget);
      expect(
        tester.widget<TextField>(find.byType(TextField).first).readOnly,
        isFalse,
      );
    },
  );

  test(
    'CalDAV event uses its frozen connection and activity credential',
    () async {
      final repository = CalDavCalendarRepository();
      final remote = CalendarEntry(
        'remote',
        'Remote event',
        DateTime(2026, 9, 22, 10),
        DateTime(2026, 9, 22, 11),
        sourceId: repository.connection.id,
        etag: '"one"',
        remoteUrl: 'https://calendar.example.test/home/remote.ics',
      );
      repository.calendarCache = [remote];
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await workspace.refreshCalDavConnections();
      workspace.events = [remote];

      expect(await workspace.saveEvent(remote), isTrue);
      await Future<void>.delayed(Duration.zero);
      expect(repository.calDavExecuted, hasLength(1));

      final action = CalendarActivity({
        'id': 'uncertain-caldav',
        'status': 'uncertain',
        'error': 'The provider result needs checking.',
        'created': 1,
        'subject': null,
        'connection_id': repository.connection.id,
        'connection_revision': repository.connection.revision,
        'credential_slot': repository.connection.credentialSlot,
        'mutation': {
          'save': {
            'before': remote.toCalendarJson(),
            'after': remote.toCalendarJson(),
          },
        },
        'receipt': null,
      });
      await workspace.inspectCalendarActivity(action);
      expect(repository.calDavInspected, ['uncertain-caldav']);
    },
  );

  testWidgets('CalDAV controls durably admit before checking credentials', (
    tester,
  ) async {
    final repository = CalDavCalendarRepository();
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(body: CalDavConnectionsCard(workspace: workspace)),
      ),
    );
    await tester.pump();
    await tester.tap(find.text('Add calendar'));
    await tester.pump();
    await tester.enterText(
      find.widgetWithText(TextField, 'Calendar URL'),
      'https://calendar.example.test/home/',
    );
    await tester.enterText(find.widgetWithText(TextField, 'Username'), 'sam');
    await tester.enterText(
      find.widgetWithText(TextField, 'Password'),
      'private-password',
    );
    await tester.tap(find.widgetWithText(FilledButton, 'Connect'));
    await tester.pump();

    expect(repository.setupAttempts, hasLength(1));
    expect(repository.setupAttempts.single.connectionId, startsWith('caldav-'));
    expect(
      repository.setupAttempts.single.connectionId,
      isNot(contains('calendar.example.test')),
    );
    expect(repository.savedPasswords, ['private-password']);
  });

  test('late CalDAV refresh cannot restore stale connections', () async {
    final repository = CalDavCalendarRepository();
    final first = Completer<List<CalDavConnection>>();
    var reads = 0;
    repository.connectionsOverride = () {
      reads++;
      return reads == 1 ? first.future : Future.value(const []);
    };
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    final stale = workspace.refreshCalDavConnections();
    await Future<void>.delayed(Duration.zero);
    await workspace.refreshCalDavConnections();
    first.complete([repository.connection]);
    await stale;

    expect(workspace.calDavConnections, isEmpty);
  });

  test('disposed workspace does not resume a late CalDAV setup page', () async {
    final repository = CalDavCalendarRepository();
    final pending = Completer<List<CalDavAttempt>>();
    final attempt = await repository.admitCalDavConnection(
      attemptId: 'late-attempt',
      connectionId: 'late-calendar',
      url: 'https://calendar.example.test/late/',
      username: 'sam',
    );
    repository.attemptsOverride = (runnable) =>
        runnable ? pending.future : Future.value([attempt]);
    final workspace = Workspace(repository, MemorySettings());
    final refresh = workspace.refreshCalDavConnections(resume: true);
    await Future<void>.delayed(Duration.zero);
    workspace.dispose();
    pending.complete([attempt]);
    await refresh;

    expect(repository.activatedAttempts, isEmpty);
  });

  testWidgets('checked uncertain event offers explicit current-state adoption', (
    tester,
  ) async {
    final repository = CalendarRepository()
      ..activity = [
        CalendarActivity({
          ...activityData('uncertain'),
          'checked': true,
          'error':
              'The provider no longer matches the requested event. Keep this change for review until you explicitly accept the current state.',
        }),
      ];
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await workspace.refreshCalendarActivity();
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(body: CalendarView(workspace: workspace)),
      ),
    );
    await tester.pump();

    expect(find.widgetWithText(TextButton, 'Keep current'), findsOneWidget);
    await tester.tap(find.widgetWithText(TextButton, 'Keep current'));
    await tester.pump();
    expect(repository.acceptedCurrent, ['action']);
    expect(find.widgetWithText(TextButton, 'Keep current'), findsNothing);
  });

  testWidgets('compact dark calendar recovery visual evidence', (tester) async {
    await (FontLoader(
      'Roboto',
    )..addFont(rootBundle.load('assets/Roboto-Regular.ttf'))).load();
    await (FontLoader(
      'NotoSans',
    )..addFont(rootBundle.load('assets/NotoSans-Regular.ttf'))).load();
    tester.view.physicalSize = const Size(390, 700);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final repository = CalendarRepository()
      ..activity = [CalendarActivity(activityData('waiting'))];
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await tester.pumpWidget(
      MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: shepTheme(Brightness.light),
        darkTheme: shepTheme(Brightness.dark),
        themeMode: ThemeMode.dark,
        home: Scaffold(body: CalendarView(workspace: workspace)),
      ),
    );
    await tester.pump();
    expect(find.widgetWithText(TextButton, 'Retry'), findsOneWidget);
    expect(find.widgetWithText(TextButton, 'Cancel'), findsOneWidget);
    await expectLater(
      find.byType(MaterialApp),
      matchesGoldenFile('goldens/calendar_waiting_compact_dark.png'),
    );
  });

  testWidgets('compact light CalDAV credential re-entry visual evidence', (
    tester,
  ) async {
    await (FontLoader(
      'Roboto',
    )..addFont(rootBundle.load('assets/Roboto-Regular.ttf'))).load();
    await (FontLoader(
      'NotoSans',
    )..addFont(rootBundle.load('assets/NotoSans-Regular.ttf'))).load();
    tester.view.physicalSize = const Size(390, 700);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final repository = CalDavCalendarRepository();
    repository.setupAttempts.add(
      CalDavAttempt({
        'id': 'waiting-setup',
        'status': 'waiting',
        'error': 'Unlock device credential storage, then retry.',
        'request': {
          'connection': {
            'id': 'caldav-waiting',
            'url': 'https://calendar.example.test/home/',
            'username': 'sam',
          },
          'credential_slot': 'calendar-waiting-setup',
        },
      }),
    );
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await workspace.refreshCalDavConnections();
    await tester.pumpWidget(
      MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: shepTheme(Brightness.light),
        home: Scaffold(body: CalDavConnectionsCard(workspace: workspace)),
      ),
    );
    await tester.pump();
    await tester.tap(find.text('Enter password'));
    await tester.pump();
    expect(
      find.text('Unlock device credential storage, then retry.'),
      findsOneWidget,
    );
    expect(find.widgetWithText(FilledButton, 'Connect'), findsOneWidget);
    await expectLater(
      find.byType(MaterialApp),
      matchesGoldenFile('goldens/caldav_setup_compact_light.png'),
    );
  });

  testWidgets('compact dark checked event recovery visual evidence', (
    tester,
  ) async {
    await (FontLoader(
      'Roboto',
    )..addFont(rootBundle.load('assets/Roboto-Regular.ttf'))).load();
    await (FontLoader(
      'NotoSans',
    )..addFont(rootBundle.load('assets/NotoSans-Regular.ttf'))).load();
    tester.view.physicalSize = const Size(390, 700);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final recurring = CalendarEntry(
      'series-instance',
      'Weekly review',
      DateTime(2026, 9, 21, 10),
      DateTime(2026, 9, 21, 11),
      calendar: 'Team calendar',
      sourceId: 'caldav-team',
      etag: '"series-etag"',
      description: 'Recurring event',
    );
    final repository = CalendarRepository()
      ..calendarCache = [recurring]
      ..activity = [
        CalendarActivity({
          'id': 'checked-action',
          'status': 'uncertain',
          'error':
              'The provider no longer matches the requested event. Keep this change for review until you explicitly accept the current state.',
          'created': 1,
          'subject': null,
          'connection_id': 'caldav-team',
          'connection_revision': 2,
          'credential_slot': 'calendar-team',
          'checked': true,
          'mutation': {
            'save': {
              'before': recurring.toCalendarJson(),
              'after': recurring.toCalendarJson(),
            },
          },
          'receipt': null,
        }),
      ];
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await workspace.refreshCalendarActivity();
    await tester.pumpWidget(
      MaterialApp(
        debugShowCheckedModeBanner: false,
        theme: shepTheme(Brightness.light),
        darkTheme: shepTheme(Brightness.dark),
        themeMode: ThemeMode.dark,
        home: Scaffold(body: CalendarView(workspace: workspace)),
      ),
    );
    await tester.pump();
    expect(find.text('Keep current'), findsOneWidget);
    await expectLater(
      find.byType(MaterialApp),
      matchesGoldenFile('goldens/caldav_recovery_compact_dark.png'),
    );
    await tester.tap(find.text('Weekly review').last);
    await tester.pumpAndSettle();
    expect(
      find.textContaining('Recurring CalDAV events are view-only'),
      findsOneWidget,
    );
    expect(find.widgetWithText(FilledButton, 'Save event'), findsNothing);
    expect(find.widgetWithText(TextButton, 'Delete'), findsNothing);
    await expectLater(
      find.byType(MaterialApp),
      matchesGoldenFile('goldens/caldav_recurring_view_compact_dark.png'),
    );
  });

  testWidgets('CalDAV removal cleanup failure stays visible and retryable', (
    tester,
  ) async {
    final repository = CalDavCalendarRepository()
      ..failRemovalCleanup = true
      ..failCleanup = true;
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await workspace.refreshCalDavConnections();
    await workspace.removeCalDav(repository.connection);
    expect(
      workspace.calDavCleanupError,
      'Calendar was removed, but credential cleanup is waiting.',
    );
    expect(workspace.error, isNot(contains('private removal detail')));

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(body: CalDavConnectionsCard(workspace: workspace)),
      ),
    );
    await tester.pump();
    expect(find.text('Retry cleanup'), findsOneWidget);
    final initialCleanupCalls = repository.cleanupCalls;
    repository.failCleanup = true;
    await tester.tap(find.text('Retry cleanup'));
    await tester.pumpAndSettle();
    expect(
      workspace.error,
      'Unlock device credential storage, then retry cleanup.',
    );
    repository.failCleanup = false;
    await tester.tap(find.text('Retry cleanup'));
    await tester.pumpAndSettle();
    expect(repository.cleanupCalls, initialCleanupCalls + 2);
    expect(workspace.calDavCleanupError, isNull);
    expect(workspace.error, isNull);
  });

  test('CalDAV precommit refusal retains rows and unrelated errors', () async {
    final repository = CalDavCalendarRepository()..refuseRemoval = true;
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await workspace.refreshCalDavConnections();
    await workspace.removeCalDav(repository.connection);
    expect(workspace.calDavConnections, isNotEmpty);
    expect(workspace.calDavCleanupError, isNull);
    workspace.error = 'A newer unrelated failure';
    repository.refuseRemoval = false;
    await workspace.retryCalDavCleanup();
    expect(workspace.error, 'A newer unrelated failure');
  });

  testWidgets(
    'CalDAV cleanup resumes after workspace restart with visible recovery',
    (tester) async {
      final repository = CalDavCalendarRepository()..failCleanup = true;
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: ListenableBuilder(
              listenable: workspace,
              builder: (context, child) =>
                  CalDavConnectionsCard(workspace: workspace),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(repository.cleanupCalls, 1);
      expect(
        workspace.calDavCleanupError,
        'Unlock device credential storage, then retry cleanup.',
      );
      expect(find.text('Retry cleanup'), findsOneWidget);
      repository.failCleanup = false;
      await tester.tap(find.text('Retry cleanup'));
      await tester.pumpAndSettle();
      expect(repository.cleanupCalls, 2);
      expect(workspace.calDavCleanupError, isNull);
      expect(workspace.error, isNull);
      expect(find.text('Retry cleanup'), findsNothing);
    },
  );
}
