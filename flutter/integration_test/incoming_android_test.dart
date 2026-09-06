import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import '../test/native_repository_test.dart' show FixtureCredentials;
import '../test/workspace_test.dart' show MemorySettings;

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets(
    'cached incoming save cancellation and exact file, reader survives move and refresh',
    (tester) async {
      expect(Platform.isAndroid, isTrue);
      final support = await getApplicationSupportDirectory();
      final fixture = File('${support.path}/shep-incoming-fixture.sqlite'),
          request = File('${support.path}/shep-incoming-request');
      await request.writeAsString('waiting-before-profile-open', flush: true);
      final deadline = DateTime.now().add(const Duration(seconds: 30));
      while (!await fixture.exists()) {
        if (DateTime.now().isAfter(deadline)) {
          fail(
            'Use android_e2e.py --incoming-only to seed this isolated profile',
          );
        }
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      final dir = await Directory.systemTemp.createTemp('shep-incoming-');
      addTearDown(() async {
        await dir.delete(recursive: true);
        if (await fixture.exists()) await fixture.delete();
        if (await request.exists()) await request.delete();
      });
      final path = '${dir.path}/mail.sqlite';
      await fixture.copy(path);
      final credentials = FixtureCredentials()..unavailable = true;
      final repository = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      final workspace = Workspace(repository, MemorySettings());
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(workspace: workspace));
      await tester.pumpAndSettle();
      Future<void> wait(bool Function() condition) async {
        final end = DateTime.now().add(const Duration(seconds: 50));
        while (!condition()) {
          if (DateTime.now().isAfter(end)) {
            fail('Incoming attachment controls did not settle');
          }
          await tester.pump(const Duration(milliseconds: 100));
        }
        await tester.pumpAndSettle();
      }

      await tester.tap(find.text('Incoming files fixture'));
      await tester.pumpAndSettle();
      final save = find.text('Save binary.bin (5 bytes)');
      await wait(() => save.evaluate().isNotEmpty);
      await tester.ensureVisible(save);
      await tester.tap(save);
      await tester.pump();
      await wait(() => find.text('Save cancelled.').evaluate().isNotEmpty);
      expect(credentials.reads, 0);
      // A local POP3 move followed by actual Refresh removes this row from the
      // Inbox page. The reader and its cached file controls must remain available.
      await tester.ensureVisible(find.text('Move'));
      await tester.tap(find.text('Move'));
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(SimpleDialogOption, 'Archive'));
      await tester.pumpAndSettle();
      await wait(() => workspace.pending == 0);
      await tester.tap(find.byTooltip('Refresh mail'));
      await tester.pumpAndSettle();
      await wait(() => !workspace.syncing && workspace.visible.isEmpty);
      expect(find.text('Incoming files fixture'), findsOneWidget);
      await tester.ensureVisible(save);
      await request.writeAsString('save-file', flush: true);
      await tester.tap(save);
      await tester.pump();
      await wait(() => find.text('binary.bin saved.').evaluate().isNotEmpty);
      await binding.convertFlutterSurfaceToImage();
      await tester.pumpAndSettle();
      await binding.takeScreenshot('native-incoming-saved');
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Preferences').last);
      await tester.pumpAndSettle();
      Future<void> show(Finder target, {double delta = -400}) async {
        for (var i = 0; i < 20 && target.evaluate().isEmpty; i++) {
          await tester.drag(find.byType(ListView).last, Offset(0, delta));
          await tester.pumpAndSettle();
        }
        await tester.ensureVisible(target);
        await tester.pumpAndSettle();
      }

      final removeAccount = find.text('Remove owner@example.test');
      await show(removeAccount);
      await tester.tap(removeAccount);
      await tester.pumpAndSettle();
      await wait(
        () => find.textContaining('1 cached messages').evaluate().isNotEmpty,
      );
      await binding.takeScreenshot('native-account-removal-light');
      await tester.tap(find.text('Cancel'));
      await tester.pumpAndSettle();
      expect(repository.mailAccounts, hasLength(1));
      final appearance = find.byType(DropdownButton<ThemeMode>);
      await show(appearance, delta: 400);
      await tester.tap(appearance);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Dark').last);
      await tester.pumpAndSettle();
      await show(removeAccount);
      await tester.tap(removeAccount);
      await tester.pumpAndSettle();
      await wait(
        () => find.textContaining('1 cached messages').evaluate().isNotEmpty,
      );
      await binding.takeScreenshot('native-account-removal-dark');
      expect(workspace.drafts.values.single.subject, 'Account removal draft');
      credentials.removeUnavailable = true;
      await tester.tap(find.text('Remove from device'));
      await tester.pumpAndSettle();
      await wait(() => repository.mailAccounts.isEmpty);
      final retryCleanup = find.text('Retry cleanup');
      await show(retryCleanup);
      await binding.takeScreenshot('native-account-removal-cleanup');
      expect(repository.pendingCredentialCleanup, 1);
      credentials.removeUnavailable = false;
      await tester.tap(retryCleanup);
      await tester.pumpAndSettle();
      await wait(() => repository.pendingCredentialCleanup == 0);
      expect(find.text('Removed account passwords need cleanup'), findsNothing);
      expect(workspace.visible, isEmpty);
      expect(workspace.drafts, isEmpty);
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
    },
  );
}
