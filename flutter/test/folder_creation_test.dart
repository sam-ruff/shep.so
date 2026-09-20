import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/folders.dart';
import 'package:shep_mobile/model/folder_creations.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'package:shep_mobile/ui/folder_creations.dart';
import 'package:shep_mobile/ui/theme.dart';
import 'mail_activity_visual_test.dart' show loadPreviewFonts, VisualSettings;
import 'support/preview_repository.dart';

class FolderFixture extends PreviewRepository
    implements FolderCreationRepository {
  FolderFixture() : super(delay: Duration.zero);
  final account = const FolderAccount({
    'account': 'account-a',
    'connection': 'saved',
    'label': 'Personal',
    'email': 'alex@example.test',
    'names': ['INBOX'],
  });
  final entries = <FolderCreation>[];
  final provider = Completer<void>();
  final submissions = <String>[];
  Completer<void>? admissionGate;
  bool admissionFailure = false, lostAdmissionReply = false;
  int executions = 0;
  @override
  Future<List<FolderAccount>> folderOptions() async => [account];
  @override
  Future<List<FolderCreation>> folderCreations() async => List.of(entries);
  @override
  Future<FolderCreation> admitFolder(
    String id,
    FolderAccount account,
    String? parent,
    String name,
  ) async {
    submissions.add(id);
    if (admissionFailure) throw StateError('disk full');
    final result =
        entries.where((entry) => entry.id == id).firstOrNull ??
        FolderCreation({
          'id': id,
          'account': account.id,
          'name': name,
          'parent': parent,
          'status': 'queued',
          'revision': 1,
          'acknowledged': false,
        });
    entries.removeWhere((entry) => entry.id == id);
    entries.add(result);
    if (lostAdmissionReply) throw StateError('reply lost');
    await admissionGate?.future;
    return result;
  }

  @override
  Future<FolderCreation> executeFolder(FolderCreation request) async {
    executions++;
    await provider.future;
    return request;
  }

  @override
  Future<FolderCreation> decideFolder(
    FolderCreation request,
    String decision,
  ) async {
    final result = FolderCreation({
      ...request.data,
      'revision': request.revision + 1,
      'status': switch (decision) {
        'cancel' => 'cancelled',
        'check' => 'checking',
        'retry' => 'queued',
        _ => 'dismissed',
      },
    });
    entries.removeWhere((entry) => entry.id == request.id);
    entries.add(result);
    return result;
  }
}

Future<FolderCreations> controls(
  WidgetTester tester,
  FolderFixture repository, {
  bool dark = false,
}) async {
  await loadPreviewFonts();
  tester.view.physicalSize = const Size(390, 700);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.resetPhysicalSize);
  addTearDown(tester.view.resetDevicePixelRatio);
  final controller = FolderCreations(repository, changed: () {});
  addTearDown(controller.dispose);
  await controller.refresh();
  await tester.pumpWidget(
    MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: shepTheme(dark ? Brightness.dark : Brightness.light),
      home: Builder(
        builder: (context) => Scaffold(
          body: TextButton(
            onPressed: () => showNewFolder(context, controller),
            child: const Text('New folder'),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('New folder'));
  await tester.pumpAndSettle();
  return controller;
}

void main() {
  testWidgets(
    'drawer keeps admitted folder beneath its account without enabling navigation',
    (tester) async {
      await loadPreviewFonts();
      tester.view.physicalSize = const Size(390, 700);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final repository = FolderFixture();
      final workspace = Workspace(repository, VisualSettings());
      try {
        await workspace.initialize();
        await tester.pumpWidget(ShepApp(workspace: workspace));
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Open navigation menu'));
        await tester.pumpAndSettle();
        await tester.scrollUntilVisible(
          find.text('New folder'),
          200,
          scrollable: find.descendant(
            of: find.byType(Drawer),
            matching: find.byType(Scrollable),
          ),
        );
        await tester.tap(find.text('New folder'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Account root'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('INBOX').last);
        await tester.pumpAndSettle();
        await tester.enterText(find.byType(TextField), 'Projects');
        await tester.pumpAndSettle();
        await tester.tap(find.text('Create folder'));
        await tester.pumpAndSettle();
        expect(repository.provider.isCompleted, isFalse);
        expect(repository.entries.single.parent, 'INBOX');
        await tester.tap(find.byTooltip('Open navigation menu'));
        await tester.pumpAndSettle();
        await tester.scrollUntilVisible(
          find.text('Projects'),
          150,
          scrollable: find.descendant(
            of: find.byType(Drawer),
            matching: find.byType(Scrollable),
          ),
        );
        expect(
          find.descendant(
            of: find.byType(Drawer),
            matching: find.text('Personal'),
          ),
          findsOneWidget,
        );
        expect(find.text('Queued'), findsOneWidget);
        expect(workspace.folders, isNot(contains('Projects')));
        expect(
          find.ancestor(
            of: find.text('Projects'),
            matching: find.byType(InkWell),
          ),
          findsNothing,
        );
        await expectLater(
          find.byType(MaterialApp),
          matchesGoldenFile('goldens/folder_pending_sidebar_light.png'),
        );
      } finally {
        await tester.pumpWidget(const SizedBox.shrink());
        workspace.dispose();
      }
    },
  );
  testWidgets('local admission closes form before held provider finishes', (
    tester,
  ) async {
    final repository = FolderFixture();
    final controller = await controls(tester, repository);
    await tester.enterText(find.byType(TextField), 'Projects');
    await tester.pumpAndSettle();
    await expectLater(
      find.byType(MaterialApp),
      matchesGoldenFile('goldens/folder_creation_light.png'),
    );
    await tester.tap(find.text('Create folder'));
    await tester.pumpAndSettle();
    expect(find.byType(NewFolderDialog), findsNothing);
    expect(controller.entries.single.name, 'Projects');
    expect(controller.entries.single.status, 'queued');
    expect(repository.executions, 1);
    expect(repository.provider.isCompleted, isFalse);
  });

  testWidgets(
    'lost admission response keeps name and retries exact request identity',
    (tester) async {
      final repository = FolderFixture()..lostAdmissionReply = true;
      final controller = await controls(tester, repository, dark: true);
      await tester.enterText(find.byType(TextField), 'Projects');
      await tester.pump();
      await tester.tap(find.text('Create folder'));
      await tester.pumpAndSettle();
      expect(find.textContaining('Your name has been kept'), findsOneWidget);
      expect(find.text('Projects'), findsOneWidget);
      repository.lostAdmissionReply = false;
      await tester.tap(find.text('Create folder'));
      await tester.pumpAndSettle();
      expect(repository.submissions, hasLength(2));
      expect(repository.submissions[0], repository.submissions[1]);
      expect(controller.entries, hasLength(1));
    },
  );

  testWidgets(
    'unknown creation exposes check and stop tracking but no blind retry',
    (tester) async {
      final repository = FolderFixture();
      final controller = await controls(tester, repository, dark: true);
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      repository.entries.add(
        const FolderCreation({
          'id': 'unknown',
          'account': 'account-a',
          'name': 'Projects',
          'parent': 'INBOX',
          'status': 'uncertain',
          'revision': 3,
          'acknowledged': false,
          'error': 'The server response was lost. Check the saved folder.',
        }),
      );
      await controller.refresh();
      await tester.pumpWidget(
        MaterialApp(
          debugShowCheckedModeBanner: false,
          theme: shepTheme(Brightness.dark),
          home: FolderActivityScreen(controller: controller),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Check server'), findsOneWidget);
      expect(find.text('Retry'), findsNothing);
      await expectLater(
        find.byType(MaterialApp),
        matchesGoldenFile('goldens/folder_recovery_dark.png'),
      );
      await tester.tap(find.text('Stop tracking'));
      await tester.pumpAndSettle();
      expect(repository.entries.single.status, 'dismissed');
      expect(repository.executions, 0);
    },
  );

  test('foreground resumption keeps one provider call per account', () async {
    final repository = FolderFixture();
    final controller = FolderCreations(repository, changed: () {});
    addTearDown(controller.dispose);
    for (final name in ['First', 'Second']) {
      await repository.admitFolder(
        FolderCreations.identity(),
        repository.account,
        null,
        name,
      );
    }
    await controller.refresh(resume: true);
    await controller.refresh(resume: true);
    expect(repository.executions, 1);
    controller.foreground(false);
    repository.provider.complete();
    await Future<void>.delayed(Duration.zero);
    expect(repository.executions, 1);
  });

  test(
    'late provider failure cannot republish attention after cancellation',
    () async {
      final repository = FolderFixture();
      final controller = FolderCreations(repository, changed: () {});
      addTearDown(controller.dispose);
      await controller.admit(
        FolderCreations.identity(),
        repository.account,
        null,
        'Projects',
      );
      await controller.decide(controller.entries.single, 'cancel');
      repository.provider.completeError(StateError('late provider failure'));
      await Future<void>.delayed(Duration.zero);
      expect(controller.entries.single.status, 'cancelled');
      expect(controller.failures, isEmpty);
    },
  );
  test(
    'close or background during admission cannot start new provider work',
    () async {
      for (final closing in [true, false]) {
        final repository = FolderFixture()..admissionGate = Completer<void>();
        final controller = FolderCreations(repository, changed: () {});
        final pending = controller.admit(
          FolderCreations.identity(),
          repository.account,
          null,
          'Saved before close',
        );
        await Future<void>.delayed(Duration.zero);
        expect(repository.entries, hasLength(1));
        if (closing) {
          controller.dispose();
        } else {
          controller.foreground(false);
        }
        repository.admissionGate!.complete();
        await pending;
        expect(repository.executions, 0);
        expect(repository.entries.single.status, 'queued');
        await expectLater(
          controller.admit(
            FolderCreations.identity(),
            repository.account,
            null,
            'Stale input',
          ),
          throwsStateError,
        );
        expect(repository.entries, hasLength(1));
        if (!closing) {
          controller.foreground(true);
          await controller.refresh(resume: true);
          expect(repository.executions, 1);
          controller.dispose();
        }
      }
    },
  );
}
