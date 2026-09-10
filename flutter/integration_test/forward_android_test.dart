import 'dart:async';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/model/preferences.dart';
import 'package:shep_mobile/ui/app.dart';
import '../test/native_repository_test.dart' show FixtureCredentials;
import '../test/support/forward_repository.dart';
import '../test/workspace_test.dart' show MemorySettings;

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets(
    'native Forward retains complete source, files, retry and independent edits',
    (tester) async {
      expect(Platform.isAndroid, true);
      final support = await getApplicationSupportDirectory();
      final fixture = File('${support.path}/shep-forward-fixture.sqlite');
      final request = File('${support.path}/shep-forward-request');
      await request.writeAsString('waiting-before-profile-open', flush: true);
      Future<void> wait(bool Function() ready, {bool settle = true}) async {
        final end = DateTime.now().add(const Duration(seconds: 25));
        while (!ready()) {
          if (DateTime.now().isAfter(end)) {
            fail('Native Forward state did not settle');
          }
          await tester.pump(const Duration(milliseconds: 100));
        }
        if (settle) await tester.pumpAndSettle();
      }

      final end = DateTime.now().add(const Duration(seconds: 25));
      while (!await fixture.exists()) {
        if (DateTime.now().isAfter(end)) {
          fail('Run with android_forward_fixture.py');
        }
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      final directory = await Directory.systemTemp.createTemp(
        'shep-forward-native-',
      );
      final path = '${directory.path}/mail.sqlite3';
      await fixture.copy(path);
      final credentials = FixtureCredentials();
      late ForwardFixtureRepository repository;
      late Workspace workspace;
      Future<void> mount({bool dark = false}) async {
        final native = await NativeRepository.open(
          path,
          credentials: credentials,
        );
        repository = ForwardFixtureRepository(native.profile, credentials);
        final settings = MemorySettings();
        if (dark) {
          settings.value = const Preferences(appearance: ThemeMode.dark);
        }
        workspace = Workspace(repository, settings);
        await workspace.initialize();
        workspace.setForeground(false);
        await tester.pumpWidget(
          ShepApp(key: UniqueKey(), workspace: workspace),
        );
        await tester.pumpAndSettle();
      }

      Future<void> capture(String name) async {
        await tester.pumpAndSettle();
        await request.writeAsString('capture:$name', flush: true);
        final end = DateTime.now().add(const Duration(seconds: 20));
        while ((await request.readAsString()).trim() != 'captured:$name') {
          if (DateTime.now().isAfter(end)) {
            fail('External Forward capture failed');
          }
          await tester.pump(const Duration(milliseconds: 100));
        }
      }

      Finder field(String label) => find.widgetWithText(TextField, label);
      String text(String label) =>
          tester.widget<TextField>(field(label)).controller!.text;
      Future<void> editable() => wait(
        () =>
            field('To').evaluate().isNotEmpty &&
            !tester.widget<TextField>(field('To')).readOnly,
      );
      Future<void> clickForward() async {
        await tester.ensureVisible(find.text('Forward'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Forward'));
        await tester.pump();
      }

      await mount();
      addTearDown(() async {
        await tester.pumpWidget(const SizedBox());
        workspace.dispose();
        await directory.delete(recursive: true);
        if (await fixture.exists()) await fixture.delete();
        if (await request.exists()) await request.delete();
      });
      await tester.tap(find.text('Café project'));
      await tester.pumpAndSettle();
      repository.loseAcknowledgment = true;
      await clickForward();
      await wait(
        () => workspace.error?.contains('lost forward acknowledgment') ?? false,
      );
      expect((await repository.call({'op': 'drafts'}) as List).length, 1);
      await capture('native-forward-retry');
      await clickForward();
      await editable();
      expect(
        find.descendant(
          of: find.byType(AppBar),
          matching: find.text('Forward'),
        ),
        findsOneWidget,
      );
      expect(text('To'), isEmpty);
      expect(text('Subject'), 'Fwd: Café project');
      expect(text('Message'), contains('Complete café.'));
      expect(text('Message'), isNot(contains('private@example.test')));
      expect(find.byType(InputChip), findsNWidgets(3));
      final original = workspace.drafts.values.single;
      expect(original.inReplyTo, isNull);
      expect(original.references, isEmpty);
      expect(original.attachments.last.contentId, startsWith('shep-'));
      expect(credentials.reads, 0);
      await tester.enterText(field('To'), 'reviewer@example.test');
      await tester.enterText(
        field('Message'),
        'Please review.\n${text('Message')}',
      );
      await wait(() => tester.view.viewInsets.bottom > 0);
      await request.writeAsString('hide-keyboard', flush: true);
      final keyboardDeadline = DateTime.now().add(const Duration(seconds: 20));
      while ((await request.readAsString()).trim() != 'keyboard-hidden') {
        if (DateTime.now().isAfter(keyboardDeadline)) {
          fail('Native keyboard dismissal failed');
        }
        await tester.pump(const Duration(milliseconds: 100));
      }
      await wait(() => tester.view.viewInsets.bottom == 0);
      await Scrollable.ensureVisible(
        tester.element(find.byType(InputChip).first),
        alignment: 0.5,
      );
      await tester.pumpAndSettle();
      await wait(
        () => find
            .byTooltip('Remove duplicate.bin')
            .first
            .hitTestable()
            .evaluate()
            .isNotEmpty,
      );
      await tester.tap(find.byTooltip('Remove duplicate.bin').first);
      await wait(
        () =>
            tester
                .widget<TextButton>(
                  find.widgetWithText(TextButton, 'Save draft'),
                )
                .onPressed !=
            null,
      );
      expect(find.byType(InputChip), findsNWidgets(2));
      await capture('native-forward-light');
      await tester.tap(find.text('Save draft'));
      await wait(
        () => find
            .descendant(of: find.byType(AppBar), matching: find.text('Forward'))
            .evaluate()
            .isEmpty,
      );
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
      await mount(dark: true);
      await tester.tap(find.byTooltip('Open navigation menu'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Drafts'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Fwd: Café project'));
      await editable();
      expect(text('Message'), startsWith('Please review.'));
      expect(text('To'), 'reviewer@example.test');
      expect(find.byType(InputChip), findsNWidgets(2));
      expect(
        workspace.drafts.values.single.forward!.toJson(),
        original.forward!.toJson(),
      );
      expect(
        workspace.drafts.values.single.attachments.last.contentId,
        original.attachments.last.contentId,
      );
      await capture('native-forward-dark');
      await tester.tap(find.byTooltip('Send'));
      await wait(
        () => workspace.error?.contains('password is missing') ?? false,
      );
      await editable();
      expect(text('Message'), startsWith('Please review.'));
      await tester.tap(find.text('Save draft'));
      await wait(
        () => find
            .descendant(of: find.byType(AppBar), matching: find.text('Forward'))
            .evaluate()
            .isEmpty,
      );
      await tester.tap(find.byTooltip('Open navigation menu'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Inbox').first);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Long forward source'));
      await tester.pumpAndSettle();
      await clickForward();
      await editable();
      expect(text('Message').length, greaterThan(32000));
      expect(text('Message'), endsWith('END OF COMPLETE ORIGINAL'));
      await tester.tap(find.text('Save draft'));
      await wait(
        () => find
            .descendant(of: find.byType(AppBar), matching: find.text('Forward'))
            .evaluate()
            .isEmpty,
      );
      await tester.tap(find.byTooltip('Back'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Damaged forward'));
      await tester.pumpAndSettle();
      final count = workspace.drafts.length;
      await clickForward();
      await wait(() => workspace.error?.contains('Could not decode') ?? false);
      expect(workspace.drafts.length, count);
      await tester.tap(find.byTooltip('Back'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Other letter'));
      await tester.pumpAndSettle();
      repository.release = Completer<void>();
      repository.started = Completer<void>();
      await clickForward();
      // The deliberately held operation now has an animated busy icon. Wait
      // for its transport barrier, then navigate using the actual Back control.
      await wait(() => repository.started!.isCompleted, settle: false);
      await tester.tap(find.byTooltip('Back'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Compose'));
      await editable();
      await tester.enterText(field('Subject'), 'Independent note');
      repository.release!.complete();
      await wait(() => !workspace.isForwarding('fixture:INBOX:other'));
      expect(find.text('New message'), findsOneWidget);
      expect(text('Subject'), 'Independent note');
      expect(
        workspace.drafts.values.any((d) => d.subject == 'Fwd: Other letter'),
        true,
      );
      await capture('native-forward-independent');
      await tester.tap(find.text('Save draft'));
      await wait(() => find.text('New message').evaluate().isEmpty);
      await capture('complete');
    },
  );
}
