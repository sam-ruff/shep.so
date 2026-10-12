import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/group_history.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/mail_groups.dart';
import 'package:shep_mobile/ui/theme.dart';
import 'support/history_repository.dart';
import 'workspace_test.dart' show MemorySettings;
import 'mail_activity_visual_test.dart' show loadPreviewFonts;

void main() {
  Future<void> wait(WidgetTester tester, bool Function() ready) async {
    for (var i = 0; !ready(); i++) {
      if (i == 300) fail('History controls did not settle');
      await tester.pump(const Duration(milliseconds: 20));
    }
    await tester.pump();
  }

  Future<void> tap(
    WidgetTester tester,
    Finder finder, {
    double delta = 500,
  }) async {
    if (finder.evaluate().isEmpty) {
      await tester.scrollUntilVisible(
        finder,
        delta,
        scrollable: find.byType(Scrollable).last,
        maxScrolls: 100,
      );
    }
    await tester.ensureVisible(finder);
    await tester.pump();
    await tester.tap(finder);
    await tester.pump();
  }

  GroupHistory observation(WidgetTester tester) =>
      (tester.state(find.byType(GroupHistoryScreen)) as dynamic).history
          as GroupHistory;
  Finder card(String id) => find.byKey(ValueKey('group-history-$id'));
  Finder rowAction(String id, String label) => find
      .descendant(
        of: card(id),
        matching: find.widgetWithText(TextButton, label),
      )
      .first;

  Future<void> scenario(
    WidgetTester tester,
    HistoryRepository repository,
    Future<void> Function(Workspace) run, {
    bool dark = false,
    bool attentionBanner = false,
  }) async {
    tester.view.physicalSize = const Size(1080, 2280);
    tester.view.devicePixelRatio = 2.625;
    addTearDown(tester.view.reset);
    await loadPreviewFonts();
    final workspace = Workspace(repository, MemorySettings());
    try {
      if (attentionBanner) await workspace.groups!.refreshHistory();
      await tester.pumpWidget(
        MaterialApp(
          debugShowCheckedModeBanner: false,
          theme: shepTheme(dark ? Brightness.dark : Brightness.light),
          home: Builder(
            builder: (context) => Scaffold(
              body: Column(
                children: [
                  if (attentionBanner)
                    AnimatedBuilder(
                      animation: workspace,
                      builder: (_, _) =>
                          GroupActionBanner(workspace: workspace),
                    ),
                  TextButton(
                    onPressed: () => Navigator.push(
                      context,
                      MaterialPageRoute<void>(
                        builder: (_) =>
                            GroupHistoryScreen(workspace: workspace),
                      ),
                    ),
                    child: const Text('Open group History'),
                  ),
                ],
              ),
            ),
          ),
        ),
      );
      await tap(tester, find.text('Open group History'));
      await wait(
        tester,
        () =>
            find.byType(GroupHistoryScreen).evaluate().isNotEmpty &&
            !observation(tester).loading,
      );
      await tester.pumpAndSettle();
      await run(workspace);
      expect(tester.takeException(), isNull);
    } finally {
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
    }
  }

  for (final accept in [false, true]) {
    for (final scope in ['group', 'page', 'away-and-back']) {
      testWidgets(
        'held ${accept ? 'Accept' : 'Retry'} cannot replace $scope details or new control targets',
        (tester) async {
          final repository = HistoryRepository();
          await scenario(tester, repository, (workspace) async {
            await tap(tester, find.text('Move to group-064 125 messages'));
            await wait(tester, () => !observation(tester).itemsLoading);
            repository.heldDecisionKind = accept ? 'accept' : 'retry';
            repository.heldDecisionJob = 'group-064';
            repository.decisionHold = Completer<void>();
            await tap(
              tester,
              rowAction('group-064', accept ? 'Accept current state' : 'Retry'),
            );
            await wait(tester, () => repository.decisionStarted.isCompleted);
            var expected = 'group-064', position = 50;
            if (scope != 'page') {
              await tap(tester, find.text('Move to group-063 125 messages'));
              await wait(tester, () => !observation(tester).itemsLoading);
              expected = 'group-063';
              position = 0;
            }
            if (scope == 'away-and-back') {
              await tap(
                tester,
                find.text('Move to group-064 125 messages'),
                delta: -500,
              );
              await wait(tester, () => !observation(tester).itemsLoading);
              expected = 'group-064';
              position = 50;
            }
            if (scope != 'group') {
              await tap(
                tester,
                find.widgetWithText(TextButton, 'Next 50 messages'),
              );
              await wait(tester, () => !observation(tester).itemsLoading);
            }
            final readCount = repository.calls
                .where((c) => c['kind'] == 'items')
                .length;
            repository.decisionHold!.complete();
            await tester.pumpAndSettle();
            expect(observation(tester).selected, expected);
            expect(observation(tester).rows.first.position, position);
            expect(
              repository.calls.where((c) => c['kind'] == 'items').length,
              readCount,
            );
            await tap(
              tester,
              rowAction(expected, accept ? 'Accept current state' : 'Retry'),
            );
            await wait(
              tester,
              () =>
                  repository.decisions.length == 2 &&
                  !observation(tester).itemsLoading,
            );
            expect(repository.decisions.last, (
              accept ? 'accept' : 'retry',
              expected,
              position + (accept ? 1 : 0),
              '$expected-mail-${position + (accept ? 1 : 0)}',
            ));
            expect(observation(tester).rows.length, lessThanOrEqualTo(50));
          });
        },
      );
    }
  }

  for (final dark in [false, true]) {
    testWidgets(
      'bounded History paging, failed-page Retry and compact ${dark ? 'dark' : 'light'} capture',
      (tester) async {
        final repository = HistoryRepository(items: 100);
        await scenario(tester, repository, (workspace) async {
          expect(observation(tester).jobs.length, 20);
          for (var i = 0; i < 3; i++) {
            await tap(tester, find.text('Older groups'));
            await wait(tester, () => !observation(tester).loading);
            expect(observation(tester).jobs.length, lessThanOrEqualTo(20));
          }
          expect(observation(tester).jobs.length, 5);
          expect(
            tester
                .widget<TextButton>(
                  find.widgetWithText(TextButton, 'Older groups'),
                )
                .onPressed,
            isNull,
          );
          expect(repository.records.length, 65);
          await tap(tester, find.text('Move to group-004 100 messages'));
          await wait(tester, () => !observation(tester).itemsLoading);
          repository.failItems = true;
          await tap(tester, find.text('Next 50 messages'));
          await wait(tester, () => !observation(tester).itemsLoading);
          expect(observation(tester).rows, isEmpty);
          expect(find.text('Retry messages'), findsOneWidget);
          await tester.pumpAndSettle();
          await tester.scrollUntilVisible(
            find.text('Move to group-004 100 messages'),
            -500,
            scrollable: find.byType(Scrollable).last,
            maxScrolls: 100,
          );
          await tester.ensureVisible(
            find.text('Move to group-004 100 messages'),
          );
          await tester.pumpAndSettle();
          await expectLater(
            find.byType(MaterialApp),
            matchesGoldenFile(
              'goldens/group_history_retry_${dark ? 'dark' : 'light'}.png',
            ),
          );
          await tap(tester, find.text('Retry messages'));
          await wait(tester, () => !observation(tester).itemsLoading);
          expect(observation(tester).rows.length, 50);
          expect(observation(tester).rows.first.position, 50);
          await tester.scrollUntilVisible(
            find.text('Move to group-004 100 messages'),
            -500,
            scrollable: find.byType(Scrollable).last,
            maxScrolls: 100,
          );
          await tester.ensureVisible(
            find.text('Move to group-004 100 messages'),
          );
          await tester.pump();
          await tester.scrollUntilVisible(
            find.text('Next 50 messages'),
            500,
            scrollable: find.byType(Scrollable).last,
            maxScrolls: 100,
          );
          await tester.pump();
          expect(
            tester
                .widget<TextButton>(
                  find.widgetWithText(TextButton, 'Next 50 messages'),
                )
                .onPressed,
            isNull,
          );
          await tap(tester, find.text('Previous 50 messages'));
          await wait(tester, () => !observation(tester).itemsLoading);
          expect(observation(tester).rows.first.position, 0);
          await tap(tester, find.text('Newer groups'));
          await wait(tester, () => !observation(tester).loading);
          expect(observation(tester).jobs.length, 20);
          expect(observation(tester).rows, isEmpty);
        }, dark: dark);
      },
    );
  }

  testWidgets(
    'Pause, Undo, Resume and close remain independent of held detail reads',
    (tester) async {
      final repository = HistoryRepository();
      repository.records['group-064']!['state'] = 'running';
      repository.members['group-064']![0]['state'] = 'done';
      await scenario(tester, repository, (workspace) async {
        repository.heldItemsJob = 'group-064';
        repository.itemsHold = Completer<void>();
        await tap(tester, find.text('Move to group-064 125 messages'));
        expect(observation(tester).itemsLoading, true);
        await tap(tester, rowAction('group-064', 'Pause'));
        await wait(
          tester,
          () => repository.records['group-064']!['state'] == 'paused',
        );
        await tap(tester, rowAction('group-064', 'Undo'));
        await wait(
          tester,
          () => repository.records['group-064']!['undo'] == true,
        );
        await tap(tester, rowAction('group-064', 'Resume'));
        await wait(
          tester,
          () => repository.records['group-064']!['state'] == 'running',
        );
        await tester.pageBack();
        await tester.pumpAndSettle();
        final reads = repository.calls
            .where((c) => c['kind'] == 'items')
            .length;
        repository.itemsHold!.complete();
        await tester.pumpAndSettle();
        expect(find.text('Open group History'), findsOneWidget);
        expect(
          repository.calls.where((c) => c['kind'] == 'items').length,
          reads,
        );
      });
    },
  );

  testWidgets('rapid real group choices coalesce held detail reads', (
    tester,
  ) async {
    final repository = HistoryRepository();
    await scenario(tester, repository, (workspace) async {
      repository.heldItemsJob = 'group-064';
      repository.itemsHold = Completer<void>();
      await tap(tester, find.text('Move to group-064 125 messages'));
      for (var n = 0; n < 3; n++) {
        await tap(tester, find.text('Move to group-063 125 messages'));
        await tap(
          tester,
          find.text('Move to group-064 125 messages'),
          delta: -500,
        );
      }
      await tap(tester, find.text('Move to group-063 125 messages'));
      expect(repository.calls.where((c) => c['kind'] == 'items').length, 1);
      repository.itemsHold!.completeError(StateError('Old item failure'));
      await wait(tester, () => !observation(tester).itemsLoading);
      expect(repository.calls.where((c) => c['kind'] == 'items').length, 2);
      expect(observation(tester).itemsError, isNull);
      await tap(tester, rowAction('group-063', 'Retry'));
      await wait(tester, () => repository.decisions.isNotEmpty);
      expect(repository.decisions.single, (
        'retry',
        'group-063',
        0,
        'group-063-mail-0',
      ));
    });
  });

  testWidgets('older attention banner clears through real Accept and Remove', (
    tester,
  ) async {
    final repository = HistoryRepository();
    for (final rows in repository.members.values) {
      for (final row in rows) {
        row['state'] = 'done';
      }
    }
    repository.members['group-000']![0]['state'] = 'uncertain';
    repository.members['group-000']![1]['state'] = 'failed';
    await scenario(tester, repository, (workspace) async {
      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.text('2 group action steps need review.'), findsOneWidget);
      await tap(tester, find.widgetWithText(TextButton, 'History'));
      await wait(
        tester,
        () =>
            find.byType(GroupHistoryScreen).evaluate().isNotEmpty &&
            !observation(tester).itemsLoading &&
            !observation(tester).loading,
      );
      await tester.pumpAndSettle();
      expect(observation(tester).selected, 'group-000');
      await tap(tester, rowAction('group-000', 'Accept current state'));
      await wait(tester, () => workspace.groups!.attentionCount == 1);
      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.text('1 group action step needs review.'), findsOneWidget);
      await tap(tester, find.widgetWithText(TextButton, 'History'));
      await wait(
        tester,
        () =>
            find.byType(GroupHistoryScreen).evaluate().isNotEmpty &&
            !observation(tester).loading,
      );
      await tester.pumpAndSettle();
      await tap(tester, rowAction('group-000', 'Remove'));
      await wait(tester, () => workspace.groups!.attentionCount == 0);
      await tester.pageBack();
      await tester.pumpAndSettle();
      expect(find.textContaining('group action step'), findsNothing);
      expect(workspace.groups!.attentionTarget, isNull);
      expect(repository.decisions.single, (
        'accept',
        'group-000',
        0,
        'group-000-mail-0',
      ));
    }, attentionBanner: true);
  });

  testWidgets('two real Remove controls fence a held Refresh page', (
    tester,
  ) async {
    final repository = HistoryRepository();
    await scenario(tester, repository, (workspace) async {
      repository.historyHold = Completer<void>();
      await tap(tester, find.byTooltip('Refresh History'));
      expect(observation(tester).loading, true);
      await tap(tester, rowAction('group-064', 'Remove'));
      await wait(tester, () => !repository.records.containsKey('group-064'));
      await tap(tester, rowAction('group-063', 'Remove'));
      await wait(tester, () => !repository.records.containsKey('group-063'));
      repository.historyHold!.complete();
      await wait(tester, () => !observation(tester).loading);
      expect(card('group-064'), findsNothing);
      expect(card('group-063'), findsNothing);
      expect(observation(tester).jobs.length, 20);
      expect(repository.records.length, 63);
    });
  });
}
