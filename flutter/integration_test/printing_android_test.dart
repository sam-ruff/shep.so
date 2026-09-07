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
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets(
    'native Print opens Android dialog, cancels, retries and saves complete PDFs',
    (tester) async {
      expect(Platform.isAndroid, true);
      final support = await getApplicationSupportDirectory();
      final fixture = File('${support.path}/shep-print-fixture.sqlite');
      final request = File('${support.path}/shep-print-request');
      await request.writeAsString('waiting-before-profile-open', flush: true);
      final end = DateTime.now().add(const Duration(seconds: 30));
      while (!await fixture.exists()) {
        if (DateTime.now().isAfter(end)) {
          fail('Run with android_print_fixture.py');
        }
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      final directory = await Directory.systemTemp.createTemp(
        'shep-print-native-',
      );
      final path = '${directory.path}/mail.sqlite3';
      await fixture.copy(path);
      final repository = await NativeRepository.open(
        path,
        credentials: FixtureCredentials(),
      );
      final workspace = Workspace(repository, MemorySettings());
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(workspace: workspace));
      await tester.pumpAndSettle();
      addTearDown(() async {
        await tester.pumpWidget(const SizedBox());
        workspace.dispose();
        await directory.delete(recursive: true);
        await fixture.delete();
        await request.delete();
      });
      Future<void> print(String action) async {
        await tester.ensureVisible(find.text('Print'));
        await tester.pumpAndSettle();
        await request.writeAsString(action, flush: true);
        await tester.tap(find.text('Print'));
        await tester.pump();
        final deadline = DateTime.now().add(const Duration(seconds: 60));
        while ((await request.readAsString()).trim() != 'done:$action') {
          if (workspace.error != null) fail(workspace.error!);
          if (DateTime.now().isAfter(deadline)) {
            fail('Native printer did not finish: $action');
          }
          await tester.pump(const Duration(milliseconds: 100));
        }
        await tester.pumpAndSettle();
        expect(workspace.error, isNull);
      }

      await tester.tap(find.text('Café project'));
      await tester.pumpAndSettle();
      await print('cancel');
      await print('save:formatted');
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Long forward source'));
      await tester.pumpAndSettle();
      await print('save:long');
      await request.writeAsString('complete', flush: true);
      final completed = DateTime.now().add(const Duration(seconds: 10));
      while ((await request.readAsString()).trim() != 'done:complete') {
        if (DateTime.now().isAfter(completed)) {
          fail('Print helper did not acknowledge completion');
        }
        await tester.pump(const Duration(milliseconds: 100));
      }
    },
  );
}
