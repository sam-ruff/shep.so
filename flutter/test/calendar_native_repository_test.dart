import 'dart:io';

import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/credentials.dart';
import 'package:shep_mobile/data/native_repository.dart';
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
}
