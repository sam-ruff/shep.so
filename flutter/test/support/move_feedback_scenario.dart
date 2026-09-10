import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import '../workspace_test.dart' show MemorySettings;
import 'paged_repository.dart';

/// Real native controls; only transport completions are controlled by fixtures.
Future<void> moveFeedbackScenario(
  WidgetTester tester, {
  Future<void> Function(String)? capture,
}) async {
  final repo = PagedRepository();
  final workspace = Workspace(repo, MemorySettings());
  await workspace.initialize();
  workspace.setForeground(false);
  await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
  await tester.pumpAndSettle();
  Future<void> wait(bool Function() ready) async {
    for (var n = 0; !ready(); n++) {
      if (n >= 200) {
        fail('Move feedback did not reach the expected native state');
      }
      await tester.pump(const Duration(milliseconds: 100));
    }
    await tester.pumpAndSettle();
  }

  Future<void> swipe(String id) async {
    await tester.drag(find.byKey(ValueKey('mail-$id')), const Offset(-350, 0));
    await tester.pumpAndSettle();
  }

  try {
    await swipe('1');
    await wait(() => repo.jobs.length == 1);
    await swipe('2');
    await wait(() => repo.jobs.length == 2);
    expect(find.text('Archived 2 messages'), findsOneWidget);
    if (capture != null) await capture('native-counted-archive');
    repo.jobs[0].completeError(StateError('First move rejected'));
    await wait(() => workspace.pending == 1);
    expect(find.text('Archived 1 message'), findsOneWidget);
    await tester.tap(find.text('Undo'));
    await tester.pumpAndSettle();
    expect(find.text('Restored 1 message'), findsOneWidget);
    expect(find.text('Your week, a little clearer'), findsOneWidget);
    repo.jobs[1].complete();
    await wait(() => repo.jobs.length == 3);
    repo.jobs[2].completeError(StateError('Undo rejected'));
    await wait(() => workspace.pending == 0);
    expect(find.text('1 move needs review.'), findsOneWidget);
    if (capture != null) await capture('native-counted-undo-failure');
    await tester.tap(find.text('Retry Undo'));
    await wait(() => repo.jobs.length == 4);
    repo.jobs[3].complete();
    await wait(() => workspace.pending == 0);
    expect(workspace.error, isNull);
    expect(workspace.undoFailures, isEmpty);
    await tester.tap(find.byTooltip('Dismiss move notification'));
    await tester.pumpAndSettle();
    expect(find.text('Restored 1 message'), findsNothing);

    await tester.tap(find.text('A little room for good ideas'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Archive'));
    await wait(() => repo.jobs.length == 5); // Read is held before MOVE.
    expect(find.text('Archived 1 message'), findsOneWidget);
    // Archive returns to Inbox. Open another message and undo from its reader.
    await tester.tap(find.text('Your week, a little clearer'));
    await tester.pumpAndSettle();
    expect(find.text('Message'), findsOneWidget);
    await tester.tap(find.text('Undo'));
    await tester.pumpAndSettle();
    expect(find.text('Restored 1 message'), findsOneWidget);
    repo.jobs[4].complete();
    await wait(() => workspace.pending == 0);
    expect(repo.jobs.length, 5); // Unsent MOVE and reverse were both cancelled.
    expect(repo.cached.firstWhere((m) => m.id == '1').folder, 'Inbox');
    expect(find.text('Message'), findsOneWidget);
    await tester.tap(find.byTooltip('Refresh mail'));
    await tester.pumpAndSettle();
    expect(find.text('Preview refreshed'), findsOneWidget);
    expect(find.text('Restored 1 message'), findsOneWidget);
    if (capture != null) await capture('native-reader-counted-undo');
    await tester.pump(const Duration(seconds: 6));
    await tester.pumpAndSettle();
    expect(find.text('Restored 1 message'), findsNothing);
    expect(find.text('Undo'), findsNothing);
    expect(tester.takeException(), isNull);
  } finally {
    workspace.dispose();
    await tester.pumpWidget(const SizedBox());
  }
}
