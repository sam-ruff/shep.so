import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'logical_mail_actions_test.dart' show LogicalRepository;
import 'mail_activity_visual_test.dart' show loadPreviewFonts;
import 'workspace_test.dart' show MemorySettings;

void main() {
  Future<void> wait(WidgetTester tester, bool Function() ready) async {
    for (var n = 0; !ready(); n++) {
      if (n == 200) fail('Logical action control did not settle');
      await tester.pump(const Duration(milliseconds: 10));
    }
    await tester.pumpAndSettle();
  }

  for (final dark in [false, true]) {
    testWidgets(
      'logical CREATE Waiting and visible Undo (${dark ? 'dark' : 'light'})',
      (tester) async {
        tester.view.physicalSize = const Size(390, 700);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
        await loadPreviewFonts();
        final repository = LogicalRepository()..waiting = true;
        final workspace = Workspace(
          repository,
          MemorySettings()
            ..value = Preferences(
              appearance: dark ? ThemeMode.dark : ThemeMode.light,
            ),
        );
        await workspace.initialize();
        workspace.setForeground(false);
        try {
          await tester.pumpWidget(ShepApp(workspace: workspace));
          await tester.pumpAndSettle();
          final mail = workspace.visible.first;
          await tester.tap(find.byTooltip('Actions for ${mail.subject}'));
          await tester.pumpAndSettle();
          await tester.tap(find.text('Archive').last);
          await wait(tester, () => workspace.pending == 0);
          expect(repository.requests.map((r) => r.$3), ['archive', 'archive']);
          expect(find.text('Archiving 1 message'), findsOneWidget);
          expect(find.text('Archived 1 message'), findsNothing);
          expect(
            find.textContaining('Destination CREATE needs review'),
            findsOneWidget,
          );
          expect(workspace.moves.records.single.committed, false);
          await expectLater(
            find.byType(ShepApp),
            matchesGoldenFile(
              'goldens/logical_destination_waiting_${dark ? 'dark' : 'light'}.png',
            ),
          );
          final action = workspace.moves.records.single.actionId;
          await tester.tap(find.widgetWithText(TextButton, 'Undo'));
          await wait(tester, () => repository.cancelled.isNotEmpty);
          expect(repository.cancelled, [action]);
          expect(
            repository.requests.where((r) => r.$1 == 'physical-admit'),
            isEmpty,
          );
          expect(workspace.mail(mail.id)!.folder, 'Inbox');
          expect(tester.takeException(), isNull);
        } finally {
          await tester.pumpWidget(const SizedBox());
          workspace.dispose();
        }
      },
    );
  }
}
