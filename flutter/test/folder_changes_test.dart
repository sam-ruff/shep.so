import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shep_mobile/data/folders.dart';
import 'package:shep_mobile/model/folder_creations.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'package:shep_mobile/ui/folder_changes.dart';
import 'package:shep_mobile/ui/folder_creations.dart';
import 'package:shep_mobile/ui/theme.dart';
import 'folder_creation_test.dart' show FolderFixture;
import 'mail_activity_visual_test.dart' show loadPreviewFonts, VisualSettings;

class ChangeFixture extends FolderFixture implements FolderChangeRepository {
  @override
  FolderAccount get account => const FolderAccount({
    'account': 'account-a',
    'connection': 'saved',
    'label': 'Personal',
    'email': 'alex@example.test',
    'names': ['INBOX', 'Projects', 'Projects/Design', 'Elsewhere'],
  });
  @override
  Future<Map<String, dynamic>> reviewFolderChange(
    FolderAccount account,
    String source,
    Object action,
  ) async => {
    'account': account.id,
    'connection': account.connection,
    'messages': 10,
    'fingerprint': 'frozen',
    'plan': {
      'source': source,
      'action': action,
      'parent': null,
      'members': [
        for (final path in ['Projects/Design', 'Projects'])
          {
            'path': path,
            'mailbox': {'name': path},
            'destination': action == 'Delete'
                ? null
                : (action as Map).containsKey('Rename')
                ? path.replaceFirst(
                    'Projects',
                    (action['Rename'] as Map)['name'] as String,
                  )
                : 'Elsewhere/$path',
          },
      ],
    },
  };
  @override
  Future<FolderCreation> admitFolderChange(
    String id,
    Map<String, dynamic> review,
  ) async {
    final saved = await admitFolder(
      id,
      account,
      null,
      review['plan']['source'] as String,
    );
    final changed = FolderCreation({
      ...saved.data,
      'mutation': {
        'review': review,
        'completed': 0,
        'receipt': null,
        'observed': false,
        'checked': false,
      },
    });
    entries.removeWhere((entry) => entry.id == id);
    entries.add(changed);
    return changed;
  }
}

Future<FolderCreations> open(
  WidgetTester tester,
  ChangeFixture repository, {
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
            onPressed: () => showFolderChange(context, controller),
            child: const Text('Manage folders'),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Manage folders'));
  await tester.pumpAndSettle();
  await tester.tap(
    find.widgetWithText(DropdownButtonFormField<String>, 'Folder'),
  );
  await tester.pumpAndSettle();
  await tester.tap(find.text('Projects').last);
  await tester.pumpAndSettle();
  return controller;
}

void main() {
  testWidgets(
    'actual drawer exposes accessible Manage folders and queued cancellation',
    (tester) async {
      await loadPreviewFonts();
      tester.view.physicalSize = const Size(390, 700);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final repository = ChangeFixture();
      final workspace = Workspace(repository, VisualSettings());
      try {
        await workspace.initialize();
        await tester.pumpWidget(ShepApp(workspace: workspace));
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Open navigation menu'));
        await tester.pumpAndSettle();
        await tester.scrollUntilVisible(
          find.text('Manage folders'),
          150,
          scrollable: find.descendant(
            of: find.byType(Drawer),
            matching: find.byType(Scrollable),
          ),
        );
        await tester.tap(find.text('Manage folders'));
        await tester.pumpAndSettle();
        expect(find.byType(FolderChangeDialog), findsOneWidget);
        await tester.tap(
          find.widgetWithText(DropdownButtonFormField<String>, 'Folder'),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.text('Projects').last);
        await tester.pumpAndSettle();
        await tester.enterText(find.byType(TextField), 'Work');
        await tester.tap(find.text('Review change'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Confirm Rename'));
        await tester.pumpAndSettle();
        expect(repository.entries.single.name, 'Work');
        expect(repository.provider.isCompleted, isFalse);
        await tester.tap(find.byTooltip('Open navigation menu'));
        await tester.pumpAndSettle();
        await tester.scrollUntilVisible(
          find.text('Folder activity'),
          150,
          scrollable: find.descendant(
            of: find.byType(Drawer),
            matching: find.byType(Scrollable),
          ),
        );
        await tester.tap(find.text('Folder activity'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Cancel request'));
        await tester.pumpAndSettle();
        expect(find.text('Cancelled'), findsOneWidget);
      } finally {
        await tester.pumpWidget(const SizedBox.shrink());
        workspace.dispose();
      }
    },
  );
  for (final dark in [false, true]) {
    testWidgets(
      'checked ${dark ? 'delete dark' : 'rename light'} closes before provider and retains pending logical result',
      (tester) async {
        final repository = ChangeFixture();
        final controller = await open(tester, repository, dark: dark);
        if (dark) {
          await tester.tap(find.text('Rename'));
          await tester.pumpAndSettle();
          await tester.tap(find.text('Delete').last);
          await tester.pumpAndSettle();
        } else {
          await tester.enterText(find.byType(TextField), 'Work');
        }
        await tester.tap(find.text('Review change'));
        await tester.pumpAndSettle();
        expect(find.text('2 folders · 10 cached messages'), findsOneWidget);
        await expectLater(
          find.byType(MaterialApp),
          matchesGoldenFile(
            'goldens/folder_${dark ? 'delete_review_dark' : 'rename_review_light'}.png',
          ),
        );
        await tester.tap(find.text(dark ? 'Confirm Delete' : 'Confirm Rename'));
        await tester.pumpAndSettle();
        expect(find.byType(FolderChangeDialog), findsNothing);
        expect(repository.executions, 1);
        expect(repository.provider.isCompleted, isFalse);
        expect(
          controller.visibleNames(repository.account.id, [
            'Projects',
            'Projects/Design',
            'Elsewhere',
          ]),
          ['Elsewhere'],
        );
        expect(controller.entries.single.name, dark ? 'Projects' : 'Work');
        await controller.decide(controller.entries.single, 'cancel');
        expect(controller.visibleNames(repository.account.id, ['Projects']), [
          'Projects',
        ]);
      },
    );
  }
  testWidgets(
    'unknown change only offers checked recovery and retains earlier failure',
    (tester) async {
      final repository = ChangeFixture();
      final controller = await open(tester, repository);
      await tester.enterText(find.byType(TextField), 'Work');
      await tester.tap(find.text('Review change'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Confirm Rename'));
      await tester.pumpAndSettle();
      final saved = repository.entries.single;
      repository.entries[0] = FolderCreation({
        ...saved.data,
        'status': 'uncertain',
        'error': 'The server result is unknown. Check before continuing.',
      });
      await controller.refresh();
      await tester.pumpWidget(
        MaterialApp(
          theme: shepTheme(Brightness.dark),
          home: FolderActivityScreen(controller: controller),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Check server'), findsOneWidget);
      expect(find.text('Retry'), findsNothing);
      expect(find.text('Stop tracking'), findsNothing);
    },
  );
}
