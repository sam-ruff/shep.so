import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/outgoing.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'support/preview_repository.dart';
import 'workspace_test.dart' show MemorySettings;

class ControlledOutbox extends PreviewRepository implements OutgoingRepository {
  ControlledOutbox({this.total = 1, this.sent = false})
    : super(delay: Duration.zero);
  int total;
  final bool sent;
  final actions = <OutgoingAction>[];
  bool failRecovery = true;
  final offsets = <int>[];
  final decisions = <bool>[];
  final pending = Completer<void>();
  @override
  Future<OutgoingPage> outbox({int offset = 0}) async {
    offsets.add(offset);
    return OutgoingPage(
      List.generate(
        (total - offset).clamp(0, 20),
        (i) => OutgoingEntry(
          id: 'entry-${offset + i}',
          draftId: 'draft-${offset + i}',
          accountId: 'fixture',
          subject: 'Review ${offset + i}',
          to: 'recipient@example.test',
          from: 'sender@example.test',
          state: sent ? 'delivered' : 'uncertain',
          sent: sent ? 'uncertain' : null,
          protocol: sent ? 'Imap' : null,
          sentPolicy: sent ? 'Automatic' : null,
        ),
      ),
      offset,
      total,
    );
  }

  @override
  Future<OutgoingResult> recoverOutgoing(
    String id,
    OutgoingAction action, {
    bool confirmed = false,
  }) async {
    decisions.add(confirmed);
    actions.add(action);
    if (failRecovery) throw Exception('Save failed. Free storage and retry.');
    await pending.future;
    total = 0;
    return const OutgoingResult(recovery: 'returned');
  }
}

void main() {
  Future<Workspace> open(
    WidgetTester tester,
    ControlledOutbox repository,
  ) async {
    tester.view.physicalSize = const Size(412, 892);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await tester.pumpWidget(ShepApp(workspace: workspace));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Open navigation menu'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Outbox'));
    await tester.pumpAndSettle();
    return workspace;
  }

  testWidgets(
    'uncertain review gates recovery, keeps storage errors visible and permits navigation while pending',
    (tester) async {
      final repository = ControlledOutbox();
      await open(tester, repository);
      final back = find.widgetWithText(OutlinedButton, 'Return to drafts');
      expect(tester.widget<OutlinedButton>(back).onPressed, isNull);
      await tester.tap(find.byType(CheckboxListTile));
      await tester.pumpAndSettle();
      await tester.tap(back);
      await tester.pumpAndSettle();
      expect(find.textContaining('Free storage and retry'), findsOneWidget);
      expect(find.text('Review 0'), findsOneWidget);
      expect(tester.widget<OutlinedButton>(back).onPressed, isNull);
      repository.failRecovery = false;
      await tester.tap(find.byType(CheckboxListTile));
      await tester.pumpAndSettle();
      await tester.tap(back);
      await tester.pump();
      expect(find.byType(LinearProgressIndicator), findsOneWidget);
      expect(tester.widget<OutlinedButton>(back).onPressed, isNull);
      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.text('Compose'), findsOneWidget);
      repository.pending.complete();
      await tester.pumpAndSettle();
      expect(repository.decisions, [true, true]);
      expect(tester.takeException(), isNull);
    },
  );
  testWidgets(
    'Outbox paging uses controls and resets previous review decisions',
    (tester) async {
      final repository = ControlledOutbox(total: 21);
      await open(tester, repository);
      await tester.tap(find.byType(CheckboxListTile).first);
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Next Outbox page'));
      await tester.pumpAndSettle();
      expect(find.text('Review 20'), findsOneWidget);
      expect(find.text('21–21 of 21'), findsOneWidget);
      expect(
        tester
            .widget<OutlinedButton>(
              find.widgetWithText(OutlinedButton, 'Return to drafts'),
            )
            .onPressed,
        isNull,
      );
      await tester.tap(find.byTooltip('Previous Outbox page'));
      await tester.pumpAndSettle();
      expect(find.text('Review 0'), findsOneWidget);
      expect(
        tester
            .widget<CheckboxListTile>(find.byType(CheckboxListTile).first)
            .value,
        false,
      );
      expect(repository.offsets, [0, 20, 0]);
      expect(tester.takeException(), isNull);
    },
  );
  testWidgets(
    'Sent copy retry needs a separate review; lookup never authorizes upload',
    (tester) async {
      final repository = ControlledOutbox(sent: true);
      await open(tester, repository);
      final upload = find.widgetWithText(
        OutlinedButton,
        'Save server Sent copy',
      );
      expect(tester.widget<OutlinedButton>(upload).onPressed, isNull);
      expect(find.text('Return to drafts'), findsNothing);
      await tester.tap(
        find.widgetWithText(OutlinedButton, 'Check provider Sent'),
      );
      await tester.pumpAndSettle();
      expect(repository.actions, [OutgoingAction.checkSent]);
      expect(repository.decisions, [false]);
      expect(tester.widget<OutlinedButton>(upload).onPressed, isNull);
      await tester.ensureVisible(find.byType(CheckboxListTile));
      await tester.tap(find.byType(CheckboxListTile));
      await tester.pumpAndSettle();
      await tester.ensureVisible(upload);
      await tester.tap(upload);
      await tester.pumpAndSettle();
      expect(repository.actions, [
        OutgoingAction.checkSent,
        OutgoingAction.copySent,
      ]);
      expect(repository.decisions, [false, true]);
      expect(tester.widget<OutlinedButton>(upload).onPressed, isNull);
      expect(find.textContaining('Free storage and retry'), findsOneWidget);
    },
  );
}
