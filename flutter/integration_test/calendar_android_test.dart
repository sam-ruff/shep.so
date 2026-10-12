import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/data/repository.dart';
import 'package:shep_mobile/ui/calendar.dart';
import 'package:shep_mobile/ui/theme.dart';
import '../test/calendar_native_repository_test.dart' show UnusedCredentials;
import '../test/calendar_lifecycle_test.dart' show MemorySettings;
import '../test/event_editor_test.dart' show chooseDate, chooseTime, save;

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  Future<CalendarActivity> admitted(
    WidgetTester tester,
    NativeRepository repository,
    Workspace workspace,
    String title,
  ) async {
    final deadline = DateTime.now().add(const Duration(seconds: 60));
    while (true) {
      final actions = await repository.calendarActions();
      final action = actions
          .where((action) => action.requested.title == title)
          .firstOrNull;
      final open = find.byType(EventEditor).evaluate().isNotEmpty;
      if (action != null && !open) {
        await tester.pumpAndSettle();
        return action;
      }
      final saveButton = find.widgetWithText(FilledButton, 'Save event');
      final failed =
          open &&
          saveButton.evaluate().isNotEmpty &&
          tester.widget<FilledButton>(saveButton).onPressed != null;
      if (failed || DateTime.now().isAfter(deadline)) {
        final visible = tester
            .widgetList<Text>(find.byType(Text))
            .map((text) => text.data ?? text.textSpan?.toPlainText() ?? '')
            .join('\n');
        await binding.takeScreenshot('calendar-save-outcome-failure');
        fail(
          'Calendar save did not reach its admitted/closed outcome for $title. '
          'Journal row: ${action?.status}. Workspace error: ${workspace.error}. UI:\n$visible',
        );
      }
      await tester.pump(const Duration(milliseconds: 100));
    }
  }

  testWidgets(
    'Android schedule controls retain native journal dates and requests through reopening',
    (tester) async {
      final support = await getApplicationSupportDirectory();
      final request = File('${support.path}/shep-calendar-request');
      final fixture = File('${support.path}/shep-calendar-fixture.sqlite');
      await request.writeAsString('waiting-before-profile-open', flush: true);
      final deadline = DateTime.now().add(const Duration(seconds: 60));
      while (!await fixture.exists()) {
        if (DateTime.now().isAfter(deadline)) {
          fail('Run the saved android_calendar_fixture.py helper');
        }
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      final directory = await Directory.systemTemp.createTemp(
        'calendar-controls-',
      );
      final path = '${directory.path}/mail.sqlite';
      await fixture.copy(path);
      var repository = await NativeRepository.open(
        path,
        credentials: UnusedCredentials(),
      );
      addTearDown(() {
        if (!repository.profile.isDisposed) repository.profile.dispose();
      });
      final workspace = Workspace(repository, MemorySettings());
      await workspace.refreshCalendarActivity();
      final completed = <String>[];
      var converted = false;
      for (final brightness in Brightness.values) {
        await tester.pumpWidget(
          MaterialApp(
            key: UniqueKey(),
            theme: shepTheme(brightness),
            builder: (context, child) => MediaQuery(
              data: MediaQuery.of(
                context,
              ).copyWith(alwaysUse24HourFormat: true),
              child: child!,
            ),
            home: Builder(
              builder: (context) => Scaffold(
                body: TextButton(
                  onPressed: () => showDialog<void>(
                    context: context,
                    builder: (_) => EventEditor(
                      workspace: workspace,
                      date: DateTime(2026, 3, 28),
                    ),
                  ),
                  child: const Text('New event'),
                ),
              ),
            ),
          ),
        );
        await tester.tap(find.text('New event'));
        await tester.pumpAndSettle();
        await tester.enterText(
          find.widgetWithText(TextField, 'Event title'),
          'Timed ${brightness.name}',
        );
        await tester.tap(find.byType(SwitchListTile));
        await tester.pumpAndSettle();
        await chooseDate(tester, 'Last date', 30);
        await chooseTime(tester, 'From', '14', '15');
        await chooseTime(tester, 'To', '16', '45');
        if (Platform.isAndroid && !converted) {
          await binding.convertFlutterSurfaceToImage();
          converted = true;
        }
        await tester.pumpAndSettle();
        await binding.takeScreenshot('calendar-timed-${brightness.name}');
        await save(tester);
        await tester.pumpAndSettle();
        final timed = await admitted(
          tester,
          repository,
          workspace,
          'Timed ${brightness.name}',
        );
        expect(timed.requested.start, DateTime(2026, 3, 28, 14, 15));
        expect(timed.requested.end, DateTime(2026, 3, 30, 16, 45));
        expect(timed.requested.allDay, isFalse);
        completed.add('timed-${brightness.name}');
        await tester.tap(find.text('New event'));
        await tester.pumpAndSettle();
        await tester.enterText(
          find.widgetWithText(TextField, 'Event title'),
          'Days ${brightness.name}',
        );
        await chooseDate(tester, 'Last date', 30);
        await save(tester);
        await tester.pumpAndSettle();
        final days = await admitted(
          tester,
          repository,
          workspace,
          'Days ${brightness.name}',
        );
        expect(days.requested.start, DateTime.utc(2026, 3, 28));
        expect(days.requested.end, DateTime.utc(2026, 3, 31));
        expect(days.requested.allDay, isTrue);
        completed.add('all-day-${brightness.name}');
      }
      final saved = {
        for (final action in await repository.calendarActions())
          action.id: action.data['mutation'],
      };
      await workspace.refreshCalendarActivity();
      await tester.pumpWidget(const SizedBox.shrink());
      workspace.dispose();
      repository.profile.dispose();
      repository = await NativeRepository.open(
        path,
        credentials: UnusedCredentials(),
      );
      expect({
        for (final action in await repository.calendarActions())
          action.id: action.data['mutation'],
      }, saved);
      completed.add('native-reopen');
      binding.reportData = {
        ...?binding.reportData,
        'calendar_native': completed,
      };
      await request.writeAsString('done', flush: true);
      repository.profile.dispose();
      await fixture.delete();
      await directory.delete(recursive: true);
    },
  );
}
