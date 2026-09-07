import 'dart:convert';
import 'dart:io';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:path_provider/path_provider.dart';
import 'package:shep_mobile/data/native_repository.dart';
import 'package:shep_mobile/model/workspace.dart';
import 'package:shep_mobile/ui/app.dart';
import 'package:shep_mobile/ui/formatted_view.dart';
import '../test/native_repository_test.dart' show FixtureCredentials;
import '../test/workspace_test.dart' show MemorySettings;

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();
  testWidgets(
    'native Rust formatted mail, Find, quotes, plain text and recoverable damaged HTML',
    (tester) async {
      expect(Platform.isAndroid, isTrue);
      final support = await getApplicationSupportDirectory();
      final request = File('${support.path}/shep-html-request'),
          fixture = File('${support.path}/shep-html-fixture.sqlite');
      await request.writeAsString('waiting-before-profile-open', flush: true);
      final deadline = DateTime.now().add(const Duration(seconds: 30));
      while (!await fixture.exists()) {
        if (DateTime.now().isAfter(deadline)) {
          fail(
            'Use android_e2e.py --formatted-only to seed the isolated cache',
          );
        }
        await Future<void>.delayed(const Duration(milliseconds: 100));
      }
      final dir = await Directory.systemTemp.createTemp('shep-formatted-');
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
      Future<void> wait(
        bool Function() ready, [
        String reason = 'Formatted controls did not settle',
      ]) async {
        final end = DateTime.now().add(const Duration(seconds: 30));
        while (!ready()) {
          if (DateTime.now().isAfter(end)) fail(reason);
          await tester.pump(const Duration(milliseconds: 100));
        }
        await tester.pumpAndSettle();
        expect(tester.takeException(), isNull);
      }

      Future<void> snapshot(String name) async {
        await request.writeAsString('capture:$name', flush: true);
        final end = DateTime.now().add(const Duration(seconds: 20));
        while ((await request.readAsString()).trim() != 'captured:$name') {
          if (DateTime.now().isAfter(end)) {
            fail('Native screenshot was not captured: $name');
          }
          await tester.pump(const Duration(milliseconds: 100));
        }
      }

      final view = find.byType(FormattedView);
      Future<dynamic> observe(String expression) async {
        // Read-only oracle. All actions below use Flutter/native pointer input.
        final dynamic state = tester.state(view);
        var value = await state.controller.runJavaScriptReturningResult(
          'JSON.stringify($expression)',
        );
        if (value is String) value = jsonDecode(value);
        if (value is String) {
          try {
            value = jsonDecode(value);
          } on FormatException {
            /* WKWebView already returned the string value. */
          }
        }
        return value;
      }

      Future<void> content() async {
        await wait(() => view.evaluate().isNotEmpty);
        final end = DateTime.now().add(const Duration(seconds: 20));
        while (await observe(
              'document.querySelector("h1")?.textContent ?? null',
            ) !=
            'Verification needed') {
          if (DateTime.now().isAfter(end)) {
            fail('Native WebView did not show prepared HTML');
          }
          await tester.pump(const Duration(milliseconds: 100));
        }
        await tester.pumpAndSettle();
      }

      await tester.tap(find.text('Shep formatted reader fixture'));
      await content();
      await tester.ensureVisible(view);
      await tester.pumpAndSettle();
      expect(
        await observe(
          'getComputedStyle(document.querySelector("table.card")).backgroundColor',
        ),
        'rgb(27, 27, 32)',
      );
      expect(await observe('document.images[0].naturalWidth'), greaterThan(0));
      expect(
        await observe('document.querySelectorAll("blockquote").length'),
        0,
      );
      expect(credentials.reads, 0);
      await tester.pumpAndSettle();
      await snapshot('native-formatted-light');
      await tester.tap(find.byTooltip('Find in message'));
      await tester.pumpAndSettle();
      final input = find.widgetWithText(TextField, 'Find in message');
      Future<void> query(String text, String expected) async {
        await tester.enterText(input, text);
        await wait(() => find.text(expected).evaluate().isNotEmpty);
      }

      await query('Alpha across spans', '1 of 1');
      expect(
        await observe(
          'CSS.highlights?.get("shep-active")?.size ?? document.querySelectorAll("shep-match[data-active]").length',
        ),
        3,
      );
      await query('Alpha', '1 of 2');
      await tester.ensureVisible(find.text('Show quoted history'));
      await tester.tap(find.text('Show quoted history'));
      await wait(() => find.text('1 of 3').evaluate().isNotEmpty);
      await tester.tap(find.byTooltip('Next match'));
      await wait(() => find.text('2 of 3').evaluate().isNotEmpty);
      await tester.tap(find.byTooltip('Next match'));
      await wait(() => find.text('3 of 3').evaluate().isNotEmpty);
      expect(await observe('scrollY'), greaterThan(1000));
      await snapshot('native-formatted-find-tail');
      await tester.tap(find.byTooltip('Flag'));
      await tester.pumpAndSettle();
      await wait(() => workspace.pending == 0);
      expect(await observe('scrollY'), greaterThan(1000));
      await tester.ensureVisible(find.text('Plain text'));
      await tester.tap(find.text('Plain text'));
      await wait(() => find.text('1 of 2').evaluate().isNotEmpty);
      expect(find.textContaining('Plain alternative'), findsOneWidget);
      await tester.ensureVisible(find.text('Formatted'));
      await tester.tap(find.text('Formatted'));
      await wait(() => find.text('1 of 3').evaluate().isNotEmpty);
      await tester.tap(find.byTooltip('Close Find'));
      await tester.pumpAndSettle();
      await tester.pageBack();
      await tester.pumpAndSettle();
      await tester.tap(find.text('Damaged HTML fixture'));
      await tester.pumpAndSettle();
      await wait(
        () => find.text('Retry formatted message').evaluate().isNotEmpty,
      );
      expect(find.textContaining('Readable damaged message.'), findsOneWidget);
      await tester.ensureVisible(find.text('Retry formatted message'));
      await tester.tap(find.text('Retry formatted message'));
      await tester.pumpAndSettle();
      await wait(
        () => find.text('Retry formatted message').evaluate().isNotEmpty,
      );
      await snapshot('native-formatted-retry');
      await snapshot('complete');
    },
  );
}
