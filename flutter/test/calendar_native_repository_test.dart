import 'dart:async';
import 'dart:io';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/credentials.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/data/accounts.dart';
import 'package:shep_mobile/data/repository.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/src/rust/frb_generated.dart';

class UnusedCredentials implements CredentialStore {
  @override
  Future<String?> read(String account, bool smtp) async => null;
  @override
  Future<void> remove(String account) async {}
  @override
  Future<void> save(String account, String incoming, String smtp) async {}
}

class MemoryCredentials implements CredentialStore {
  final values = <String, String>{};
  @override
  Future<String?> read(String account, bool smtp) async => values[account];
  @override
  Future<void> remove(String account) async => values.remove(account);
  @override
  Future<void> save(String account, String incoming, String smtp) async {
    values[account] = incoming;
  }
}

class HeldCredentials extends MemoryCredentials {
  final saveEntered = Completer<void>();
  final releaseSave = Completer<void>();
  bool failRemoval = false;

  @override
  Future<void> save(String account, String incoming, String smtp) async {
    if (!saveEntered.isCompleted) saveEntered.complete();
    await releaseSave.future;
    await super.save(account, incoming, smtp);
  }

  @override
  Future<void> remove(String account) async {
    if (failRemoval) throw StateError('credential store locked');
    await super.remove(account);
  }
}

class RefusingCredentials extends MemoryCredentials {
  bool refuseSave = true;
  bool refuseRead = false;
  bool refuseRemoval = false;

  @override
  Future<String?> read(String account, bool smtp) async {
    if (refuseRead) throw StateError('private-password-in-read-error');
    return super.read(account, smtp);
  }

  @override
  Future<void> remove(String account) async {
    if (refuseRemoval) throw StateError('private-password-in-remove-error');
    await super.remove(account);
  }

  @override
  Future<void> save(String account, String incoming, String smtp) async {
    if (refuseSave) throw StateError('private-password-in-provider-error');
    await super.save(account, incoming, smtp);
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    final name = Platform.isWindows
        ? 'shep_mobile_native.dll'
        : Platform.isMacOS
        ? 'libshep_mobile_native.dylib'
        : 'libshep_mobile_native.so';
    await ShepNative.init(
      externalLibrary: ExternalLibrary.open(
        'build/native_assets/${Platform.operatingSystem}/$name',
      ),
    );
  });

  test(
    'calendar admission is durable through the actual FFI and cancel',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-calendar-host-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final repository = await NativeRepository.open(
        '${directory.path}/mail.sqlite',
        credentials: UnusedCredentials(),
      );
      final seeded = await Process.run('sqlite3', [
        '${directory.path}/mail.sqlite',
        '''INSERT INTO calendar_sources(id,source) VALUES('primary','{"id":"primary","name":"Personal","read_only":false}'); INSERT INTO calendar_binding(id,subject) VALUES(1,'fixture-google-user');''',
      ]);
      expect(seeded.exitCode, 0, reason: '${seeded.stderr}');
      final event = CalendarEntry(
        'local-event',
        'Review',
        DateTime.utc(2027, 1, 4, 10),
        DateTime.utc(2027, 1, 4, 11),
        sourceId: 'primary',
      );

      final admitted = await repository.admitCalendarAction(
        'calendar-action',
        event,
        null,
        subject: 'fixture-google-user',
      );
      expect(admitted.status, 'queued');
      expect(admitted.saved, isNull);
      final reconciled = await repository.admitCalendarAction(
        'calendar-action',
        event,
        null,
        subject: 'fixture-google-user',
      );
      expect(reconciled.status, 'queued');
      expect(
        (await repository.calendarActionAdmission('calendar-action'))?.id,
        'calendar-action',
      );
      await expectLater(
        repository.admitCalendarAction(
          'calendar-action',
          CalendarEntry(
            event.id,
            'Changed request',
            event.start,
            event.end,
            sourceId: event.sourceId,
          ),
          null,
          subject: 'fixture-google-user',
        ),
        throwsA(anything),
      );
      var actions = await repository.calendarActions();
      expect(actions, hasLength(1));
      expect(actions.single.status, 'queued');
      expect(actions.single.requested.title, 'Review');
      expect(actions.single.subject, 'fixture-google-user');
      expect(
        (await repository.calendarSnapshot()).subject,
        'fixture-google-user',
      );
      await expectLater(
        repository.admitCalendarAction(
          'wrong-owner',
          event,
          null,
          subject: 'replacement-subject',
        ),
        throwsA(anything),
      );

      await repository.waitCalendarAction(
        'calendar-action',
        'Connect Google Calendar',
      );
      actions = await repository.calendarActions();
      expect(actions.single.status, 'waiting');
      expect(actions.single.error, 'Connect Google Calendar');

      await repository.cancelCalendarAction('calendar-action');
      actions = await repository.calendarActions();
      expect(actions, isEmpty);
      expect(await repository.calendarEvents(), isEmpty);
    },
  );

  test(
    'durable delete decodes and remains projected after reopen through actual FFI',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-calendar-delete-host-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final path = '${directory.path}/mail.sqlite';
      var repository = await NativeRepository.open(
        path,
        credentials: UnusedCredentials(),
      );
      final source = '{"id":"primary","name":"Personal","read_only":false}';
      final event =
          '{"id":"shared","source_id":"primary","title":"Review","start":"2027-01-04T10:00:00Z","end":"2027-01-04T11:00:00Z","location":"","description":"","all_day":false,"etag":"v1","remote_url":"shared"}';
      final seeded = await Process.run('sqlite3', [
        path,
        "INSERT INTO calendar_sources(id,source) VALUES('primary','$source'); INSERT INTO calendar_events(source_id,id,event) VALUES('primary','shared','$event'); INSERT INTO calendar_binding(id,subject) VALUES(1,'fixture-google-user');",
      ]);
      expect(seeded.exitCode, 0, reason: '${seeded.stderr}');
      final cached = (await repository.calendarEvents()).single;
      expect(cached.readOnly, isFalse);
      await repository.admitCalendarDelete(
        'delete-action',
        cached,
        subject: 'fixture-google-user',
      );
      var actions = await repository.calendarActions();
      expect(actions.single.isDelete, isTrue);
      expect(actions.single.requested.id, 'shared');
      expect(actions.single.subject, 'fixture-google-user');
      expect((await repository.calendarSnapshot()).events, isEmpty);
      repository = await NativeRepository.open(
        path,
        credentials: UnusedCredentials(),
      );
      actions = await repository.calendarActions();
      expect(actions.single.isDelete, isTrue);
      expect((await repository.calendarSnapshot()).events, isEmpty);
      await repository.cancelCalendarAction('delete-action');
      expect((await repository.calendarSnapshot()).events.single.id, 'shared');
    },
  );

  test('CalDAV admission retains connection identity through actual FFI', () async {
    final directory = await Directory.systemTemp.createTemp(
      'shep-caldav-action-host-',
    );
    addTearDown(() => directory.delete(recursive: true));
    final path = '${directory.path}/mail.sqlite';
    final repository = await NativeRepository.open(
      path,
      credentials: UnusedCredentials(),
    );
    final config =
        '{"id":"caldav-home","url":"https://calendar.example.test/home/","username":"sam"}';
    final source = '{"id":"caldav-home","name":"Home","read_only":false}';
    final seeded = await Process.run('sqlite3', [
      path,
      "INSERT INTO calendar_connections(id,config,credential_slot,revision) VALUES('caldav-home','$config','calendar-slot',4); INSERT INTO calendar_sources(id,source,connection_id) VALUES('caldav-home','$source','caldav-home');",
    ]);
    expect(seeded.exitCode, 0, reason: '${seeded.stderr}');
    final connection = (await repository.calDavConnections()).single;
    final event = CalendarEntry(
      'reserved-event',
      'Review',
      DateTime.utc(2027, 1, 4, 10),
      DateTime.utc(2027, 1, 4, 11),
      sourceId: 'caldav-home',
    );

    final admitted = await repository.admitCalDavAction(
      'caldav-action',
      event,
      null,
      connection,
    );
    expect(admitted.status, 'queued');
    expect(admitted.connectionId, 'caldav-home');
    expect(admitted.connectionRevision, 4);
    expect(admitted.credentialSlot, 'calendar-slot');
    final activity = (await repository.calendarActions()).single;
    expect(activity.isCalDav, isTrue);
    expect(activity.connectionId, 'caldav-home');
    expect(activity.connectionRevision, 4);
    expect(activity.credentialSlot, 'calendar-slot');
    expect((await repository.calendarSnapshot()).events.single.id, event.id);
  });

  test(
    'CalDAV setup and credential cleanup use the durable FFI journal',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-caldav-setup-host-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final credentials = MemoryCredentials();
      final repository = await NativeRepository.open(
        '${directory.path}/mail.sqlite',
        credentials: credentials,
      );

      final attempt = await repository.admitCalDavConnection(
        attemptId: 'setup-action',
        connectionId: 'caldav-reserved',
        url: 'https://calendar.example.test/home/',
        username: 'sam',
      );
      expect(attempt.status, 'prepared');
      expect(
        (await repository.calDavAttempts(pending: true)).single.id,
        attempt.id,
      );
      await repository.saveCalDavPassword(attempt, 'secret');
      expect(credentials.values[attempt.credentialSlot], 'secret');

      await repository.cancelCalDavConnection(attempt);
      expect(credentials.values, isEmpty);
      expect(await repository.calDavAttempts(pending: true), isEmpty);
    },
  );

  test(
    'CalDAV credential failures retain only a public durable error',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-caldav-credential-failure-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final repository = await NativeRepository.open(
        '${directory.path}/mail.sqlite',
        credentials: RefusingCredentials(),
      );
      final attempt = await repository.admitCalDavConnection(
        attemptId: 'refused-save',
        connectionId: 'caldav-refused',
        url: 'https://calendar.example.test/refused/',
        username: 'sam',
      );

      await expectLater(
        repository.saveCalDavPassword(attempt, 'private-password'),
        throwsA(anything),
      );

      final saved = await repository.calDavAttempt(attempt.id);
      expect(saved?.status, 'waiting');
      expect(saved?.error, 'Unlock device credential storage, then retry.');
      expect(saved?.error, isNot(contains('private-password')));
      expect(saved?.error, isNot(contains('provider-error')));
    },
  );

  test('CalDAV credential read and cleanup failures remain public', () async {
    final directory = await Directory.systemTemp.createTemp(
      'shep-caldav-private-credential-errors-',
    );
    addTearDown(() => directory.delete(recursive: true));
    final credentials = RefusingCredentials();
    credentials.refuseSave = false;
    final repository = await NativeRepository.open(
      '${directory.path}/mail.sqlite',
      credentials: credentials,
    );
    final attempt = await repository.admitCalDavConnection(
      attemptId: 'private-errors',
      connectionId: 'caldav-private-errors',
      url: 'https://calendar.example.test/private-errors/',
      username: 'sam',
    );
    await repository.saveCalDavPassword(attempt, 'private-password');
    credentials.refuseRead = true;

    await expectLater(
      repository.activateCalDavConnection(attempt),
      throwsA(
        isA<MailOperationFailure>().having(
          (failure) => failure.message,
          'message',
          'Unlock device credential storage, then retry.',
        ),
      ),
    );
    expect(
      (await repository.calDavAttempt(attempt.id))?.error,
      'Unlock device credential storage, then retry.',
    );

    credentials.refuseRead = false;
    credentials.refuseRemoval = true;
    await expectLater(
      repository.cancelCalDavConnection(attempt),
      throwsA(
        isA<CalendarCredentialCleanupFailure>()
            .having(
              (failure) => failure.message,
              'message',
              contains('cleanup'),
            )
            .having(
              (failure) => failure.message,
              'private detail',
              isNot(contains('private-password')),
            ),
      ),
    );
  });

  test(
    'CalDAV credential staging and cancellation share the lifecycle FIFO',
    () async {
      final directory = await Directory.systemTemp.createTemp(
        'shep-caldav-held-credential-',
      );
      addTearDown(() => directory.delete(recursive: true));
      final path = '${directory.path}/mail.sqlite';
      final credentials = HeldCredentials();
      final first = await NativeRepository.open(path, credentials: credentials);
      final second = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      final attempt = await first.admitCalDavConnection(
        attemptId: 'held-save',
        connectionId: 'caldav-held',
        url: 'https://calendar.example.test/held/',
        username: 'sam',
      );
      final saving = first.saveCalDavPassword(attempt, 'secret');
      await credentials.saveEntered.future;
      var cancelled = false;
      final cancelling = second
          .cancelCalDavConnection(attempt)
          .then((_) => cancelled = true);
      await Future<void>.delayed(Duration.zero);
      expect(cancelled, isFalse);

      credentials.releaseSave.complete();
      await saving;
      await cancelling;
      expect(credentials.values, isEmpty);
      expect(await first.calDavAttempts(pending: true), isEmpty);
    },
  );

  test('failed CalDAV credential cleanup remains durably retryable', () async {
    final directory = await Directory.systemTemp.createTemp(
      'shep-caldav-cleanup-failure-',
    );
    addTearDown(() => directory.delete(recursive: true));
    final path = '${directory.path}/mail.sqlite';
    final credentials = MemoryCredentials();
    final repository = await NativeRepository.open(
      path,
      credentials: credentials,
    );
    final attempt = await repository.admitCalDavConnection(
      attemptId: 'cleanup-failure',
      connectionId: 'caldav-cleanup',
      url: 'https://calendar.example.test/cleanup/',
      username: 'sam',
    );
    await repository.saveCalDavPassword(attempt, 'secret');
    final failing = HeldCredentials()..failRemoval = true;
    failing.releaseSave.complete();
    failing.values.addAll(credentials.values);
    final second = await NativeRepository.open(path, credentials: failing);

    await expectLater(
      second.cancelCalDavConnection(attempt),
      throwsA(anything),
    );
    final result = await Process.run('sqlite3', [
      path,
      'SELECT count(*) FROM calendar_credential_cleanup;',
    ]);
    expect(result.exitCode, 0, reason: '${result.stderr}');
    expect('${result.stdout}'.trim(), '1');
  });
}
