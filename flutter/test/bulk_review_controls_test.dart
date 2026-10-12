import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'support/bulk_fixture.dart';
import 'support/paged_repository.dart';
import 'workspace_test.dart' show MemorySettings;
import 'mail_activity_visual_test.dart' show loadPreviewFonts;

class _ReviewRepository extends PagedRepository {
  _ReviewRepository() : super(extra: bulkFixtureMail());
  String? failNext, heldKind;
  Completer<void>? hold;
  final started = Completer<void>();

  @override
  Future<dynamic> groups(Map<String, Object?> command) async {
    if (command['kind'] == failNext) {
      failNext = null;
      throw StateError('Review fixture failure');
    }
    final result = await super.groups(command);
    if (command['kind'] == heldKind && hold != null) {
      if (!started.isCompleted) started.complete();
      await hold!.future;
    }
    return result;
  }
}

void main() {
  Future<void> wait(WidgetTester tester, bool Function() ready) async {
    for (var i = 0; !ready(); i++) {
      if (i == 300) fail('Review controls did not settle');
      await tester.pump(const Duration(milliseconds: 50));
    }
    await tester.pumpAndSettle();
  }

  Future<Workspace> start(
    WidgetTester tester,
    _ReviewRepository repository,
  ) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 2.625;
    addTearDown(tester.view.reset);
    await loadPreviewFonts();
    final workspace = Workspace(repository, MemorySettings());
    await workspace.initialize();
    workspace.setForeground(false);
    await tester.pumpWidget(ShepApp(workspace: workspace));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('Select'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Select all'));
    await wait(tester, () => workspace.selection!.ready);
    return workspace;
  }

  Future<void> open(WidgetTester tester, Workspace workspace) async {
    await tester.tap(find.byTooltip('Archive selected'));
    await wait(tester, () => workspace.groups!.review != null);
    expect(find.text('Archive 130 messages'), findsOneWidget);
  }

  Future<void> close(WidgetTester tester, Workspace workspace) async {
    await tester.pumpWidget(const SizedBox());
    workspace.dispose();
  }

  Future<void> scenario(
    WidgetTester tester,
    Future<void> Function(_ReviewRepository, Workspace) run,
  ) async {
    final repository = _ReviewRepository();
    final workspace = await start(tester, repository);
    try {
      await run(repository, workspace);
    } finally {
      await close(tester, workspace);
    }
  }

  testWidgets('failed approval stays open and retries the exact review', (
    tester,
  ) async {
    await scenario(tester, (repository, workspace) async {
      await open(tester, workspace);
      final review = workspace.groups!.review!.id;
      final capture = workspace.selection!.snapshot!.id;
      repository.failNext = 'approve';
      await tester.tap(find.widgetWithText(FilledButton, 'Archive'));
      await wait(tester, () => !workspace.groups!.deciding);
      expect(find.text('Archive 130 messages'), findsOneWidget);
      expect(
        find.textContaining('Could not confirm this action.'),
        findsWidgets,
      );
      expect(workspace.groups!.review!.id, review);
      expect(workspace.selection!.snapshot!.id, capture);
      expect(workspace.selection!.count, 130);
      await expectLater(
        find.byType(MaterialApp),
        matchesGoldenFile('goldens/bulk_review_retry_light.png'),
      );
      await tester.tap(find.widgetWithText(FilledButton, 'Archive'));
      await wait(tester, () => workspace.groups!.review == null);
      expect(find.text('Archive 130 messages'), findsNothing);
      expect(workspace.selection!.mode, false);
      await wait(tester, () => !workspace.groups!.running);
      expect(
        repository.groupPreview.calls.where((c) => c == 'approve').length,
        1,
      );
    });
  });

  testWidgets('failed dismissal retains a visible review recovery control', (
    tester,
  ) async {
    await scenario(tester, (repository, workspace) async {
      await open(tester, workspace);
      final review = workspace.groups!.review!.id;
      repository.failNext = 'decline';
      await tester.tapAt(const Offset(5, 5));
      await wait(tester, () => !workspace.groups!.deciding);
      expect(find.text('Archive 130 messages'), findsNothing);
      expect(find.text('Review action'), findsOneWidget);
      expect(workspace.groups!.review!.id, review);
      expect(workspace.selection!.count, 130);
      await expectLater(
        find.byType(MaterialApp),
        matchesGoldenFile('goldens/bulk_review_cleanup_light.png'),
      );
      await tester.tap(find.widgetWithText(TextButton, 'Retry'));
      await tester.pumpAndSettle();
      expect(find.text('Archive 130 messages'), findsOneWidget);
      await tester.tap(find.text('Cancel'));
      await wait(tester, () => workspace.groups!.review == null);
      expect(repository.groupPreview.jobs, isEmpty);
      expect(workspace.selection!.count, 130);
    });
  });

  testWidgets('held approval disables decisions and blocks Back until saved', (
    tester,
  ) async {
    await scenario(tester, (repository, workspace) async {
      await open(tester, workspace);
      repository.heldKind = 'approve';
      repository.hold = Completer<void>();
      await tester.tap(find.widgetWithText(FilledButton, 'Archive'));
      await wait(tester, () => repository.started.isCompleted);
      expect(
        tester
            .widget<FilledButton>(find.widgetWithText(FilledButton, 'Archive'))
            .onPressed,
        isNull,
      );
      expect(
        tester
            .widget<TextButton>(find.widgetWithText(TextButton, 'Cancel'))
            .onPressed,
        isNull,
      );
      await tester.binding.handlePopRoute();
      await tester.pumpAndSettle();
      expect(find.text('Archive 130 messages'), findsOneWidget);
      expect(workspace.selection!.mode, true);
      repository.hold!.complete();
      await wait(tester, () => workspace.groups!.review == null);
      expect(workspace.selection!.mode, false);
      await wait(tester, () => !workspace.groups!.running);
    });
  });

  testWidgets('Done and a new selection retire a late prepared review', (
    tester,
  ) async {
    await scenario(tester, (repository, workspace) async {
      repository.heldKind = 'prepare';
      repository.hold = Completer<void>();
      await tester.tap(find.byTooltip('Archive selected'));
      await wait(tester, () => repository.started.isCompleted);
      await tester.tap(find.text('Done'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Select'));
      await tester.pumpAndSettle();
      final semantics = tester.ensureSemantics();
      try {
        await tester.tap(
          find.bySemanticsLabel('Select A little room for good ideas'),
        );
        await wait(tester, () => workspace.selection!.ready);
        final newer = workspace.selection!.snapshot!.id;
        repository.hold!.complete();
        await wait(tester, () => !workspace.groups!.preparing);
        expect(find.text('Archive 130 messages'), findsNothing);
        expect(workspace.selection!.snapshot!.id, newer);
        expect(workspace.selection!.count, 1);
        expect(repository.groupPreview.jobs, isEmpty);
        expect(tester.takeException(), isNull);
      } finally {
        semantics.dispose();
      }
    });
  });
}
