import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'package:shep_mobile/ui/composer.dart';
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
      final settings = MemorySettings();
      var workspace = Workspace(repository, settings);
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
      await tester.pumpAndSettle();
      Future<void> wait(bool Function() ready) async {
        await tester.pump();
        final end = DateTime.now().add(const Duration(seconds: 45));
        while (!ready()) {
          if (DateTime.now().isAfter(end)) {
            fail('Native compose controls did not settle');
          }
          await tester.pump(const Duration(milliseconds: 100));
        }
        await tester.pumpAndSettle();
      }

      bool saveEnabled() =>
          tester
              .widget<TextButton>(find.widgetWithText(TextButton, 'Save draft'))
              .onPressed !=
          null;
      Future<void> editable() => wait(saveEnabled);
      Future<void> show(Finder target, {bool back = false}) async {
        await tester.scrollUntilVisible(
          target,
          back ? -250 : 250,
          scrollable: find.byType(Scrollable).first,
        );
        await tester.pumpAndSettle();
      }

      Map<String, dynamic>? originalSnapshot;
      Future<void> reopen() async {
        await tester.pumpWidget(const SizedBox());
        workspace.dispose();
        repository.profile.dispose();
        repository = await NativeRepository.open(
          path,
          credentials: credentials,
        );
        workspace = Workspace(repository, settings);
        await workspace.initialize();
        if (originalSnapshot case final saved?) {
          final reopened = repository.savedDrafts.single;
          expect(reopened.replyContext?.toJson(), saved['reply_context']);
          expect(reopened.references, saved['references']);
          expect(reopened.inReplyTo, saved['in_reply_to']);
          expect(reopened.bcc, saved['bcc']);
        }
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

      await tester.tap(find.text('Preferences').last);
      await tester.pumpAndSettle();
      final defaultControl = find.byKey(
        const ValueKey('preference-reply-original'),
      );
      await tester.ensureVisible(defaultControl);
      await tester.tap(defaultControl);
      await wait(() => !workspace.preferences.replyIncludeOriginal);
      await tester.tap(find.text('Mail').last);
      await tester.pumpAndSettle();
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
      final toggle = find.byType(CheckboxListTile);
      expect(tester.widget<CheckboxListTile>(toggle).value, isFalse);
      expect(find.textContaining('On 06 Sep 2026'), findsOneWidget);
      await tester.tap(toggle);
      await tester.pumpAndSettle();
      final message = find.widgetWithText(TextField, 'Message');
      await show(message);
      expect(tester.widget<TextField>(message).controller!.text, isEmpty);
      await tester.enterText(
        message,
        'Typed answer survives toggling the original.',
      );
      await show(toggle, back: true);
      await tester.tap(toggle);
      await tester.pumpAndSettle();
      await show(message);
      expect(
        tester.widget<TextField>(message).controller!.text,
        'Typed answer survives toggling the original.',
      );
      Future<void> attach() async {
        await show(find.text('Attach files'), back: true);
        await tester.tap(find.text('Attach files'));
        await tester.pump();
        // Observe the lock while the helper drives native DocumentsUI controls.
        await wait(() => !saveEnabled());
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
      await wait(() => find.byType(Composer).evaluate().isEmpty);
      originalSnapshot = Map<String, dynamic>.from(
        (await repository.call({'op': 'drafts'}) as List).single as Map,
      );
      await reopen();
      expect(tester.widget<CheckboxListTile>(toggle).value, isFalse);
      await show(message);
      await tester.enterText(
        find.widgetWithText(TextField, 'Message'),
        'Pending text survives removing a file.',
      );
      await show(find.byTooltip('Remove shep-e2e-first.txt'), back: true);
      await tester.tap(find.byTooltip('Remove shep-e2e-first.txt'));
      await editable();
      expect(find.byTooltip('Remove shep-e2e-first.txt'), findsNothing);
      // Reopen without Save/Close: file removal also flushes pending text.
      await reopen();
      expect(tester.widget<CheckboxListTile>(toggle).value, isFalse);
      await show(message);
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
      await wait(() => find.byType(Composer).evaluate().isEmpty);
      await reopen();
      expect(tester.widget<CheckboxListTile>(toggle).value, isFalse);
      await show(find.byTooltip('Remove shep-e2e-binary.bin'));
      expect(find.byTooltip('Remove shep-e2e-first.txt'), findsNothing);
      expect(find.byTooltip('Remove shep-e2e-binary.bin'), findsOneWidget);
      await show(message);
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Message'))
            .controller!
            .text,
        'Saved reply with one binary attachment.',
      );
      await show(toggle, back: true);
      await binding.convertFlutterSurfaceToImage();
      await tester.pumpAndSettle();
      await binding.takeScreenshot('native-reply-attachments');
      await tester.tap(find.byTooltip('Send'));
      await wait(() => find.byType(Composer).evaluate().isEmpty);
      await tester.tap(find.byTooltip('Open navigation menu'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Outbox'));
      await wait(
        () => find.text('Waiting for account access').evaluate().isNotEmpty,
      );
      final queued = (await repository.outbox()).rows.single;
      expect(queued.state, 'waiting');
      expect(queued.subject, 'Re: Shared reply');
      expect(queued.accountId, originalSnapshot!['account_id']);
      expect(credentials.values, isEmpty);
      await binding.takeScreenshot('native-reply-outbox-waiting');
      await tester.tap(find.text('Cancel'));
      await wait(
        () => find
            .text('No outgoing messages need attention.')
            .evaluate()
            .isNotEmpty,
      );
      expect((await repository.outbox()).rows, isEmpty);
      await reopen();
      expect(tester.widget<CheckboxListTile>(toggle).value, isFalse);
      await show(find.byTooltip('Remove shep-e2e-binary.bin'));
      expect(find.byTooltip('Remove shep-e2e-binary.bin'), findsOneWidget);
      await show(message);
      expect(
        tester.widget<TextField>(message).controller!.text,
        'Saved reply with one binary attachment.',
      );
      binding.reportData = {
        ...?binding.reportData,
        'scenarios': ['native-reply-original-default-files-reopen'],
      };
      expect(credentials.values, isEmpty);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
      repository.profile.dispose();
    },
  );
}
