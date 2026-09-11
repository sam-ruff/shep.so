import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import '../test/native_repository_test.dart' show FixtureCredentials;
import '../test/support/bulk_controls_scenario.dart';
import '../test/workspace_test.dart' show MemorySettings;

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  final completed = <String>[];
  var surfaceConverted = false;
  setUp(() => surfaceConverted = false);
  Future<void> capture(WidgetTester tester, String name) async {
    if (Platform.isAndroid && !surfaceConverted) {
      await binding.convertFlutterSurfaceToImage();
      surfaceConverted = true;
    }
    await tester.pumpAndSettle();
    await binding.takeScreenshot(name);
  }

  Future<void> wait(
    WidgetTester tester,
    bool Function() ready,
    String what,
  ) async {
    final end = DateTime.now().add(const Duration(seconds: 60));
    while (!ready()) {
      if (DateTime.now().isAfter(end)) {
        fail('Native group journal did not reach $what');
      }
      await tester.pump(const Duration(milliseconds: 100));
    }
    await tester.pumpAndSettle();
  }

  Future<void> tap(WidgetTester tester, Finder finder) async {
    await tester.ensureVisible(finder);
    await tester.pumpAndSettle();
    await tester.tap(finder);
    await tester.pumpAndSettle();
  }

  for (final dark in [false, true]) {
    final scheme = dark ? 'dark' : 'light';
    testWidgets('native group controls with a synthetic journal ($scheme)', (
      tester,
    ) async {
      await bulkControlsScenario(
        tester,
        dark: dark,
        capture: (name) => capture(tester, 'native-$name'),
      );
      completed.add('bulk-controls-$scheme');
      binding.reportData = {...?binding.reportData, 'bulk_native': completed};
    });
  }

  testWidgets(
    'native SQLite journal freezes, executes, undoes and survives reopening',
    (tester) async {
      expect(Platform.isAndroid, true);
      final support = await getApplicationSupportDirectory();
      final fixture = File('${support.path}/shep-bulk-fixture.sqlite');
      final request = File('${support.path}/shep-bulk-request');
      await request.writeAsString('waiting-before-profile-open', flush: true);
      final end = DateTime.now().add(const Duration(seconds: 30));
      while (!await fixture.exists()) {
        if (DateTime.now().isAfter(end)) {
          fail('Run with android_e2e.py --bulk-only to seed this profile');
        }
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      final directory = await Directory.systemTemp.createTemp(
        'shep-bulk-native-',
      );
      final path = '${directory.path}/mail.sqlite3';
      await fixture.copy(path);
      final credentials = FixtureCredentials()..unavailable = true;
      var repository = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      var workspace = Workspace(repository, MemorySettings());
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
      await tester.pumpAndSettle();
      addTearDown(() async {
        await tester.pumpWidget(const SizedBox());
        workspace.dispose();
        await request.writeAsString('done', flush: true);
        await directory.delete(recursive: true);
        await fixture.delete();
      });
      expect(find.text('Bulk message 1'), findsOneWidget);
      final selection = workspace.selection!;
      final groups = workspace.groups!;
      await tap(tester, find.byTooltip('Select'));
      await tap(tester, find.text('Select all'));
      await wait(
        tester,
        () => selection.ready && selection.count == 130,
        'select all',
      );
      expect(find.text('All 130 selected'), findsOneWidget);
      await tap(tester, find.byTooltip('Archive selected'));
      await wait(tester, () => groups.review != null, 'review');
      expect(find.text('Archive 130 messages'), findsOneWidget);
      expect(
        find.textContaining('owner@example.test · Inbox: 130'),
        findsOneWidget,
      );
      await tap(tester, find.widgetWithText(FilledButton, 'Archive'));
      await wait(tester, () => workspace.resultCount == 0, 'painted intent');
      await wait(tester, () => groups.completed != null, 'completion');
      expect(find.text('Archived 130'), findsOneWidget);
      expect(groups.jobs.single.count('done'), 130);
      await capture(tester, 'native-bulk-journal-complete');
      await tap(tester, find.widgetWithText(TextButton, 'Undo'));
      await wait(tester, () => workspace.resultCount == 130, 'restored rows');
      await wait(
        tester,
        () => !groups.running && groups.jobs.single.finished,
        'undo',
      );
      expect(groups.jobs.single.count('undone'), 130);
      expect(find.text('Undone: 130 restored'), findsOneWidget);
      final jobId = groups.jobs.single.id;

      // Reopen the same profile: the journal and its receipts are durable.
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
      repository = await NativeRepository.open(path, credentials: credentials);
      workspace = Workspace(repository, MemorySettings());
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
      await tester.pumpAndSettle();
      final reopened = workspace.groups!;
      await wait(tester, () => reopened.jobs.isNotEmpty, 'reloaded History');
      expect(reopened.jobs.single.id, jobId);
      expect(reopened.jobs.single.undo, true);
      expect(reopened.jobs.single.count('undone'), 130);
      await tap(tester, find.byTooltip('Open navigation menu'));
      await tap(tester, find.text('Group History'));
      await wait(
        tester,
        () => find.text('Undone: 130 restored').evaluate().isNotEmpty,
        'History',
      );
      await tap(tester, find.textContaining('Archive 130 messages'));
      await wait(
        tester,
        () => find.text('Restored').evaluate().isNotEmpty,
        'items',
      );
      await capture(tester, 'native-bulk-journal-history');
      expect(workspace.resultCount, 130);
      expect(credentials.reads, 0, reason: 'POP3 steps never open credentials');
      expect(tester.takeException(), isNull);
      completed.add('bulk-native-journal-restart');
      binding.reportData = {...?binding.reportData, 'bulk_native': completed};
    },
  );
}
