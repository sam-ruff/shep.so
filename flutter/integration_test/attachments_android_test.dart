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
    'cached Reply all, real Android multi-file picker, removal and restart',
    (tester) async {
      expect(
        Platform.isAndroid,
        isTrue,
        reason: 'Use android_e2e.py with its DocumentsUI helper',
      );
      final support = await getApplicationSupportDirectory();
      final fixture = File('${support.path}/shep-compose-fixture.sqlite');
      final request = File('${support.path}/shep-compose-request');
      await request.writeAsString('waiting-before-profile-open', flush: true);
      final limit = DateTime.now().add(const Duration(seconds: 25));
      while (!await fixture.exists()) {
        if (DateTime.now().isAfter(limit)) {
          fail('Run through android_e2e.py to prepare the synthetic profile');
        }
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      final directory = await Directory.systemTemp.createTemp(
        'shep-compose-fixture-',
      );
      addTearDown(() async {
        await directory.delete(recursive: true);
        if (await request.exists()) await request.delete();
        if (await fixture.exists()) await fixture.delete();
      });
      final path = '${directory.path}/mail.sqlite3';
      await fixture.copy(path);
      final credentials = FixtureCredentials();
      var repository = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      var workspace = Workspace(repository, MemorySettings());
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
      await tester.pumpAndSettle();
      Future<void> wait(bool Function() ready) async {
        final end = DateTime.now().add(const Duration(seconds: 45));
        while (!ready()) {
          if (DateTime.now().isAfter(end)) {
            fail('Native compose controls did not settle');
          }
          await tester.pump(const Duration(milliseconds: 100));
        }
        await tester.pumpAndSettle();
      }

      Future<void> editable() => wait(
        () => !tester
            .widget<TextField>(find.widgetWithText(TextField, 'To'))
            .readOnly,
      );
      Future<void> reopen() async {
        await tester.pumpWidget(const SizedBox());
        workspace.dispose();
        repository = await NativeRepository.open(
          path,
          credentials: credentials,
        );
        workspace = Workspace(repository, MemorySettings());
        await workspace.initialize();
        workspace.setForeground(false);
        await tester.pumpWidget(
          ShepApp(key: UniqueKey(), workspace: workspace),
        );
        await tester.pumpAndSettle();
        await tester.tap(find.byTooltip('Open navigation menu'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Drafts'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Re: Shared reply'));
        await tester.pumpAndSettle();
        await editable();
      }

      await tester.tap(find.text('Shared reply'));
      await tester.pumpAndSettle();
      await tester.ensureVisible(find.text('Reply all'));
      await tester.tap(find.text('Reply all'));
      await tester.pumpAndSettle();
      await editable();
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'To'))
            .controller!
            .text,
        'Support <support@example.test>, Peer <peer@example.test>',
      );
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Cc'))
            .controller!
            .text,
        'Copy <copy@example.test>',
      );
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Message'))
            .controller!
            .text,
        contains('On 06 Sep 2026'),
      );
      Future<void> attach() async {
        await tester.ensureVisible(find.text('Attach files'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Attach files'));
        await tester.pump();
        // Observe the lock while the helper drives native DocumentsUI controls.
        await wait(
          () => tester
              .widget<TextField>(find.widgetWithText(TextField, 'To'))
              .readOnly,
        );
        await editable();
      }

      await request.writeAsString('picker-cancel', flush: true);
      await attach(); // Host presses Android Back: no files added.
      expect(find.byType(InputChip), findsNothing);
      await request.writeAsString('picker-select', flush: true);
      await attach(); // Host long-presses/selects both synthetic files.
      expect(find.byTooltip('Remove shep-e2e-first.txt'), findsOneWidget);
      expect(find.byTooltip('Remove shep-e2e-binary.bin'), findsOneWidget);
      await tester.tap(find.text('Save draft'));
      await wait(() => find.text('New message').evaluate().isEmpty);
      await reopen();
      await tester.enterText(
        find.widgetWithText(TextField, 'Message'),
        'Pending text survives removing a file.',
      );
      await tester.ensureVisible(find.byTooltip('Remove shep-e2e-first.txt'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Remove shep-e2e-first.txt'));
      await editable();
      expect(find.byTooltip('Remove shep-e2e-first.txt'), findsNothing);
      // Reopen without Save/Close: file removal also flushes pending text.
      await reopen();
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Message'))
            .controller!
            .text,
        'Pending text survives removing a file.',
      );
      await tester.enterText(
        find.widgetWithText(TextField, 'Message'),
        'Saved reply with one binary attachment.',
      );
      await tester.tap(find.text('Save draft'));
      await wait(() => find.text('New message').evaluate().isEmpty);
      await reopen();
      expect(find.byTooltip('Remove shep-e2e-first.txt'), findsNothing);
      expect(find.byTooltip('Remove shep-e2e-binary.bin'), findsOneWidget);
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Message'))
            .controller!
            .text,
        'Saved reply with one binary attachment.',
      );
      await tester.tap(find.byTooltip('Send'));
      await editable();
      expect(workspace.error, contains('Reconnect'));
      expect(find.byTooltip('Remove shep-e2e-binary.bin'), findsOneWidget);
      await tester.ensureVisible(find.text('Attach files'));
      await binding.convertFlutterSurfaceToImage();
      await tester.pumpAndSettle();
      await binding.takeScreenshot('native-reply-attachments');
      expect(credentials.values, isEmpty);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
    },
  );
}
