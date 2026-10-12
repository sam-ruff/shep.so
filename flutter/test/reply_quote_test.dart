import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/drafts.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/composer.dart';
import 'package:shep_mobile/ui/preferences.dart';
import 'package:shep_mobile/ui/theme.dart';
import 'support/paged_repository.dart';
import 'composer_lifecycle_test.dart' show MemorySettings, loadPreviewFonts;

const original = ReplyContext(
  accountId: 'fixture',
  mailId: 'mail-1',
  quote: '\n\nOn yesterday, Sender wrote:\n> Original message text',
);
const prepared = Draft(
  id: 'reply',
  accountId: 'fixture',
  to: 'sender@example.test',
  subject: 'Re: Plan',
  inReplyTo: '<original@example.test>',
  references: ['<root@example.test>', '<original@example.test>'],
  replyContext: original,
);

class QuoteRepository extends PagedRepository implements DraftRepository {
  final saves = <Draft>[];
  Completer<void>? saveGate;
  Completer<Draft>? replyGate;
  @override
  Future<void> saveDraft(Draft draft) async {
    saves.add(draft);
    if (saves.length == 1 && saveGate != null) await saveGate!.future;
  }

  @override
  Future<Draft> reply(String id, bool all) async =>
      replyGate == null ? prepared : replyGate!.future;
  @override
  Future<DraftFiles> files(String id) async => const DraftFiles([], 0);
  @override
  Future<DraftFiles> addFiles(
    String id,
    List<SelectedAttachment> selected,
  ) async => const DraftFiles([], 1);
  @override
  Future<DraftFiles> removeFile(String id, String file) async =>
      const DraftFiles([], 2);
}

Future<void> showComposer(
  WidgetTester tester,
  Workspace workspace,
  Draft draft,
) async {
  await tester.pumpWidget(
    MaterialApp(
      home: Composer(workspace: workspace, draft: draft),
    ),
  );
  await tester.pump();
  await tester.pump();
}

Future<void> reveal(
  WidgetTester tester,
  Finder target, {
  bool back = false,
}) async {
  await tester.scrollUntilVisible(
    target,
    back ? -300 : 300,
    scrollable: find.byType(Scrollable).first,
  );
  await tester.pump();
}

void main() {
  test(
    'default is captured before held reply preparation and affects only new replies',
    () async {
      final repo = QuoteRepository();
      final actual = Workspace(repo, MemorySettings());
      addTearDown(actual.dispose);
      repo.replyGate = Completer<Draft>();
      final pending = actual.reply(repo.cached.first.id, true);
      actual.preferences = actual.preferences.copy(replyIncludeOriginal: false);
      repo.replyGate!.complete(prepared);
      final first = await pending;
      expect(first?.replyContext?.includeQuote, isTrue);
      repo.replyGate = null;
      expect(
        (await actual.reply(
          repo.cached.first.id,
          false,
        ))?.replyContext?.includeQuote,
        isFalse,
      );
      expect(first?.replyContext?.includeQuote, isTrue);
      expect(first?.body, isEmpty);
    },
  );

  testWidgets(
    'visible reply quote control preserves typed text and exact references',
    (tester) async {
      final repo = QuoteRepository();
      final workspace = Workspace(repo, MemorySettings());
      await showComposer(tester, workspace, prepared);
      expect(find.text('Include original message'), findsOneWidget);
      expect(find.textContaining('Original message text'), findsOneWidget);
      final message = find.widgetWithText(TextField, 'Message');
      await reveal(tester, message);
      await tester.enterText(message, 'My typed answer');
      final toggle = find.byType(CheckboxListTile);
      await reveal(tester, toggle, back: true);
      await tester.tap(toggle);
      await tester.pump(const Duration(milliseconds: 600));
      expect(repo.saves.last.body, 'My typed answer');
      expect(repo.saves.last.replyContext?.includeQuote, isFalse);
      expect(repo.saves.last.replyContext?.quote, original.quote);
      expect(repo.saves.last.references, prepared.references);
      expect(repo.saves.last.inReplyTo, prepared.inReplyTo);
      await tester.tap(toggle);
      await tester.pump(const Duration(milliseconds: 600));
      expect(repo.saves.last.body, 'My typed answer');
      expect(repo.saves.last.replyContext?.includeQuote, isTrue);
      await tester.pumpWidget(const SizedBox.shrink());
      workspace.dispose();
    },
  );

  testWidgets('held older save cannot replace newer quote choice or text', (
    tester,
  ) async {
    final repo = QuoteRepository()..saveGate = Completer<void>();
    final workspace = Workspace(repo, MemorySettings());
    await showComposer(tester, workspace, prepared);
    final message = find.widgetWithText(TextField, 'Message');
    await reveal(tester, message);
    await tester.enterText(message, 'First text');
    await tester.pump(const Duration(milliseconds: 600));
    expect(repo.saves, hasLength(1));
    final toggle = find.byType(CheckboxListTile);
    await reveal(tester, toggle, back: true);
    await tester.tap(toggle);
    await reveal(tester, message);
    await tester.enterText(message, 'Newer text');
    await tester.pump(const Duration(milliseconds: 600));
    repo.saveGate!.complete();
    await tester.pump();
    await tester.pump();
    expect(repo.saves.last.body, 'Newer text');
    expect(repo.saves.last.replyContext?.includeQuote, isFalse);
    expect(workspace.drafts['reply']?.replyContext?.includeQuote, isFalse);
    await tester.pumpWidget(const SizedBox.shrink());
    workspace.dispose();
  });

  testWidgets(
    'legacy reply body stays unchanged without guessed quote context',
    (tester) async {
      final repo = QuoteRepository();
      final workspace = Workspace(repo, MemorySettings());
      const legacy = Draft(
        id: 'legacy',
        body: 'Typed text\n\n> Old inline quote',
        inReplyTo: '<original@example.test>',
      );
      await showComposer(tester, workspace, legacy);
      expect(find.text('Include original message'), findsNothing);
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Message'))
            .controller
            ?.text,
        legacy.body,
      );
      await tester.pumpWidget(const SizedBox.shrink());
      workspace.dispose();
    },
  );

  testWidgets(
    'Preferences exposes the new-reply default and preserves saved draft choice',
    (tester) async {
      final repo = QuoteRepository();
      final workspace = Workspace(repo, MemorySettings());
      workspace.drafts[prepared.id] = prepared;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(body: PreferencesView(workspace: workspace)),
        ),
      );
      final setting = find.byKey(const ValueKey('preference-reply-original'));
      await tester.ensureVisible(setting);
      await tester.tap(setting);
      await tester.pump();
      await tester.pump();
      expect(workspace.preferences.replyIncludeOriginal, isFalse);
      expect(workspace.drafts[prepared.id]?.replyContext?.includeQuote, isTrue);
      await tester.pumpWidget(const SizedBox.shrink());
      workspace.dispose();
    },
  );

  for (final dark in [false, true]) {
    testWidgets('compact reply original preview ${dark ? 'dark' : 'light'}', (
      tester,
    ) async {
      await loadPreviewFonts();
      tester.view.physicalSize = const Size(390, 760);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final workspace = Workspace(QuoteRepository(), MemorySettings());
      await tester.pumpWidget(
        MaterialApp(
          debugShowCheckedModeBanner: false,
          theme: shepTheme(dark ? Brightness.dark : Brightness.light),
          home: Composer(workspace: workspace, draft: prepared),
        ),
      );
      await tester.pumpAndSettle();
      await expectLater(
        find.byType(MaterialApp),
        matchesGoldenFile(
          'goldens/reply_original_compact_${dark ? 'dark' : 'light'}.png',
        ),
      );
      await tester.pumpWidget(const SizedBox.shrink());
      workspace.dispose();
    });
  }
}
