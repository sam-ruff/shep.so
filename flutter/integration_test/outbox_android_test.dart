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
    'native Outbox recovery, immutable files, local Sent and profile reopen',
    (tester) async {
      expect(Platform.isAndroid, isTrue);
      final previousHitTestPolicy =
          WidgetController.hitTestWarningShouldBeFatal;
      WidgetController.hitTestWarningShouldBeFatal = true;
      addTearDown(
        () => WidgetController.hitTestWarningShouldBeFatal =
            previousHitTestPolicy,
      );
      final support = await getApplicationSupportDirectory();
      final fixture = File('${support.path}/shep-outbox-fixture.sqlite');
      final marker = File('${support.path}/shep-outbox-request');
      final lockChecked = File('${support.path}/shep-outbox-lock-checked');
      await marker.writeAsString('waiting-before-profile-open', flush: true);
      final deadline = DateTime.now().add(const Duration(seconds: 30));
      while (!await fixture.exists()) {
        if (DateTime.now().isAfter(deadline)) {
          fail('Run the Outbox fixture helper before this test');
        }
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      final directory = await Directory.systemTemp.createTemp(
        'shep-outbox-fixture-',
      );
      addTearDown(() async {
        await directory.delete(recursive: true);
        if (await marker.exists()) await marker.delete();
        if (await fixture.exists()) await fixture.delete();
        if (await lockChecked.exists()) await lockChecked.delete();
      });
      final path = '${directory.path}/mail.sqlite3';
      await fixture.copy(path);
      final credentials = FixtureCredentials();
      var repository = await NativeRepository.open(
        path,
        credentials: credentials,
      );
      await marker.writeAsString('profile-open:$path', flush: true);
      final lockDeadline = DateTime.now().add(const Duration(seconds: 30));
      while (!await lockChecked.exists()) {
        if (DateTime.now().isAfter(lockDeadline)) {
          fail('Android process lock was not verified');
        }
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      var workspace = Workspace(repository, MemorySettings());
      await workspace.initialize();
      workspace.setForeground(false);
      await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
      await tester.pumpAndSettle();
      Future<void> wait(bool Function() condition) async {
        final until = DateTime.now().add(const Duration(seconds: 30));
        while (!condition()) {
          if (DateTime.now().isAfter(until)) {
            fail('Outbox controls did not settle');
          }
          await tester.pump(const Duration(milliseconds: 100));
        }
        await tester.pumpAndSettle();
      }

      Future<void> openOutbox() async {
        await tester.tap(find.byTooltip('Open navigation menu'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Outbox'));
        await tester.pumpAndSettle();
        await wait(
          () => find.byType(LinearProgressIndicator).evaluate().isEmpty,
        );
      }

      bool surfaceConverted = false;
      Future<void> capture(String name) async {
        if (!surfaceConverted) {
          await binding.convertFlutterSurfaceToImage();
          surfaceConverted = true;
        }
        await tester.pumpAndSettle();
        await binding.takeScreenshot(name);
      }

      Finder card(String name) => find.byKey(ValueKey('outgoing-outbox-$name'));
      Finder action(String name, String text) => find.descendant(
        of: card(name),
        matching: find.widgetWithText(OutlinedButton, text),
      );
      Future<void> show(Finder target) async {
        // Lazy rows can leave the viewport when a preceding card disappears.
        // Find them with actual touch scrolling before tapping their controls.
        for (final delta in [600.0, -400.0]) {
          for (var step = 0; step < 12 && target.evaluate().isEmpty; step++) {
            await tester.drag(find.byType(ListView).last, Offset(0, delta));
            await tester.pumpAndSettle();
          }
          if (target.evaluate().isNotEmpty) {
            break;
          }
        }
        if (target.evaluate().isEmpty) {
          await capture('native-sent-action-failure');
        }
        expect(target, findsOneWidget);
        await tester.ensureVisible(target);
        await tester.pumpAndSettle();
      }

      Future<void> click(String name, String text) async {
        final target = action(name, text);
        await show(target);
        await tester.pumpAndSettle();
        expect(tester.widget<OutlinedButton>(target).onPressed, isNotNull);
        await tester.tap(target);
        await tester.pumpAndSettle();
        await wait(
          () => find.byType(LinearProgressIndicator).evaluate().isEmpty,
        );
      }

      await openOutbox();
      expect(
        tester
            .widget<OutlinedButton>(action('return', 'Return to drafts'))
            .onPressed,
        isNull,
      );
      expect(
        tester
            .widget<OutlinedButton>(action('return', 'Record as sent'))
            .onPressed,
        isNull,
      );
      await capture('native-outbox-review-light');
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Preferences').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byType(DropdownButton<ThemeMode>));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Dark').last);
      await tester.pumpAndSettle();
      await wait(() => workspace.preferences.appearance == ThemeMode.dark);
      await openOutbox();
      await capture('native-outbox-review-dark');

      await tester.tap(
        find.descendant(
          of: card('return'),
          matching: find.byType(CheckboxListTile),
        ),
      );
      await tester.pumpAndSettle();
      await click('return', 'Return to drafts');
      expect(
        find.text('Returned to drafts. Sending requires a new Send action.'),
        findsOneWidget,
      );
      await tester.tap(find.text('Open recovered draft'));
      await tester.pumpAndSettle();
      await wait(
        () => !tester
            .widget<TextField>(find.widgetWithText(TextField, 'To'))
            .readOnly,
      );
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Message'))
            .controller!
            .text,
        'Original native return body.',
      );
      await tester.ensureVisible(find.byTooltip('Remove review.bin'));
      expect(find.byTooltip('Remove review.bin'), findsOneWidget);
      await capture('native-outbox-recovered-draft');
      await tester.tap(find.byTooltip('Remove review.bin'));
      await tester.pumpAndSettle();
      await wait(() => find.byTooltip('Remove review.bin').evaluate().isEmpty);
      await tester.tap(find.text('Save draft'));
      await tester.pumpAndSettle();
      await wait(() => find.text('Outbox').evaluate().isNotEmpty);
      await tester.ensureVisible(
        find.descendant(
          of: card('mark'),
          matching: find.byType(CheckboxListTile),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(
        find.descendant(
          of: card('mark'),
          matching: find.byType(CheckboxListTile),
        ),
      );
      await tester.pumpAndSettle();
      await click('mark', 'Record as sent');
      await click('rejected', 'Return to drafts');
      await click('delivered', 'Keep local copy');

      // A persisted provider acknowledgment repairs the exact local copy even
      // without credentials. The unacknowledged upload needs a separate review.
      await click('copy-saved', 'Finish saving Sent copy');
      await click('copy-handover', 'Finish saving Sent copy');
      final upload = action('copy-uncertain', 'Save server Sent copy');
      await show(upload);
      await tester.pumpAndSettle();
      expect(tester.widget<OutlinedButton>(upload).onPressed, isNull);
      await click('copy-uncertain', 'Check provider Sent');
      expect(
        find.textContaining('Unlock device credentials').evaluate().isNotEmpty,
        isTrue,
      );
      await capture('native-sent-copy-review');
      final reviewCopy = find.descendant(
        of: card('copy-uncertain'),
        matching: find.byType(CheckboxListTile),
      );
      await tester.ensureVisible(reviewCopy);
      await tester.pumpAndSettle();
      await tester.tap(reviewCopy);
      await tester.pumpAndSettle();
      await click('copy-uncertain', 'Save server Sent copy');
      expect(tester.widget<OutlinedButton>(upload).onPressed, isNull);
      expect(
        find.textContaining('Unlock device credentials').evaluate().isNotEmpty,
        isTrue,
      );
      await click('copy-uncertain', 'Keep local copy');
      expect(find.text('No outgoing messages need attention.'), findsOneWidget);
      await capture('native-outbox-empty');
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Open navigation menu'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Sent'));
      await tester.pumpAndSettle();
      await wait(() => find.text('Native Outbox mark').evaluate().isNotEmpty);
      await tester.tap(find.text('Native Outbox mark'));
      await tester.pumpAndSettle();
      await wait(
        () => find
            .byWidgetPredicate(
              (widget) =>
                  widget is SelectableText &&
                  widget.data?.trim() == 'Original native mark body.',
            )
            .evaluate()
            .isNotEmpty,
      );
      await capture('native-outbox-local-sent');

      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Native Outbox copy-saved'));
      await tester.pumpAndSettle();
      await wait(() => workspace.loadingBodies.isEmpty);
      const localId = 'imap-fixture:Sent:local-sent-outbox-copy-saved';
      credentials.unavailable = true;
      final credentialReads = credentials.reads;
      await tester.tap(find.byTooltip('Flag'));
      await tester.pumpAndSettle();
      await wait(() => workspace.pending == 0);
      expect(workspace.error, isNull);
      expect(workspace.mail(localId)!.starred, isTrue);
      await tester.tap(find.byTooltip('Mark unread'));
      await tester.pumpAndSettle();
      await wait(() => workspace.pending == 0);
      expect(workspace.error, isNull);
      expect(workspace.mail(localId)!.unread, isTrue);
      await capture('native-imap-local-sent-offline');
      await tester.tap(find.byTooltip('Archive'));
      await tester.pumpAndSettle();
      await wait(() => workspace.pending == 0);
      expect(workspace.error, isNull);
      expect(find.text('Native Outbox copy-saved'), findsNothing);
      expect(credentials.reads, credentialReads);
      credentials.unavailable = false;

      await tester.tap(find.text('Native Outbox copy-handover'));
      await tester.pumpAndSettle();
      await wait(() => workspace.loadingBodies.isEmpty);
      const handoverId = 'imap-fixture:Sent:local-sent-outbox-copy-handover';
      expect(workspace.mail(handoverId)!.folder, 'Sent Mail');
      expect(workspace.mail('imap-fixture:Sent Mail:91.4')!.id, handoverId);
      expect(find.text('Original native copy-handover body.'), findsOneWidget);
      await capture('native-sent-handover');
      await tester.pageBack();
      await tester.pumpAndSettle();

      // Return to the existing reader so the following Preferences navigation
      // remains the same real-control path as the recovery scenario above.
      await tester.tap(find.text('Native Outbox mark'));
      await tester.pumpAndSettle();
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Preferences').last);
      await tester.pumpAndSettle();
      final sentSettings = find.descendant(
        of: find.widgetWithText(ListTile, 'Native Sent fixture'),
        matching: find.text('Sent copies'),
      );
      await show(sentSettings);
      await tester.pumpAndSettle();
      await tester.tap(sentSettings);
      await tester.pumpAndSettle();
      await tester.tap(find.byType(DropdownButton<String>));
      await tester.pumpAndSettle();
      await tester.tap(find.text('My server saves Sent automatically').last);
      await tester.pumpAndSettle();
      await tester.enterText(
        find.widgetWithText(TextField, 'Sent folder (optional)'),
        'Provider Sent Mail',
      );
      await capture('native-sent-preferences');
      await tester.tap(find.text('Save Sent preferences'));
      await tester.pumpAndSettle();
      await wait(
        () => repository.mailAccounts.any(
          (a) =>
              a.id == 'imap-fixture' &&
              a.sentCopy == 'ServerManaged' &&
              a.sentFolder == 'Provider Sent Mail',
        ),
      );

      // Reopen through the public profile API. Controls below inspect persisted
      // recovered drafts; no running state or storage is modified by the test.
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
      repository = await NativeRepository.open(path, credentials: credentials);
      workspace = Workspace(repository, MemorySettings());
      await workspace.initialize();
      workspace.setForeground(false);
      expect(
        repository.mailAccounts
            .firstWhere((a) => a.id == 'imap-fixture')
            .sentCopy,
        'ServerManaged',
      );
      expect(
        repository.mailAccounts
            .firstWhere((a) => a.id == 'imap-fixture')
            .sentFolder,
        'Provider Sent Mail',
      );
      await tester.pumpWidget(ShepApp(key: UniqueKey(), workspace: workspace));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Open navigation menu'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Archive'));
      await tester.pumpAndSettle();
      await wait(
        () => find.text('Native Outbox copy-saved').evaluate().isNotEmpty,
      );
      expect(workspace.mail(localId)!.starred, isTrue);
      expect(workspace.mail(localId)!.unread, isTrue);
      await tester.tap(find.text('Native Outbox copy-saved'));
      await tester.pumpAndSettle();
      await wait(() => workspace.loadingBodies.isEmpty);
      await wait(() => workspace.pending == 0);
      expect(find.byTooltip('Unflag'), findsOneWidget);
      // The saved unread state was checked before opening. The current mobile
      // default marks on open; parity with desktop's read-on-leave stays open.
      expect(find.byTooltip('Mark unread'), findsOneWidget);
      expect(workspace.mail(localId)!.unread, isFalse);
      expect(credentials.reads, credentialReads);
      await capture('native-imap-local-sent-reopened');
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Open navigation menu'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Inbox'));
      await tester.pumpAndSettle();
      await wait(
        () => find.text('Native IMAP credential fixture').evaluate().isNotEmpty,
      );
      await tester.tap(find.text('Native IMAP credential fixture'));
      await tester.pumpAndSettle();
      await wait(() => workspace.loadingBodies.isEmpty);
      const remoteId = 'imap-fixture:Sent:local-sent-provider';
      credentials.unavailable = true;
      await tester.tap(find.byTooltip('Flag'));
      await tester.pumpAndSettle();
      await wait(() => workspace.pending == 0);
      expect(workspace.error, contains('credential store is unavailable'));
      expect(
        find.textContaining('credential store is unavailable'),
        findsOneWidget,
      );
      expect(find.text('Retry'), findsOneWidget);
      expect(workspace.mail(remoteId)!.starred, isFalse);
      expect(find.byTooltip('Flag'), findsOneWidget);
      credentials.unavailable = false;
      await tester.tap(find.byTooltip('Flag'));
      await tester.pumpAndSettle();
      await wait(() => workspace.pending == 0);
      expect(workspace.error, contains('saved password is missing'));
      expect(workspace.mail(remoteId)!.starred, isFalse);
      expect(credentials.reads, credentialReads + 2);
      expect(find.textContaining('saved password is missing'), findsOneWidget);
      await capture('native-imap-credential-recovery');
      await tester.tap(find.byTooltip('Dismiss error'));
      await tester.pumpAndSettle();
      expect(find.text('Retry'), findsNothing);
      expect(workspace.error, isNull);
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Open navigation menu'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Drafts'));
      await tester.pumpAndSettle();
      expect(find.text('Native Outbox rejected'), findsOneWidget);
      await tester.tap(find.text('Native Outbox return'));
      await tester.pumpAndSettle();
      await wait(
        () => !tester
            .widget<TextField>(find.widgetWithText(TextField, 'To'))
            .readOnly,
      );
      expect(find.byTooltip('Remove review.bin'), findsNothing);
      expect(find.byTooltip('Send'), findsOneWidget);
      await tester.tap(find.byTooltip('Send'));
      await tester.pumpAndSettle();
      await wait(
        () => find
            .textContaining('saved password is missing')
            .evaluate()
            .isNotEmpty,
      );
      expect(
        tester
            .widget<TextField>(find.widgetWithText(TextField, 'Message'))
            .controller!
            .text,
        'Original native return body.',
      );
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
    },
  );
}
