import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'support/bulk_fixture.dart';
import 'support/paged_repository.dart';
import 'workspace_test.dart' show MemorySettings;
import 'mail_activity_visual_test.dart' show loadPreviewFonts;

class _UndoRepository extends PagedRepository {
  _UndoRepository() : super(extra: bulkFixtureMail());
  Completer<void>? capacity;
  bool loseUndo = false, failInspect = false;
  int? stopAfter;
  int steps = 0;
  final commands = <Map<String, Object?>>[];

  @override
  Future<dynamic> groups(Map<String, Object?> command) async {
    commands.add(Map.of(command));
    if (command['kind'] == 'inspect' && failInspect) {
      failInspect = false;
      throw StateError('Status inspection unavailable');
    }
    final result = await super.groups(command);
    if (command['kind'] == 'undo' && loseUndo) {
      loseUndo = false;
      throw StateError('Lost Undo acknowledgement');
    }
    return result;
  }

  @override
  Future<Map<String, dynamic>> groupStep() async {
    if (stopAfter == steps) capacity ??= Completer<void>();
    await capacity?.future;
    final result = await super.groupStep();
    if (result['stepped'] == true) steps++;
    return result;
  }
}

void main() {
  Future<void> wait(WidgetTester tester, bool Function() ready) async {
    for (var n = 0; !ready(); n++) {
      if (n == 300) fail('Undo controls did not settle');
      await tester.pump(const Duration(milliseconds: 20));
    }
    await tester.pumpAndSettle();
  }

  Future<void> tap(WidgetTester tester, Finder finder) async {
    await tester.ensureVisible(finder);
    await tester.pumpAndSettle();
    await tester.tap(finder);
    await tester.pumpAndSettle();
  }

  Future<void> scenario(
    WidgetTester tester,
    _UndoRepository repository,
    Future<void> Function(Workspace, String) run, {
    bool dark = false,
  }) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 2.625;
    addTearDown(tester.view.reset);
    await loadPreviewFonts();
    final workspace = Workspace(
      repository,
      MemorySettings()
        ..value = Preferences(
          appearance: dark ? ThemeMode.dark : ThemeMode.light,
        ),
    );
    try {
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(workspace: workspace));
      await tester.pumpAndSettle();
      await tap(tester, find.byTooltip('Select'));
      await tap(tester, find.text('Select all'));
      await wait(tester, () => workspace.selection!.ready);
      await tap(tester, find.byTooltip('Archive selected'));
      await wait(tester, () => workspace.groups!.review != null);
      final id = workspace.groups!.review!.id;
      await tap(tester, find.widgetWithText(FilledButton, 'Archive'));
      await wait(tester, () => workspace.groups!.review == null);
      await run(workspace, id);
      expect(tester.takeException(), isNull);
    } finally {
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
      if (repository.capacity case final capacity? when !capacity.isCompleted) {
        capacity.complete();
      }
      if (repository.groupPreview.hold case final hold?
          when !hold.isCompleted) {
        hold.complete();
      }
    }
  }

  for (final dark in [false, true]) {
    testWidgets(
      'queued notice Undo with occupied capacity (${dark ? 'dark' : 'light'})',
      (tester) async {
        final repository = _UndoRepository()..capacity = Completer<void>();
        await scenario(tester, repository, (workspace, id) async {
          expect(workspace.groups!.active!.count('done'), 0);
          expect(repository.steps, 0);
          expect(repository.cached.where((m) => m.folder == 'Inbox'), isEmpty);
          expect(find.widgetWithText(TextButton, 'Undo'), findsOneWidget);
          await expectLater(
            find.byType(MaterialApp),
            matchesGoldenFile(
              'goldens/bulk_queued_undo_${dark ? 'dark' : 'light'}.png',
            ),
          );
          await tap(tester, find.widgetWithText(TextButton, 'Undo'));
          await wait(tester, () => workspace.total == 130);
          expect(
            repository.commands.where((c) => c['kind'] == 'undo').single['id'],
            id,
          );
          expect(workspace.groups!.jobs.single.count('cancelled'), 130);
          repository.capacity!.complete();
          await wait(tester, () => !workspace.groups!.running);
          expect(repository.steps, 0);
        }, dark: dark);
      },
    );
  }

  testWidgets('paused History Undo retains a held first receipt', (
    tester,
  ) async {
    final repository = _UndoRepository();
    repository.groupPreview.hold = Completer<void>();
    repository.groupPreview.stepStarted = Completer<void>();
    await scenario(tester, repository, (workspace, id) async {
      await wait(tester, () => repository.groupPreview.stepStarted == null);
      await tap(tester, find.widgetWithText(TextButton, 'Pause'));
      await wait(tester, () => workspace.groups!.activeJobs.single.paused);
      expect(find.widgetWithText(TextButton, 'Undo'), findsOneWidget);
      await tap(tester, find.widgetWithText(TextButton, 'History'));
      await wait(
        tester,
        () => find.text('Group History').evaluate().isNotEmpty,
      );
      final card = find.byKey(ValueKey('group-history-$id'));
      await tap(
        tester,
        find.descendant(
          of: card,
          matching: find.widgetWithText(TextButton, 'Undo'),
        ),
      );
      await wait(
        tester,
        () => repository.cached.where((m) => m.folder == 'Inbox').length == 130,
      );
      final saved =
          await repository.groups({'kind': 'inspect', 'id': id}) as Map;
      expect((saved['counts'] as Map)['sending'], 1);
      expect((saved['counts'] as Map)['cancelled'], 129);
      repository.groupPreview.hold!.complete();
      repository.groupPreview.hold = null;
      await wait(tester, () => !workspace.groups!.running);
      final finished = workspace.groups!.jobs.firstWhere((j) => j.id == id);
      expect(finished.count('undone'), 1);
      expect(finished.count('cancelled'), 129);
      expect(repository.steps, 2);
      expect(repository.cached.where((m) => m.folder == 'Inbox').length, 130);
    });
  });

  testWidgets(
    'partial completion Undo cancels pending work and restores receipts',
    (tester) async {
      final repository = _UndoRepository()..stopAfter = 2;
      await scenario(tester, repository, (workspace, id) async {
        await wait(
          tester,
          () => repository.steps == 2 && repository.capacity != null,
        );
        await tap(tester, find.widgetWithText(TextButton, 'Undo'));
        await wait(tester, () => workspace.total == 130);
        repository.capacity!.complete();
        await wait(tester, () => !workspace.groups!.running);
        final finished = workspace.groups!.jobs.firstWhere((j) => j.id == id);
        expect(finished.count('undone'), 2);
        expect(finished.count('cancelled'), 128);
      });
    },
  );

  testWidgets('lost Undo status remains retryable after closing History', (
    tester,
  ) async {
    final repository = _UndoRepository()..capacity = Completer<void>();
    await scenario(tester, repository, (workspace, id) async {
      await tap(tester, find.byTooltip('Open group History'));
      await wait(
        tester,
        () => find.text('Group History').evaluate().isNotEmpty,
      );
      repository.loseUndo = repository.failInspect = true;
      final card = find.byKey(ValueKey('group-history-$id'));
      await tap(
        tester,
        find.descendant(
          of: card,
          matching: find.widgetWithText(TextButton, 'Undo'),
        ),
      );
      await wait(tester, () => workspace.groups!.undoError != null);
      expect(find.widgetWithText(TextButton, 'Retry Undo'), findsOneWidget);
      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.widgetWithText(TextButton, 'Retry Undo'), findsOneWidget);
      await tap(tester, find.widgetWithText(TextButton, 'Retry Undo'));
      await wait(tester, () => workspace.groups!.undoError == null);
      expect(repository.commands.where((c) => c['kind'] == 'undo').length, 1);
      expect(
        repository.commands
            .where((c) => c['kind'] == 'inspect')
            .map((c) => c['id']),
        [id, id],
      );
      repository.capacity!.complete();
      await wait(tester, () => !workspace.groups!.running);
    });
  });
}
