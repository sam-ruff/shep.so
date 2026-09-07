import 'support/move_feedback_scenario.dart';
import 'package:flutter/services.dart';
import 'dart:convert';
import 'dart:io';
import 'support/message_find_scenario.dart';
import 'support/sent_handover_scenario.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'support/preview_repository.dart';
import 'workspace_test.dart' show MemorySettings;

void main() {
  testWidgets('counted move Undo, partial failure, retry and reader expiry', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(412, 892);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await moveFeedbackScenario(tester);
  });
  testWidgets(
    'Find controls, visible quote scope, wrapped matches and native selection',
    (tester) async {
      tester.view.physicalSize = const Size(412, 892);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      String? copied;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          if (call.method == 'Clipboard.setData') {
            copied = call.arguments['text'] as String?;
          }
          if (call.method == 'Clipboard.getData') return {'text': copied};
          return null;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          null,
        ),
      );
      final body =
          (jsonDecode(File('../shared/find-preview.json').readAsStringSync())
                  as Map)['body']
              as String;
      final w = Workspace(
        PreviewRepository(delay: Duration.zero, firstBody: body),
        MemorySettings(),
      );
      await tester.pumpWidget(ShepApp(workspace: w));
      await tester.pumpAndSettle();
      await tester.tap(find.text('A little room for good ideas'));
      await tester.pumpAndSettle();
      await messageFindScenario(tester);
      await tester.pumpWidget(const SizedBox());
      w.dispose();
    },
  );

  Future<Workspace> start(WidgetTester t, {bool fail = false}) async {
    t.view.physicalSize = const Size(412, 892);
    t.view.devicePixelRatio = 1;
    addTearDown(t.view.resetPhysicalSize);
    addTearDown(t.view.resetDevicePixelRatio);
    final w = Workspace(
      PreviewRepository(delay: Duration.zero, fail: fail),
      MemorySettings(),
    );
    addTearDown(w.dispose);
    await t.pumpWidget(ShepApp(workspace: w));
    await t.pumpAndSettle();
    return w;
  }

  testWidgets('Sent handover preserves the open reader, late body and Undo', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(412, 892);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await sentHandoverScenario(tester);
  });
  testWidgets(
    'reader exposes failed flag, retry and dismiss without leaving mail',
    (t) async {
      final w = await start(t, fail: true);
      await t.tap(find.text('A little room for good ideas'));
      await t.pumpAndSettle();
      final starred = w.mail('1')!.starred;
      await t.tap(find.byTooltip(starred ? 'Unflag' : 'Flag'));
      await t.pumpAndSettle();
      expect(w.mail('1')!.starred, starred);
      expect(
        find.textContaining('The affected display was restored'),
        findsOneWidget,
      );
      expect(find.text('Retry'), findsOneWidget);
      await t.tap(find.text('Retry'));
      await t.pumpAndSettle();
      expect(w.mail('1')!.starred, starred);
      expect(
        find.textContaining('The affected display was restored'),
        findsOneWidget,
      );
      await t.tap(find.byTooltip('Dismiss error'));
      await t.pumpAndSettle();
      expect(find.text('Retry'), findsNothing);
      expect(find.text('Message'), findsOneWidget);
      expect(t.takeException(), isNull);
    },
  );
  testWidgets('left swipe archives; undo returns row', (t) async {
    final w = await start(t);
    await t.drag(find.byKey(const ValueKey('mail-1')), const Offset(-330, 0));
    await t.pumpAndSettle();
    expect(w.mail('1')!.folder, 'Archive');
    await t.tap(find.text('Undo'));
    await t.pumpAndSettle();
    expect(find.text('A little room for good ideas'), findsOneWidget);
    expect(find.text('Restored 1 message'), findsOneWidget);
    await t.pump(const Duration(seconds: 6));
    expect(find.text('Restored 1 message'), findsNothing);
    expect(w.undo, isNull);
  });
  testWidgets('right swipe read; disabled swipe is inert', (t) async {
    final w = await start(t);
    await t.drag(find.byKey(const ValueKey('mail-1')), const Offset(330, 0));
    await t.pumpAndSettle();
    expect(w.mail('1')!.unread, false);
    await w.savePreferences(w.preferences.copy(leftSwipe: MailAction.none));
    await t.pumpAndSettle();
    await t.drag(find.byKey(const ValueKey('mail-1')), const Offset(-330, 0));
    await t.pumpAndSettle();
    expect(w.mail('1')!.folder, 'Inbox');
  });
  testWidgets('failed swipe restores row and retry', (t) async {
    await start(t, fail: true);
    await t.drag(find.byKey(const ValueKey('mail-1')), const Offset(-330, 0));
    await t.pumpAndSettle();
    expect(find.text('A little room for good ideas'), findsOneWidget);
    expect(find.text('Retry'), findsOneWidget);
  });
  testWidgets('swipe icon and accessible menu follow read intent', (t) async {
    await start(t);
    final gesture = await t.startGesture(
      t.getCenter(find.byKey(const ValueKey('mail-1'))),
    );
    await gesture.moveBy(const Offset(25, 0));
    await t.pump();
    await gesture.moveBy(const Offset(220, 0));
    await t.pump();
    expect(find.text('Mark read'), findsOneWidget);
    expect(find.byIcon(Icons.mark_email_read_outlined), findsOneWidget);
    await gesture.up();
    await t.pumpAndSettle();
    await t.tap(find.byTooltip('Actions for A little room for good ideas'));
    await t.pumpAndSettle();
    final unreadAction = find.widgetWithText(
      PopupMenuItem<MailAction>,
      'Mark unread',
    );
    expect(unreadAction, findsOneWidget);
    await t.tap(unreadAction);
    await t.pumpAndSettle();
    await t.tap(find.byTooltip('Actions for A little room for good ideas'));
    await t.pumpAndSettle();
    expect(
      find.widgetWithText(PopupMenuItem<MailAction>, 'Mark read'),
      findsOneWidget,
    );
  });
  testWidgets('theme and swipe controls reachable', (t) async {
    await start(t);
    await t.tap(find.text('Preferences'));
    await t.pumpAndSettle();
    await t.tap(find.text('System'));
    await t.pumpAndSettle();
    await t.tap(find.text('Dark').last);
    await t.pumpAndSettle();
    expect(find.text('Swipe left'), findsOneWidget);
    expect(t.takeException(), isNull);
  });
  testWidgets('reply keeps text after send refusal', (t) async {
    await start(t);
    await t.tap(find.text('A little room for good ideas'));
    await t.pumpAndSettle();
    await t.scrollUntilVisible(
      find.text('Reply'),
      250,
      scrollable: find.byType(Scrollable).last,
    );
    await t.tap(find.text('Reply'));
    await t.pumpAndSettle();
    await t.tap(find.byTooltip('Send'));
    await t.pumpAndSettle();
    expect(
      find.text('Preview cannot send mail. Your draft is still open.'),
      findsOneWidget,
    );
  });
  testWidgets('calendar edits exact event', (t) async {
    final w = await start(t);
    await t.tap(find.text('Calendar'));
    await t.pumpAndSettle();
    await t.scrollUntilVisible(
      find.text('Coffee with Jamie'),
      200,
      scrollable: find.byType(Scrollable).last,
    );
    await t.tap(find.text('Coffee with Jamie'));
    await t.pumpAndSettle();
    await t.enterText(
      find.widgetWithText(TextField, 'Coffee with Jamie'),
      'Coffee at eleven',
    );
    await t.tap(find.text('Save event'));
    await t.pumpAndSettle();
    expect(w.events.any((e) => e.title == 'Coffee at eleven'), true);
  });
}
