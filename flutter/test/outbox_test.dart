import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/outgoing.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'package:shep_mobile/ui/outbox.dart';
import 'support/preview_repository.dart';
import 'workspace_test.dart' show MemorySettings;

class ControlledOutbox extends PreviewRepository implements OutgoingRepository {
  @override
  Future<void> cancelOutgoing(String id) async {}
  final resumed = <String>[];
  String? resumeError;
  Completer<void>? resumeGate;
  @override
  Future<void> resumeOutgoing(String id) async {
    resumed.add(id);
    await resumeGate?.future;
    outgoingError = resumeError;
  }

  ControlledOutbox({
    this.total = 1,
    this.sent = false,
    this.state = 'uncertain',
  }) : super(delay: Duration.zero);
  int total;
  final bool sent;
  String state;
  final states = <int, String>{};
  final sentStates = <int, String>{};
  String? outgoingError;
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
          state: states[offset + i] ?? (sent ? 'delivered' : state),
          sentError: outgoingError,
          sent: sentStates[offset + i] ?? (sent ? 'uncertain' : null),
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
      expect(
        find.widgetWithText(FloatingActionButton, 'New message'),
        findsOneWidget,
      );
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

  testWidgets(
    'waiting delivery resumes the same attempt and keeps errors visible',
    (tester) async {
      final repository = ControlledOutbox(state: 'waiting')
        ..resumeError = 'credential store is locked';
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await tester.pumpWidget(
        MaterialApp(home: OutboxScreen(workspace: workspace)),
      );
      await tester.pump();
      await tester.pump();

      await tester.tap(find.text('Resume delivery'));
      await tester.pump();
      await tester.pump();

      expect(repository.resumed, ['entry-0']);
      expect(find.textContaining('credential store is locked'), findsOneWidget);
      expect(find.text('Resume delivery'), findsOneWidget);
    },
  );

  testWidgets(
    'active delivery refreshes its bounded page and reveals failure',
    (tester) async {
      final repository = ControlledOutbox(state: 'submitting');
      final workspace = Workspace(repository, MemorySettings());
      addTearDown(workspace.dispose);
      await tester.pumpWidget(
        MaterialApp(home: OutboxScreen(workspace: workspace)),
      );
      await tester.pump();
      await tester.pump();
      repository
        ..state = 'rejected'
        ..outgoingError = 'SMTP refused this recipient';

      await tester.pump(const Duration(seconds: 3));
      await tester.pump();

      expect(repository.offsets, [0, 0]);
      expect(
        find.textContaining('SMTP refused this recipient'),
        findsOneWidget,
      );
      expect(find.text('Resume delivery'), findsNothing);
    },
  );

  testWidgets('polling preserves a mixed-page review and action error', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(800, 1200);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final repository = ControlledOutbox(total: 2, state: 'waiting')
      ..states[1] = 'uncertain'
      ..resumeError = 'credential store is locked';
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await tester.pumpWidget(
      MaterialApp(home: OutboxScreen(workspace: workspace)),
    );
    await tester.pump();
    await tester.pump();
    await tester.ensureVisible(find.byType(CheckboxListTile));
    await tester.pumpAndSettle();
    await tester.tap(find.byType(CheckboxListTile));
    await tester.pump();
    await tester.tap(find.text('Resume delivery'));
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('credential store is locked'), findsWidgets);

    await tester.pump(const Duration(seconds: 3));
    await tester.pump();

    expect(
      tester.widget<CheckboxListTile>(find.byType(CheckboxListTile)).value,
      isTrue,
    );
    expect(find.textContaining('credential store is locked'), findsWidgets);
  });

  testWidgets('disposing during Resume ignores the late result', (
    tester,
  ) async {
    final repository = ControlledOutbox(state: 'waiting')
      ..resumeGate = Completer<void>();
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await tester.pumpWidget(
      MaterialApp(home: OutboxScreen(workspace: workspace)),
    );
    await tester.pump();
    await tester.pump();
    await tester.tap(find.text('Resume delivery'));
    await tester.pumpWidget(const SizedBox());
    repository.resumeGate!.complete();
    await tester.pump();

    expect(tester.takeException(), isNull);
  });

  testWidgets('polling invalidates review when the Sent copy phase changes', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(800, 1200);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final repository = ControlledOutbox(total: 2, sent: true)
      ..states[0] = 'queued';
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    await tester.pumpWidget(
      MaterialApp(home: OutboxScreen(workspace: workspace)),
    );
    await tester.pump();
    await tester.pump();
    final review = find.byType(CheckboxListTile);
    await tester.ensureVisible(review);
    await tester.pumpAndSettle();
    await tester.tap(review);
    await tester.pump();
    expect(tester.widget<CheckboxListTile>(review).value, isTrue);

    repository.sentStates[1] = 'appending';
    await tester.pump(const Duration(seconds: 3));
    await tester.pump();

    expect(tester.widget<CheckboxListTile>(review).value, isFalse);
  });
}
