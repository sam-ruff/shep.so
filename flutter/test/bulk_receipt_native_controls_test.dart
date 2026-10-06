import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:shep_mobile/src/rust/frb_generated.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/mail_groups.dart';
import 'package:shep_mobile/ui/theme.dart';
import 'native_repository_test.dart' show FixtureCredentials;
import 'workspace_test.dart' show MemorySettings;
import 'mail_activity_visual_test.dart' show loadPreviewFonts;

class _ObservedRepository extends NativeRepository {
  _ObservedRepository(super.profile, super.credentials);
  int steps = 0;
  @override
  Future<Map<String, dynamic>> groupStep() {
    steps++;
    return super.groupStep();
  }
}

void main() {
  setUpAll(() async {
    final name = Platform.isWindows
        ? 'shep_mobile_native.dll'
        : Platform.isMacOS
        ? 'libshep_mobile_native.dylib'
        : 'libshep_mobile_native.so';
    await ShepNative.init(
      externalLibrary: ExternalLibrary.open(
        'build/native_assets/${Platform.operatingSystem}/$name',
      ),
    );
  });
  Future<void> script(List<String> args) async {
    final result = await Process.run('python3', args);
    expect(result.exitCode, 0, reason: '${result.stderr}');
  }

  Future<void> wait(WidgetTester tester, bool Function() ready) async {
    final deadline = DateTime.now().add(const Duration(seconds: 45));
    while (!ready()) {
      if (DateTime.now().isAfter(deadline)) {
        fail('Native receipt controls did not settle');
      }
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 20)),
      );
      await tester.pump();
    }
    await tester.pumpAndSettle();
  }

  for (final dark in [false, true]) {
    final scheme = dark ? 'dark' : 'light';
    testWidgets(
      'persistent native cache fault stops owner and exposes Undo/Retry ($scheme)',
      (tester) async {
        tester.view.physicalSize = const Size(1080, 2280);
        tester.view.devicePixelRatio = 2.625;
        addTearDown(tester.view.reset);
        await loadPreviewFonts();
        final prepared = await tester.runAsync(() async {
          final directory = await Directory.systemTemp.createTemp(
            'shep-bulk-receipt-',
          );
          final path = '${directory.path}/mail.sqlite';
          await script([
            '../scripts/clients/android_incoming_fixture.py',
            '--prepare',
            path,
          ]);
          final credentials = FixtureCredentials()..unavailable = true;
          final native = await NativeRepository.open(
            path,
            credentials: credentials,
          );
          final repository = _ObservedRepository(native.profile, credentials);
          await repository.initialize();
          final capture = await repository.selection({
            'kind': 'capture',
            'id': 'ffi-repair-selection',
            'revision': 0,
            'all': true,
            'scope': {'folder': 'Inbox'},
          });
          await repository.groups({
            'kind': 'prepare',
            'id': 'ffi-repair',
            'selection': capture['id'],
            'expected': capture['revision'],
            'action': {'kind': 'read'},
            'scope': {'folder': 'Inbox'},
          });
          await repository.groups({'kind': 'approve', 'id': 'ffi-repair'});
          await script([
            '../scripts/clients/mobile_bulk_receipt_fixture.py',
            path,
          ]);
          final workspace = Workspace(repository, MemorySettings());
          workspace.setForeground(false);
          await workspace.initialize();
          final groups = workspace.groups!;
          await groups.pump();
          expect(repository.steps, 1);
          await groups.refreshHistory();
          await groups.refreshHistory();
          expect(
            repository.steps,
            1,
            reason: 'History must not re-wake a persistently blocked repair',
          );
          expect(groups.jobs.single.count('repair'), 1);
          expect(groups.jobs.single.canUndo, true);
          return (directory, path, repository, workspace, credentials);
        });
        if (prepared == null) fail('Native receipt fixture did not open');
        final (directory, path, repository, workspace, credentials) = prepared;
        final groups = workspace.groups!;
        addTearDown(() async {
          await tester.pumpWidget(const SizedBox());
          await tester.runAsync(() async {
            workspace.dispose();
            repository.profile.dispose();
            await directory.delete(recursive: true);
          });
        });
        await tester.pumpWidget(
          MaterialApp(
            debugShowCheckedModeBanner: false,
            theme: shepTheme(dark ? Brightness.dark : Brightness.light),
            home: GroupHistoryScreen(
              workspace: workspace,
              initialJob: groups.jobs.single,
            ),
          ),
        );
        await wait(
          tester,
          () => find.widgetWithText(TextButton, 'Retry').evaluate().isNotEmpty,
        );
        expect(find.widgetWithText(TextButton, 'Undo'), findsOneWidget);
        await expectLater(
          find.byType(GroupHistoryScreen),
          matchesGoldenFile('goldens/bulk_receipt_repair_$scheme.png'),
        );
        await tester.ensureVisible(find.widgetWithText(TextButton, 'Undo'));
        await tester.tap(find.widgetWithText(TextButton, 'Undo'));
        await wait(tester, () => !groups.running && groups.jobs.single.undo);
        expect(
          repository.steps,
          2,
          reason: 'Undo retains one repair attempt and then stops',
        );
        final checked = await tester.runAsync(
          () => repository.groups({'kind': 'inspect', 'id': 'ffi-repair'}),
        );
        expect(checked['undo'], true);
        expect((checked['counts'] as Map)['repair'], 1);
        await tester.runAsync(
          () => script([
            '../scripts/clients/mobile_bulk_receipt_fixture.py',
            path,
            '--release',
          ]),
        );
        await tester.ensureVisible(find.widgetWithText(TextButton, 'Retry'));
        await tester.tap(find.widgetWithText(TextButton, 'Retry'));
        await wait(
          tester,
          () => !groups.running && groups.jobs.single.finished,
        );
        expect(groups.jobs.single.count('undone'), 1);
        expect(credentials.reads, 0);
        final detail = await tester.runAsync(
          () => repository.groups({'kind': 'items', 'id': 'ffi-repair'}),
        );
        expect(
          (detail['rows'] as List).single['receipt']['dispatch']['remote_id'],
          'files',
        );
        expect(
          (await tester.runAsync(() => repository.call({'op': 'mail_actions'}))
              as Map)['actions'],
          isEmpty,
        );
        groups.dismiss();
        await tester.pumpWidget(const SizedBox());
      },
    );
  }
}
