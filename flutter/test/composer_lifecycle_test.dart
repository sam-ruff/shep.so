import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/model/mail.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/data/settings_store.dart';
import 'package:shep_mobile/ui/composer.dart';
import 'package:shep_mobile/ui/app.dart';
import 'package:shep_mobile/ui/theme.dart';

import 'support/preview_repository.dart';
import 'support/paged_repository.dart';

class MemorySettings implements SettingsStore {
  Preferences value = const Preferences();

  @override
  Future<Preferences> read() async => value;

  @override
  Future<void> write(Preferences preferences) async => value = preferences;
}

Future<void> loadPreviewFonts() async {
  await (FontLoader(
    'Roboto',
  )..addFont(rootBundle.load('assets/Roboto-Regular.ttf'))).load();
  await (FontLoader(
    'NotoSans',
  )..addFont(rootBundle.load('assets/NotoSans-Regular.ttf'))).load();
  await (FontLoader(
    'NotoSans',
  )..addFont(rootBundle.load('assets/NotoSans-SemiBold.ttf'))).load();
}

class DraftLifecycleRepository extends PreviewRepository {
  DraftLifecycleRepository() : super(delay: Duration.zero);

  final saves = <Draft>[];
  final gates = <Completer<void>>[];
  bool sendSucceeds = false;

  @override
  Future<void> saveDraft(Draft draft) async {
    saves.add(draft);
    final gate = Completer<void>();
    gates.add(gate);
    await gate.future;
  }

  @override
  Future<void> send(Draft draft) async {
    if (!sendSucceeds) return super.send(draft);
  }
}

class RefreshDraftRepository extends PagedRepository {
  final initialized = Completer<void>();
  List<Draft> nativeDrafts = const [];

  @override
  List<Draft> get savedDrafts => nativeDrafts;

  @override
  Future<void> initialize() => initialized.future;
}

Future<(Workspace, DraftLifecycleRepository)> showComposer(
  WidgetTester tester, {
  Draft draft = const Draft(id: 'draft'),
}) async {
  await loadPreviewFonts();
  final repository = DraftLifecycleRepository();
  final workspace = Workspace(repository, MemorySettings());
  await tester.pumpWidget(
    MaterialApp(
      theme: shepTheme(Brightness.light),
      home: Composer(workspace: workspace, draft: draft),
    ),
  );
  return (workspace, repository);
}

Finder subjectField() => find.byWidgetPredicate(
  (widget) => widget is TextField && widget.decoration?.labelText == 'Subject',
);

void main() {
  test('reverse save completion retains the newest draft revision', () async {
    final repository = DraftLifecycleRepository();
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    final first = workspace.saveDraft(
      const Draft(id: 'draft', subject: 'Older', revision: 1),
    );
    final second = workspace.saveDraft(
      const Draft(id: 'draft', subject: 'Current', revision: 2),
    );
    repository.gates[1].complete();
    await second;
    repository.gates[0].complete();
    await first;
    expect(workspace.drafts['draft']!.subject, 'Current');
    expect(workspace.drafts['draft']!.revision, 2);
  });

  test('native refresh cannot overwrite a newer pending snapshot', () async {
    final repository = RefreshDraftRepository()
      ..nativeDrafts = const [
        Draft(id: 'draft', subject: 'Saved old text', revision: 1),
      ];
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    final initializing = workspace.initialize();
    workspace.stageDraft(
      const Draft(id: 'draft', subject: 'Pending current text', revision: 2),
    );
    repository.initialized.complete();
    await initializing;
    expect(workspace.drafts['draft']!.subject, 'Pending current text');
  });

  test('held save cannot resurrect a draft after Send', () async {
    final repository = DraftLifecycleRepository()..sendSucceeds = true;
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    const draft = Draft(id: 'draft', subject: 'Send me', revision: 1);
    final saving = workspace.saveDraft(draft);
    expect(repository.gates, hasLength(1));
    expect(await workspace.send(draft), isTrue);
    repository.gates.single.completeError(StateError('late failure'));
    await saving;
    expect(workspace.drafts, isNot(contains('draft')));
    expect(workspace.draftNeedsRetry('draft'), isFalse);
  });

  testWidgets('Drafts Retry saves the retained failed snapshot', (
    tester,
  ) async {
    final repository = DraftLifecycleRepository();
    final workspace = Workspace(repository, MemorySettings());
    addTearDown(workspace.dispose);
    const draft = Draft(id: 'draft', subject: 'Retained', revision: 1);
    final saving = workspace.saveDraft(draft);
    repository.gates.single.completeError(StateError('disk full'));
    await saving;
    workspace.navigate('Drafts');
    await tester.pumpWidget(MaterialApp(home: Home(workspace: workspace)));
    await tester.pump();
    expect(find.text('Not saved. Open or retry this draft.'), findsOneWidget);
    await tester.tap(find.widgetWithText(TextButton, 'Retry'));
    await tester.pump();
    expect(repository.gates, hasLength(2));
    repository.gates.last.complete();
    await tester.pump();
    await tester.pump();
    expect(find.text('Not saved. Open or retry this draft.'), findsNothing);
  });

  testWidgets('autosave failure exposes Retry and the saved revision', (
    tester,
  ) async {
    final (workspace, repository) = await showComposer(tester);
    addTearDown(workspace.dispose);
    await tester.enterText(subjectField(), 'First subject');
    await tester.pump(const Duration(milliseconds: 500));
    expect(find.text('Saving draft…'), findsOneWidget);
    repository.gates.single.completeError(StateError('disk full'));
    await tester.pump();
    await tester.pump();
    expect(find.text('Draft not saved'), findsOneWidget);
    expect(find.widgetWithText(TextButton, 'Retry'), findsOneWidget);
    await expectLater(
      find.byType(Scaffold),
      matchesGoldenFile('goldens/composer_save_failure_light.png'),
    );
    await tester.tap(find.widgetWithText(TextButton, 'Retry'));
    await tester.pump();
    expect(repository.saves, hasLength(2));
    expect(repository.saves.last.revision, repository.saves.first.revision);
    repository.gates.last.complete();
    await tester.pump();
    await tester.pump();
    expect(find.text('Saved'), findsOneWidget);
  });

  testWidgets('late save cannot replace a newer queued revision', (
    tester,
  ) async {
    final (workspace, repository) = await showComposer(tester);
    addTearDown(workspace.dispose);
    final subject = subjectField();
    await tester.enterText(subject, 'Older');
    await tester.pump(const Duration(milliseconds: 500));
    await tester.enterText(subject, 'Current');
    await tester.pump(const Duration(milliseconds: 500));
    repository.gates.first.complete();
    await tester.pump();
    await tester.pump();
    expect(find.text('Saved'), findsNothing);
    expect(repository.saves, hasLength(2));
    repository.gates.last.complete();
    await tester.pump();
    await tester.pump();
    expect(find.text('Saved'), findsOneWidget);
    expect(workspace.drafts['draft']!.subject, 'Current');
    expect(workspace.drafts['draft']!.revision, repository.saves.last.revision);
  });

  testWidgets('suspension flushes the current revision before debounce', (
    tester,
  ) async {
    final (workspace, repository) = await showComposer(tester);
    addTearDown(workspace.dispose);
    await tester.enterText(subjectField(), 'Parked');
    await tester.pump();
    expect(find.text('Unsaved changes'), findsOneWidget);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    await tester.pump();
    expect(repository.saves.single.subject, 'Parked');
    repository.gates.single.complete();
    await tester.pump();
  });

  testWidgets('an unsaved revision zero draft is admitted on suspension', (
    tester,
  ) async {
    final (workspace, repository) = await showComposer(
      tester,
      draft: const Draft(id: 'reply', subject: 'Re: Saved context'),
    );
    addTearDown(workspace.dispose);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    await tester.pump();
    expect(repository.saves.single.id, 'reply');
    expect(repository.saves.single.revision, 0);
    repository.gates.single.complete();
    await tester.pump();
  });

  testWidgets('failed disposal save is retained for reopen and Retry', (
    tester,
  ) async {
    final (workspace, repository) = await showComposer(tester);
    addTearDown(workspace.dispose);
    await tester.enterText(subjectField(), 'Navigate away');
    await tester.pump();
    expect(find.text('Unsaved changes'), findsOneWidget);
    await tester.pumpWidget(const MaterialApp(home: SizedBox()));
    await tester.pump();
    expect(repository.saves.single.subject, 'Navigate away');
    repository.gates.single.completeError(StateError('disk full'));
    await tester.pump();
    await tester.pump();
    expect(workspace.drafts['draft']!.subject, 'Navigate away');
    expect(workspace.draftNeedsRetry('draft'), isTrue);
    await tester.pumpWidget(
      MaterialApp(
        home: Composer(workspace: workspace, draft: workspace.drafts['draft']!),
      ),
    );
    await tester.pump();
    expect(find.text('Draft not saved'), findsOneWidget);
    expect(subjectField(), findsOneWidget);
    expect(
      tester.widget<TextField>(subjectField()).controller!.text,
      'Navigate away',
    );
  });
}
