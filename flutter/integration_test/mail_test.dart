import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/ui/app.dart';
import '../test/support/preview_repository.dart';
import '../test/workspace_test.dart' show MemorySettings;

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets('device swipe, undo, appearance and saved draft navigation', (
    tester,
  ) async {
    final w = Workspace(
      PreviewRepository(delay: const Duration(milliseconds: 350)),
      MemorySettings(),
    );
    await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: w));
    await tester.pumpAndSettle();
    await tester.drag(
      find.byKey(const ValueKey('mail-1')),
      const Offset(-350, 0),
    );
    await tester.pumpAndSettle();
    expect(find.text('Archived 1 message'), findsOneWidget);
    await tester.tap(find.text('Undo'));
    await tester.pumpAndSettle();
    expect(find.text('A little room for good ideas'), findsOneWidget);
    await tester.tap(find.text('Preferences'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('System'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Dark').last);
    await tester.pumpAndSettle();
    expect(find.text('Swipe left'), findsOneWidget);
    await tester.tap(find.text('Mail'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FloatingActionButton, 'New message'));
    await tester.pumpAndSettle();
    await tester.enterText(
      find.widgetWithText(TextField, 'To'),
      'test@example.test',
    );
    await tester.enterText(
      find.widgetWithText(TextField, 'Subject'),
      'Device draft',
    );
    await tester.tap(find.text('Save draft'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Open navigation menu'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Drafts'));
    await tester.pumpAndSettle();
    expect(find.text('Device draft'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
  testWidgets('device failed action rollback and calendar navigation', (
    tester,
  ) async {
    final w = Workspace(
      PreviewRepository(delay: const Duration(milliseconds: 350), fail: true),
      MemorySettings(),
    );
    await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: w));
    await tester.pumpAndSettle();
    await tester.drag(
      find.byKey(const ValueKey('mail-1')),
      const Offset(-350, 0),
    );
    await tester.pumpAndSettle();
    for (
      var attempt = 0;
      attempt < 50 && find.text('Retry').evaluate().isEmpty;
      attempt++
    ) {
      await tester.pump(const Duration(milliseconds: 100));
    }
    expect(find.text('Retry'), findsOneWidget);
    expect(find.text('A little room for good ideas'), findsOneWidget);
    await tester.tap(find.text('Calendar'));
    await tester.pumpAndSettle();
    expect(find.text('September 2026'), findsOneWidget);
  });
  testWidgets('device remaps each swipe, disables one and preserves settings', (
    tester,
  ) async {
    final settings = MemorySettings();
    final w = Workspace(PreviewRepository(delay: Duration.zero), settings);
    await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: w));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Preferences'));
    await tester.pumpAndSettle();

    Future<void> choose(String label, String value) async {
      final tile = find.ancestor(
        of: find.text(label),
        matching: find.byType(ListTile),
      );
      await tester.tap(
        find.descendant(
          of: tile,
          matching: find.byType(DropdownButton<MailAction>),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text(value).last);
      await tester.pumpAndSettle();
    }

    await choose('Swipe left', 'None');
    await choose('Swipe right', 'Flag / unflag');
    await tester.tap(find.text('Mail'));
    await tester.pumpAndSettle();
    await tester.drag(
      find.byKey(const ValueKey('mail-1')),
      const Offset(-350, 0),
    );
    await tester.pumpAndSettle();
    expect(find.text('A little room for good ideas'), findsOneWidget);
    expect(find.text('Archived 1 message'), findsNothing);
    await tester.drag(
      find.byKey(const ValueKey('mail-1')),
      const Offset(350, 0),
    );
    await tester.pumpAndSettle();
    expect(
      find.byTooltip('Unflag A little room for good ideas'),
      findsOneWidget,
    );
    await tester.tap(find.text('Undo'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Flag A little room for good ideas'), findsOneWidget);

    // Reopen with the same saved settings; all actions above used native controls.
    final reopened = Workspace(
      PreviewRepository(delay: Duration.zero),
      settings,
    );
    await reopened.initialize();
    await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: reopened));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Preferences'));
    await tester.pumpAndSettle();
    expect(find.text('None'), findsOneWidget);
    expect(find.text('Flag / unflag'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
