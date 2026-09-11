import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'package:shep_mobile/ui/mail_tile.dart';
import '../workspace_test.dart' show MemorySettings;
import 'bulk_fixture.dart';
import 'paged_repository.dart';

/// Real selection and group controls over a paged preview with 130 Inbox
/// messages and a synthetic journal. Only step completions and injected
/// outcomes are fixture-controlled; every decision goes through the UI.
Future<void> bulkControlsScenario(
  WidgetTester tester, {
  required bool dark,
  Future<void> Function(String)? capture,
}) async {
  final scheme = dark ? 'dark' : 'light';
  final repository = PagedRepository(extra: bulkFixtureMail())
    ..stepDelay = const Duration(milliseconds: 2);
  final journal = repository.groupPreview;
  final workspace = Workspace(
    repository,
    MemorySettings()
      ..value = Preferences(
        appearance: dark ? ThemeMode.dark : ThemeMode.light,
      ),
  );
  await workspace.initialize();
  workspace.setForeground(false);
  await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
  await tester.pumpAndSettle();
  Future<void> wait(bool Function() ready, [String what = 'state']) async {
    for (var n = 0; !ready(); n++) {
      if (n >= 300) fail('Group controls did not reach the expected $what');
      await tester.pump(const Duration(milliseconds: 50));
    }
    await tester.pumpAndSettle();
  }

  Future<void> tap(Finder finder) async {
    await tester.ensureVisible(finder);
    await tester.pumpAndSettle();
    await tester.tap(finder);
    await tester.pumpAndSettle();
  }

  final selection = workspace.selection!;
  final groups = workspace.groups!;
  final semantics = tester.ensureSemantics();
  try {
    // Select mode: button beside Search, then per-row checkboxes.
    expect(find.byType(ShepCheckbox), findsNothing);
    await tap(find.byTooltip('Select'));
    expect(selection.mode, true);
    expect(find.text('No messages selected'), findsOneWidget);
    expect(find.byType(ShepCheckbox), findsWidgets);
    await tap(find.bySemanticsLabel('Select A little room for good ideas'));
    await wait(() => selection.ready && selection.count == 1, 'first row');
    expect(find.text('1 selected'), findsOneWidget);
    expect(
      find.bySemanticsLabel(
        RegExp(r'^Unread, A little room for good ideas, selected'),
      ),
      findsOneWidget,
    );
    // Long press extends the range from the anchor, like Shift+click.
    await tester.longPress(find.text('Coffee on Thursday?'));
    await wait(() => selection.ready && selection.count == 3, 'range');
    expect(find.text('3 selected'), findsOneWidget);
    // Tapping a selected row toggles it off without opening the reader.
    await tap(find.text('Your week, a little clearer'));
    await wait(() => selection.ready && selection.count == 2, 'toggle');
    expect(find.text('Reply all'), findsNothing);
    await tap(find.text('Select all'));
    await wait(() => selection.ready && selection.count == 130, 'select all');
    expect(find.text('All 130 selected'), findsOneWidget);
    expect(journal.jobs, isEmpty);
    await capture?.call('bulk-selection-$scheme');
    await tap(find.text('Clear'));
    await wait(() => !selection.pending && selection.count == 0, 'clear');
    expect(find.text('No messages selected'), findsOneWidget);
    expect(
      tester
          .widget<IconButton>(
            find.ancestor(
              of: find.byTooltip('Archive selected'),
              matching: find.byType(IconButton),
            ),
          )
          .onPressed,
      isNull,
    );
    await tap(find.text('Select all'));
    await wait(() => selection.ready && selection.count == 130, 'select all');

    // The frozen review shows exact counts per account and folder.
    await tap(find.byTooltip('Archive selected'));
    await wait(() => groups.review != null, 'review');
    expect(find.text('Archive 130 messages'), findsOneWidget);
    expect(find.textContaining('Personal · Inbox: 86'), findsOneWidget);
    expect(find.textContaining('Work · Inbox: 44'), findsOneWidget);
    await capture?.call('bulk-review-$scheme');
    await tap(find.text('Cancel'));
    await wait(() => groups.review == null, 'declined review');
    expect(journal.jobs, isEmpty);
    expect(selection.mode, false);
    expect(workspace.resultCount, 130);

    // Approve: intent paints immediately, one owned step at a time.
    await tap(find.byTooltip('Select'));
    await tap(find.text('Select all'));
    await wait(() => selection.ready && selection.count == 130, 'select all');
    final hold = Completer<void>(), started = Completer<void>();
    journal.hold = hold;
    journal.stepStarted = started;
    await tap(find.byTooltip('Archive selected'));
    await wait(() => groups.review != null, 'review');
    await tap(find.widgetWithText(FilledButton, 'Archive'));
    await wait(() => started.isCompleted, 'first step');
    expect(workspace.resultCount, 0, reason: 'approved intent paints');
    expect(find.text('All clear'), findsOneWidget);
    await wait(() => find.textContaining('Archiving, ').evaluate().isNotEmpty);
    await capture?.call('bulk-progress-$scheme');
    await tap(find.text('Pause'));
    journal.hold = null;
    hold.complete();
    await wait(() => !groups.running, 'paused loop');
    expect(find.textContaining('Paused'), findsOneWidget);
    expect(journal.jobs.values.single.count('done'), 1);
    await tap(find.text('Resume'));
    await wait(() => groups.completed != null, 'completion');
    expect(find.text('Archived 130'), findsOneWidget);
    expect(repository.cached.where((m) => m.folder == 'Inbox'), isEmpty);
    await capture?.call('bulk-complete-$scheme');

    // Group Undo restores acknowledged rows through inverse receipts.
    await tap(find.widgetWithText(TextButton, 'Undo'));
    await wait(() => workspace.resultCount == 130, 'restored rows');
    await wait(
      () => !groups.running && groups.jobs.first.finished,
      'undo done',
    );
    expect(groups.jobs.first.count('undone'), 130);
    expect(find.text('Undone: 130 restored'), findsOneWidget);
    await tap(find.byTooltip('Dismiss group notification'));
    expect(find.text('Undone: 130 restored'), findsNothing);

    // Injected outcomes: one definite failure and one unconfirmed step.
    journal.outcomes['1'] = 'failed';
    journal.outcomes['2'] = 'uncertain';
    await tap(find.byTooltip('Select'));
    await tap(find.text('Select all'));
    await wait(() => selection.ready && selection.count == 130, 'select all');
    await tap(find.byTooltip('Mark read selected'));
    await wait(() => groups.review != null, 'review');
    expect(find.text('Mark read 130 messages'), findsOneWidget);
    await tap(find.widgetWithText(FilledButton, 'Mark read'));
    await wait(() => !groups.running && groups.jobs.first.paused, 'pause');
    expect(find.textContaining('Paused, 2 need review'), findsOneWidget);
    await tap(find.text('History'));
    await wait(() => find.text('Group History').evaluate().isNotEmpty);
    expect(find.textContaining('Mark read 130 messages'), findsOneWidget);
    await tap(find.textContaining('Mark read 130 messages'));
    await wait(() => find.text('Retry').evaluate().isNotEmpty, 'items');
    expect(find.textContaining('Failed · Fixture rejected'), findsOneWidget);
    expect(find.textContaining('Unconfirmed · '), findsOneWidget);
    await capture?.call('bulk-history-$scheme');
    await tap(find.text('Retry'));
    await wait(() => find.text('Retry').evaluate().isEmpty, 'retried');
    expect(groups.jobs.first.count('failed'), 0);
    expect(groups.jobs.first.paused, true);
    await tap(find.text('Accept current state'));
    await wait(() => groups.jobs.first.count('accepted') == 1, 'accepted');
    expect(find.text('Accept current state'), findsNothing);
    await tap(find.widgetWithText(TextButton, 'Resume'));
    await wait(() => !groups.running && groups.jobs.first.finished, 'finish');
    expect(find.textContaining('Marked read 64, 65 skipped'), findsOneWidget);
    expect(groups.jobs.first.count('accepted'), 1);
    await tap(find.widgetWithText(TextButton, 'Remove'));
    await wait(() => groups.jobs.length == 1, 'removed');
    expect(find.textContaining('Mark read 130 messages'), findsNothing);
    expect(find.textContaining('Archive 130 messages'), findsOneWidget);
    await tester.pageBack();
    await tester.pumpAndSettle();
    expect(workspace.error, isNull);
    expect(groups.error, isNull);
    expect(tester.takeException(), isNull);
  } finally {
    semantics.dispose();
    await tester.pumpWidget(const SizedBox());
    workspace.dispose();
  }
}
