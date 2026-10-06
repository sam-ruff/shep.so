import 'dart:async';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/repository.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/calendar.dart';
import 'package:shep_mobile/ui/theme.dart';
import 'calendar_lifecycle_test.dart'
    show CalendarRepository, MemorySettings, admission;
import 'mail_activity_visual_test.dart' show loadPreviewFonts;

class EditorRepository extends CalendarRepository {
  List<CalendarSource> sources = const [
    CalendarSource('primary', 'Personal', false),
  ];
  @override
  Future<CalendarSnapshot> calendarSnapshot() async =>
      CalendarSnapshot(sources, calendarCache, subject: calendarSubject);
}

Future<Workspace> openEditor(
  WidgetTester tester,
  EditorRepository repository, {
  CalendarEntry? entry,
  Brightness brightness = Brightness.light,
  DateTime? date,
}) async {
  final workspace = Workspace(repository, MemorySettings());
  workspace.calendarSources = repository.sources;
  workspace.events = entry == null ? [] : [entry];
  await tester.pumpWidget(
    MaterialApp(
      theme: shepTheme(brightness),
      debugShowCheckedModeBanner: false,
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(context).copyWith(alwaysUse24HourFormat: true),
        child: child!,
      ),
      home: Builder(
        builder: (context) => Scaffold(
          body: Center(
            child: TextButton(
              onPressed: () => showDialog<void>(
                context: context,
                builder: (_) => EventEditor(
                  workspace: workspace,
                  date: date ?? DateTime(2026, 3, 28),
                  entry: entry,
                ),
              ),
              child: const Text('Open event'),
            ),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open event'));
  await tester.pumpAndSettle();
  return workspace;
}

Future<void> chooseDate(WidgetTester tester, String label, int day) async {
  final button = find.widgetWithText(OutlinedButton, label);
  await tester.ensureVisible(button);
  await tester.tap(button);
  await tester.pumpAndSettle();
  expect(find.byType(DatePickerDialog), findsOneWidget);
  await tester.tap(find.text('$day').last);
  await tester.pump();
  await tester.tap(find.text('OK').last);
  await tester.pumpAndSettle();
}

Future<void> chooseTime(
  WidgetTester tester,
  String label,
  String hour,
  String minute,
) async {
  final button = find.widgetWithText(OutlinedButton, label);
  await tester.ensureVisible(button);
  await tester.tap(button);
  await tester.pumpAndSettle();
  await tester.tap(find.byTooltip('Switch to text input mode'));
  await tester.pumpAndSettle();
  final fields = find.descendant(
    of: find.byType(TimePickerDialog),
    matching: find.byType(TextField),
  );
  await tester.enterText(fields.at(0), hour);
  await tester.enterText(fields.at(1), minute);
  await tester.tap(find.text('OK').last);
  await tester.pumpAndSettle();
}

Future<void> save(WidgetTester tester) async {
  await tester.tap(find.widgetWithText(FilledButton, 'Save event'));
  await tester.pump();
}

Future<void> close(WidgetTester tester, Workspace workspace) async {
  await tester.pumpWidget(const SizedBox.shrink());
  workspace.dispose();
}

void main() {
  testWidgets(
    'spring-DST input remains visible and cannot admit a normalised time',
    (tester) async {
      final zone = Platform.environment['TZ'];
      final date = zone == 'Europe/London'
          ? DateTime(2026, 3, 29)
          : DateTime(2026, 3, 8);
      final hour = zone == 'Europe/London' ? '01' : '02';
      final repo = EditorRepository();
      final workspace = await openEditor(tester, repo, date: date);
      await tester.enterText(
        find.widgetWithText(TextField, 'Event title'),
        'Gap',
      );
      await tester.tap(find.byType(SwitchListTile));
      await tester.pumpAndSettle();
      await chooseTime(tester, 'From', hour, '30');
      await chooseTime(tester, 'To', '04', '00');
      await save(tester);
      expect(
        find.text(
          'This time is unavailable on these dates. Choose another time.',
        ),
        findsOneWidget,
      );
      expect(repo.admitted, isEmpty);
      await close(tester, workspace);
    },
    skip: !const {
      'Europe/London',
      'America/Los_Angeles',
    }.contains(Platform.environment['TZ']),
  );
  testWidgets('native date/time controls save a timed multi-day event', (
    tester,
  ) async {
    final repo = EditorRepository();
    final actual = await openEditor(tester, repo);
    await tester.enterText(
      find.widgetWithText(TextField, 'Event title'),
      'Trip',
    );
    await tester.tap(find.byType(SwitchListTile));
    await tester.pumpAndSettle();
    await chooseDate(tester, 'Start date', 29);
    await chooseDate(tester, 'Last date', 30);
    await chooseTime(tester, 'From', '14', '15');
    await chooseTime(tester, 'To', '16', '45');
    await tester.enterText(
      find.widgetWithText(TextField, 'Location'),
      'Office',
    );
    await save(tester);
    final event = repo.saved.single.$2;
    expect(event.allDay, isFalse);
    expect(event.start, DateTime(2026, 3, 29, 14, 15));
    expect(event.end, DateTime(2026, 3, 30, 16, 45));
    expect(event.location, 'Office');
    expect(event.sourceId, 'primary');
    await close(tester, actual);
  });

  testWidgets('all-day Last date produces an exclusive multi-day end', (
    tester,
  ) async {
    final repo = EditorRepository();
    final workspace = await openEditor(tester, repo);
    await tester.enterText(
      find.widgetWithText(TextField, 'Event title'),
      'Days off',
    );
    await chooseDate(tester, 'Last date', 30);
    await save(tester);
    final event = repo.saved.single.$2;
    expect(event.allDay, isTrue);
    expect(event.start, DateTime.utc(2026, 3, 28));
    expect(event.end, DateTime.utc(2026, 3, 31));
    await close(tester, workspace);
  });

  testWidgets(
    'invalid timed end offers validation and never admits a request',
    (tester) async {
      final repo = EditorRepository();
      final workspace = await openEditor(tester, repo);
      await tester.enterText(
        find.widgetWithText(TextField, 'Event title'),
        'Review',
      );
      await tester.tap(find.byType(SwitchListTile));
      await tester.pumpAndSettle();
      await chooseTime(tester, 'To', '08', '00');
      await save(tester);
      expect(find.text('The event must end after it starts.'), findsOneWidget);
      expect(repo.admitted, isEmpty);
      await chooseTime(tester, 'To', '10', '30');
      await save(tester);
      expect(repo.admitted, hasLength(1));
      await close(tester, workspace);
    },
  );

  testWidgets(
    'editing scheduling retains provider identity and untouched description',
    (tester) async {
      final repo = EditorRepository();
      final original = CalendarEntry(
        'remote',
        'Review',
        DateTime(2026, 3, 28, 10),
        DateTime(2026, 3, 28, 11),
        sourceId: 'primary',
        description: 'Keep attendees and description',
        etag: '"v1"',
        remoteUrl: 'https://calendar.example.test/remote.ics',
      );
      final workspace = await openEditor(tester, repo, entry: original);
      await chooseDate(tester, 'Last date', 29);
      await chooseTime(tester, 'To', '12', '30');
      await save(tester);
      final (before, after) = repo.saved.single;
      expect(before, original);
      expect(
        (
          after.id,
          after.sourceId,
          after.etag,
          after.remoteUrl,
          after.description,
        ),
        (
          original.id,
          original.sourceId,
          original.etag,
          original.remoteUrl,
          original.description,
        ),
      );
      expect(after.end, DateTime(2026, 3, 29, 12, 30));
      await close(tester, workspace);
    },
  );

  testWidgets(
    'non-midnight legacy all-day title edit preserves exact wire times',
    (tester) async {
      final original = CalendarEntry.fromCalendarJson({
        'id': 'legacy',
        'source_id': 'primary',
        'title': 'Legacy',
        'start': '2026-07-01T23:00:00.000Z',
        'end': '2026-07-03T23:00:00.000Z',
        'all_day': true,
        'description': 'Retain',
        'etag': '"v1"',
        'remote_url': 'remote',
      });
      final repo = EditorRepository();
      final actual = await openEditor(tester, repo, entry: original);
      await tester.enterText(
        find.widgetWithText(TextField, 'Event title'),
        'Updated',
      );
      await save(tester);
      final after = repo.saved.single.$2.toCalendarJson(),
          before = original.toCalendarJson();
      expect(after['start'], before['start']);
      expect(after['end'], before['end']);
      expect(after['description'], before['description']);
      await close(tester, actual);
    },
  );

  testWidgets(
    'read-only sources and permission loss disable schedule and write controls',
    (tester) async {
      final repo = EditorRepository();
      final workspace = await openEditor(tester, repo);
      repo.sources = const [CalendarSource('primary', 'Personal', true)];
      await workspace.refreshCalendarActivity();
      await tester.pump();
      expect(find.widgetWithText(FilledButton, 'Save event'), findsNothing);
      expect(
        tester.widget<SwitchListTile>(find.byType(SwitchListTile)).onChanged,
        isNull,
      );
      expect(
        tester
            .widget<OutlinedButton>(
              find.widgetWithText(OutlinedButton, 'Start date'),
            )
            .onPressed,
        isNull,
      );
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Event title'))
            .readOnly,
        isTrue,
      );
      expect(repo.admitted, isEmpty);
      await close(tester, workspace);
    },
  );

  testWidgets(
    'held admission freezes schedule and retains newer input after completion',
    (tester) async {
      final repo = EditorRepository(), gate = Completer<CalendarAdmission>();
      CalendarEntry? frozen;
      String? identity;
      repo.admitOverride = (id, event, before, subject) async {
        identity = id;
        frozen = event;
        return gate.future;
      };
      final workspace = await openEditor(tester, repo);
      await tester.enterText(
        find.widgetWithText(TextField, 'Event title'),
        'Original',
      );
      await save(tester);
      expect(frozen?.end, DateTime.utc(2026, 3, 29));
      await chooseDate(tester, 'Last date', 30);
      gate.complete(admission(identity!, frozen!));
      await tester.pumpAndSettle();
      expect(find.byType(EventEditor), findsOneWidget);
      expect(find.text('Original'), findsOneWidget);
      expect(find.text('30/3/2026'), findsOneWidget);
      expect(
        find.text(
          'The earlier version was queued. Your newer edits are still open.',
        ),
        findsOneWidget,
      );
      expect(frozen?.end, DateTime.utc(2026, 3, 29));
      await close(tester, workspace);
    },
  );

  testWidgets(
    'lost admission fixes its calendar while newer input cannot create on another writable source',
    (tester) async {
      final repo = EditorRepository()
        ..sources = const [
          CalendarSource('primary', 'Personal', false),
          CalendarSource('team', 'Work', false),
        ];
      final requests = <(String, CalendarEntry)>[];
      repo.admitOverride = (id, event, before, subject) async {
        requests.add((id, event));
        throw StateError('lost response');
      };
      repo.admissionOverride = (id) async {
        if (requests.isEmpty) return null;
        throw StateError('lookup unavailable');
      };
      final workspace = await openEditor(tester, repo);
      await tester.enterText(
        find.widgetWithText(TextField, 'Event title'),
        'Original',
      );
      await save(tester);
      await tester.pumpAndSettle();
      final calendar = find.byType(DropdownButtonFormField<String>);
      expect(
        tester.widget<DropdownButtonFormField<String>>(calendar).onChanged,
        isNull,
      );
      await tester.tap(calendar);
      await tester.pumpAndSettle();
      expect(
        tester.widget<DropdownButtonFormField<String>>(calendar).initialValue,
        'primary',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Event title'),
        'Newer',
      );
      await chooseDate(tester, 'Last date', 30);
      await save(tester);
      await tester.pumpAndSettle();
      expect(requests, hasLength(1));
      expect(requests.single.$2.sourceId, 'primary');
      expect(find.text('Newer'), findsOneWidget);
      await close(tester, workspace);
    },
  );

  testWidgets(
    'unknown admission retry uses the same frozen request and changed input cannot mint a create',
    (tester) async {
      final repo = EditorRepository();
      final requests = <(String, CalendarEntry)>[];
      var known = false;
      repo.admitOverride = (id, event, before, subject) async {
        requests.add((id, event));
        throw StateError('lost admission response');
      };
      repo.admissionOverride = (id) async {
        if (requests.isEmpty) return null;
        if (!known) throw StateError('held inspection');
        return admission(requests.single.$1, requests.single.$2);
      };
      final workspace = await openEditor(tester, repo);
      await tester.enterText(
        find.widgetWithText(TextField, 'Event title'),
        'Original',
      );
      await save(tester);
      await tester.pumpAndSettle();
      expect(requests, hasLength(1));
      await tester.enterText(
        find.widgetWithText(TextField, 'Event title'),
        'Newer',
      );
      await chooseDate(tester, 'Last date', 30);
      await save(tester);
      await tester.pumpAndSettle();
      expect(requests, hasLength(1));
      await tester.enterText(
        find.widgetWithText(TextField, 'Event title'),
        'Original',
      );
      await chooseDate(tester, 'Last date', 28);
      known = true;
      await save(tester);
      await tester.pumpAndSettle();
      expect(requests, hasLength(1));
      expect(find.byType(EventEditor), findsNothing);
      await close(tester, workspace);
    },
  );

  testWidgets(
    'Close during held admission keeps the original event request reachable',
    (tester) async {
      final repo = EditorRepository(), gate = Completer<CalendarAdmission>();
      String? id;
      CalendarEntry? requested;
      repo.admitOverride = (identity, event, before, subject) async {
        id = identity;
        requested = event;
        return gate.future;
      };
      final workspace = await openEditor(tester, repo);
      await tester.enterText(
        find.widgetWithText(TextField, 'Event title'),
        'Keep queued',
      );
      await chooseDate(tester, 'Last date', 30);
      await save(tester);
      await tester.tap(find.text('Close'));
      await tester.pumpAndSettle();
      expect(find.byType(EventEditor), findsNothing);
      final known = admission(id!, requested!);
      repo.admissions[id!] = known;
      repo.calendarCache = [requested!];
      gate.complete(known);
      await tester.pumpAndSettle();
      expect(repo.admissions.keys, [id]);
      expect(workspace.events.single.end, DateTime.utc(2026, 3, 31));
      await close(tester, workspace);
    },
  );

  for (final brightness in Brightness.values) {
    testWidgets('compact timed scheduling controls ${brightness.name}', (
      tester,
    ) async {
      await loadPreviewFonts();
      tester.view.physicalSize = const Size(390, 760);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final repo = EditorRepository();
      final workspace = await openEditor(tester, repo, brightness: brightness);
      await tester.enterText(
        find.widgetWithText(TextField, 'Event title'),
        'Planning review',
      );
      await tester.tap(find.byType(SwitchListTile));
      await tester.pumpAndSettle();
      await expectLater(
        find.byType(MaterialApp),
        matchesGoldenFile(
          'goldens/calendar_schedule_compact_${brightness.name}.png',
        ),
      );
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(repo.admitted, isEmpty);
      await close(tester, workspace);
    });
  }
}
