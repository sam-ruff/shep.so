import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import '../workspace_test.dart' show MemorySettings;
import 'sent_handover_repository.dart';

Future<void> sentHandoverScenario(
  WidgetTester tester, {
  Future<void> Function(String)? capture,
}) async {
  final repository = SentHandoverRepository();
  final workspace = Workspace(repository, MemorySettings());
  try {
    await workspace.initialize();
    workspace.setForeground(false);
    await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
    Future<void> wait(bool Function() ready) async {
      final until = DateTime.now().add(const Duration(seconds: 20));
      while (!ready()) {
        if (DateTime.now().isAfter(until)) {
          fail('Sent handover controls did not settle');
        }
        await tester.pump(const Duration(milliseconds: 100));
      }
    }

    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Open navigation menu'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Sent'));
    await tester.pumpAndSettle();
    expect(find.text(SentHandoverRepository.subject), findsOneWidget);
    await tester.tap(find.byTooltip('Flag ${SentHandoverRepository.subject}'));
    await tester.pumpAndSettle();
    await wait(() => workspace.pending == 0);
    await tester.tap(find.byTooltip('Refresh'));
    await wait(() => repository.syncStarted);
    await tester.tap(find.text(SentHandoverRepository.subject));
    await wait(() => repository.bodyStarted);
    // Release only the synthetic network completion; the UI actions above use
    // real controls. The body result deliberately arrives after identity adoption.
    repository.syncGate.complete();
    await wait(() => !workspace.syncing);
    expect(
      workspace.mail(SentHandoverRepository.providerId)?.id,
      SentHandoverRepository.localId,
    );
    repository.bodyGate.complete();
    await wait(() => workspace.loadingBodies.isEmpty);
    await tester.pumpAndSettle();
    expect(
      find.text('This message moved. Open its destination folder.'),
      findsNothing,
    );
    expect(find.text('Exact saved body after Sent handover.'), findsOneWidget);
    expect(find.byTooltip('Unflag'), findsOneWidget);
    if (capture != null) await capture('sent-handover-reader');
    await tester.pageBack();
    await tester.pumpAndSettle();
    // Undo was created using the provider ID before it became an alias.
    await tester.tap(find.text('Undo'));
    await tester.pumpAndSettle();
    await wait(() => workspace.pending == 0);
    expect(repository.changes.last.$1, SentHandoverRepository.localId);
    expect(repository.message.starred, isFalse);
    await tester.tap(find.text(SentHandoverRepository.subject));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Archive'));
    await tester.pumpAndSettle();
    await wait(() => workspace.pending == 0);
    expect(find.text(SentHandoverRepository.subject), findsNothing);
    await tester.tap(find.text('Undo'));
    await tester.pumpAndSettle();
    await wait(() => workspace.pending == 0);
    expect(repository.changes.last.$2['folder'], 'Sent Mail');
    expect(find.text(SentHandoverRepository.subject), findsOneWidget);
    expect(workspace.error, isNull);
    if (capture != null) await capture('sent-handover-undo');
    expect(tester.takeException(), isNull);
  } finally {
    await tester.pumpWidget(const SizedBox());
    workspace.dispose();
  }
}
